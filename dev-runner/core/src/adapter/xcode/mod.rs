//! Xcode Adapter
//!
//! Handles .xcodeproj projects.

mod devices;

use std::path::{Path, PathBuf};

use crate::adapter::{
    BuildOptions, Command, Device, RunOptions, RunTarget, RunnerAdapter,
};

pub use devices::Platform;

/// Xcode project adapter
pub struct XcodeAdapter {
    /// Path to .xcodeproj
    project_path: PathBuf,
    /// Project name (derived from .xcodeproj name)
    name: String,
    /// Parsed schemes (lazy loaded)
    schemes: Vec<String>,
    /// Bundle identifier (lazy loaded)
    bundle_id: Option<String>,
    /// Target platform (detected from project)
    platform: Platform,
}

impl XcodeAdapter {
    /// Detect if path contains an Xcode project
    pub fn detect(path: &Path) -> Option<Box<dyn RunnerAdapter>> {
        // Look for .xcodeproj in the directory
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                if entry_path.extension().map_or(false, |ext| ext == "xcodeproj") {
                    return Some(Box::new(Self::new(entry_path)));
                }
            }
        }
        None
    }

    /// Detect all Xcode projects in a directory
    pub fn detect_all(path: &Path) -> Vec<Box<dyn RunnerAdapter>> {
        let mut adapters: Vec<Box<dyn RunnerAdapter>> = Vec::new();

        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                if entry_path.extension().map_or(false, |ext| ext == "xcodeproj") {
                    adapters.push(Box::new(Self::new(entry_path)));
                }
            }
        }

        adapters
    }

    /// Create a new XcodeAdapter for a .xcodeproj path
    fn new(project_path: PathBuf) -> Self {
        let name = project_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        let platform = Self::detect_platform(&project_path);

        Self {
            project_path,
            name,
            schemes: Vec::new(),
            bundle_id: None,
            platform,
        }
    }

    /// Detect platform from project.pbxproj (SDKROOT / SUPPORTED_PLATFORMS).
    ///
    /// Collects all platforms found and picks by priority (non-macOS preferred),
    /// since macOS is the default fallback.
    fn detect_platform(project_path: &Path) -> Platform {
        use std::collections::HashSet;

        let pbxproj_path = project_path.join("project.pbxproj");
        let content = match std::fs::read_to_string(&pbxproj_path) {
            Ok(c) => c,
            Err(_) => return Platform::MacOS,
        };

        let mut found: HashSet<Platform> = HashSet::new();

        for line in content.lines() {
            let line = line.trim();

            if line.starts_with("SDKROOT") || line.starts_with("SUPPORTED_PLATFORMS") {
                if line.contains("iphoneos") || line.contains("iphonesimulator") {
                    found.insert(Platform::IOS);
                }
                if line.contains("appletvos") || line.contains("appletvsimulator") {
                    found.insert(Platform::TvOS);
                }
                if line.contains("watchos") || line.contains("watchsimulator") {
                    found.insert(Platform::WatchOS);
                }
                if line.contains("xros") || line.contains("xrsimulator") {
                    found.insert(Platform::VisionOS);
                }
                if line.contains("macosx") {
                    found.insert(Platform::MacOS);
                }
            }
        }

        // Priority: non-macOS platforms first (macOS is the default fallback)
        let priority = [
            Platform::IOS,
            Platform::TvOS,
            Platform::WatchOS,
            Platform::VisionOS,
            Platform::MacOS,
        ];
        for p in priority {
            if found.contains(&p) {
                return p;
            }
        }

        Platform::MacOS
    }

    /// Parse schemes: 优先读 .xcscheme 文件（快），fallback 到 xcodebuild -list（慢）
    fn parse_schemes(&self) -> Vec<String> {
        // 优先直接读 scheme 文件（毫秒级）
        let schemes = self.parse_schemes_from_files();
        if !schemes.is_empty() {
            return schemes;
        }

        // Fallback: xcodebuild -list（秒级，但更可靠）
        self.parse_schemes_via_xcodebuild().unwrap_or_default()
    }

    /// Use xcodebuild -list to get schemes, filtered by native targets
    fn parse_schemes_via_xcodebuild(&self) -> Option<Vec<String>> {
        use std::collections::HashSet;
        use std::process::Command;

        let output = Command::new("xcodebuild")
            .arg("-list")
            .arg("-project")
            .arg(&self.project_path)
            .arg("-json")
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).ok()?;

        let project = json.get("project")?;

        // Collect native target names to filter out SPM dependency schemes
        let targets: HashSet<String> = project
            .get("targets")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let all_schemes: Vec<String> = project
            .get("schemes")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        // Only keep schemes whose name matches a native target
        let filtered: Vec<String> = all_schemes
            .iter()
            .filter(|s| targets.contains(s.as_str()))
            .cloned()
            .collect();

        // Fallback: if filtering removes everything, return all schemes
        if filtered.is_empty() && !all_schemes.is_empty() {
            return Some(all_schemes);
        }

        Some(filtered)
    }

    /// Parse schemes from xcscheme files, filtering out SPM dependency schemes.
    ///
    /// Looks for .xcscheme files in:
    /// - {project_path}/xcshareddata/xcschemes/*.xcscheme (shared schemes)
    /// - {project_path}/xcuserdata/*/xcschemes/*.xcscheme (user schemes)
    ///
    /// Only includes schemes whose ReferencedContainer points to the current project.
    fn parse_schemes_from_files(&self) -> Vec<String> {
        let mut schemes = Vec::new();

        // Build the container pattern to match: container:ProjectName.xcodeproj
        let project_file_name = self
            .project_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let container_pattern = format!("container:{}", project_file_name);

        let collect_schemes = |dir: &Path, schemes: &mut Vec<String>| {
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => return,
            };
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.extension().map_or(false, |ext| ext == "xcscheme") {
                    continue;
                }
                let name = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if schemes.contains(&name) {
                    continue; // Avoid duplicates
                }

                // Filter: check ReferencedContainer points to this project
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if content.contains(&container_pattern) {
                        schemes.push(name);
                    }
                }
            }
        };

        // Parse shared schemes
        let shared_schemes_dir = self.project_path.join("xcshareddata/xcschemes");
        collect_schemes(&shared_schemes_dir, &mut schemes);

        // Parse user schemes
        let userdata_dir = self.project_path.join("xcuserdata");
        if let Ok(users) = std::fs::read_dir(&userdata_dir) {
            for user_entry in users.filter_map(|e| e.ok()) {
                let user_schemes_dir = user_entry.path().join("xcschemes");
                collect_schemes(&user_schemes_dir, &mut schemes);
            }
        }

        schemes
    }

    /// Check if the project supports Mac Catalyst (SUPPORTS_MACCATALYST = YES)
    fn supports_catalyst(&self) -> bool {
        let pbxproj_path = self.project_path.join("project.pbxproj");
        let content = match std::fs::read_to_string(&pbxproj_path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        content.lines().any(|line| {
            let line = line.trim();
            line.starts_with("SUPPORTS_MACCATALYST") && line.contains("YES")
        })
    }

    /// Parse bundle ID from project.pbxproj
    ///
    /// Extracts the first PRODUCT_BUNDLE_IDENTIFIER found in the project file.
    fn parse_bundle_id(&self) -> Option<String> {
        let pbxproj_path = self.project_path.join("project.pbxproj");
        let content = std::fs::read_to_string(&pbxproj_path).ok()?;

        // Match PRODUCT_BUNDLE_IDENTIFIER = "xxx"; or PRODUCT_BUNDLE_IDENTIFIER = xxx;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("PRODUCT_BUNDLE_IDENTIFIER") {
                // Extract value after '='
                if let Some(eq_pos) = line.find('=') {
                    let value = line[eq_pos + 1..].trim();
                    // Remove trailing semicolon and quotes
                    let value = value.trim_end_matches(';').trim();
                    let value = value.trim_matches('"');
                    // Skip test bundle IDs
                    if !value.contains("Tests") {
                        return Some(value.to_string());
                    }
                }
            }
        }

        None
    }

    /// List available simulators for a platform
    pub fn list_simulators(platform: Platform) -> Vec<Device> {
        devices::fetch_simulators(platform)
    }

    /// List connected physical devices for a platform
    pub fn list_physical_devices(platform: Platform) -> Vec<Device> {
        devices::fetch_physical_devices(platform)
    }

    /// Generate xcodebuild -destination string for a device
    fn destination_string(&self, device: &Device) -> String {
        use crate::adapter::DeviceType;

        match device.device_type {
            DeviceType::Mac => "platform=macOS".to_string(),
            DeviceType::Simulator => format!("platform=iOS Simulator,id={}", device.id),
            DeviceType::Physical => format!("platform=iOS,id={}", device.id),
        }
    }

    /// Get derived data path for this project
    ///
    /// Uses ~/.vimo/dev-runner/DerivedData/{project_name}
    fn derived_data_path(&self) -> Option<PathBuf> {
        dirs::home_dir().map(|home| {
            home.join(".vimo")
                .join("dev-runner")
                .join("DerivedData")
                .join(&self.name)
        })
    }

    /// Get the .app bundle path after build
    ///
    /// Returns the path in DerivedData where the built .app should be located.
    fn app_bundle_path(&self, config: &str, device: &Device) -> Option<PathBuf> {
        use crate::adapter::DeviceType;

        let derived_data = self.derived_data_path()?;
        let platform_dir = match device.device_type {
            DeviceType::Mac => "Debug", // macOS doesn't have platform suffix
            DeviceType::Simulator => &format!("{}-iphonesimulator", config),
            DeviceType::Physical => &format!("{}-iphoneos", config),
        };

        Some(
            derived_data
                .join("Build")
                .join("Products")
                .join(platform_dir)
                .join(format!("{}.app", self.name)),
        )
    }
}

impl RunnerAdapter for XcodeAdapter {
    fn adapter_type(&self) -> &'static str {
        "xcode"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn path(&self) -> &Path {
        self.project_path
            .parent()
            .unwrap_or(Path::new("."))
    }

    fn targets(&self) -> Vec<RunTarget> {
        self.parse_schemes()
            .into_iter()
            .map(|name| RunTarget {
                name,
                target_type: "scheme".to_string(),
                description: None,
            })
            .collect()
    }

    fn build_cmd(&self, target: &str, options: &BuildOptions) -> Option<Command> {
        let config = if options.config.is_empty() {
            "Debug"
        } else {
            &options.config
        };

        let mut cmd = Command::new("xcodebuild")
            .arg("-project")
            .arg(self.project_path.to_string_lossy().to_string())
            .arg("-scheme")
            .arg(target)
            .arg("-configuration")
            .arg(config);

        // Add destination if device is specified
        if let Some(device) = &options.device {
            cmd = cmd.arg("-destination").arg(self.destination_string(device));
        }

        // Set derived data path for consistent build output location
        if let Some(derived_data) = self.derived_data_path() {
            cmd = cmd
                .arg("-derivedDataPath")
                .arg(derived_data.to_string_lossy().to_string());
        }

        if options.clean {
            cmd = cmd.arg("clean");
        }

        cmd = cmd.arg("build");

        Some(cmd)
    }

    fn run_cmd(&self, _target: &str, options: &RunOptions) -> Command {
        use crate::adapter::DeviceType;

        let bundle_id = self.parse_bundle_id().unwrap_or_else(|| self.name.clone());

        match options.device.as_ref().map(|d| &d.device_type) {
            Some(DeviceType::Simulator) => {
                let device = options.device.as_ref().unwrap();
                // xcrun simctl launch --console-pty <device-id> <bundle-id> [args...]
                // --console-pty uses a PTY to capture app's stdout/stderr (print() etc.)
                // Note: --console (pipe mode) does NOT capture iOS app print() output
                let mut cmd = Command::new("xcrun")
                    .arg("simctl")
                    .arg("launch")
                    .arg("--console-pty")
                    .arg(&device.id)
                    .arg(&bundle_id);

                for arg in &options.args {
                    cmd = cmd.arg(arg);
                }

                cmd
            }
            Some(DeviceType::Physical) => {
                let device = options.device.as_ref().unwrap();
                // xcrun devicectl device process launch --device <id> <bundle-id>
                let mut cmd = Command::new("xcrun")
                    .arg("devicectl")
                    .arg("device")
                    .arg("process")
                    .arg("launch")
                    .arg("--device")
                    .arg(&device.id)
                    .arg(&bundle_id);

                for arg in &options.args {
                    cmd = cmd.arg(arg);
                }

                cmd
            }
            Some(DeviceType::Mac) | None => {
                // Run the executable directly (not via `open`) to capture stdout/stderr
                let app_path = self
                    .app_bundle_path("Debug", &Device {
                        id: "mac".to_string(),
                        name: "My Mac".to_string(),
                        device_type: DeviceType::Mac,
                        os_version: None,
                        state: crate::adapter::DeviceState::Available,
                    })
                    .unwrap_or_else(|| PathBuf::from(format!("{}.app", self.name)));

                // The executable is inside Contents/MacOS/<app-name>
                let executable = app_path.join("Contents/MacOS").join(&self.name);

                let mut cmd = Command::new(executable.to_string_lossy().to_string());

                for arg in &options.args {
                    cmd = cmd.arg(arg);
                }

                cmd
            }
        }
    }

    fn log_cmd(&self, _target: &str, options: &RunOptions) -> Option<Command> {
        use crate::adapter::DeviceType;

        let bundle_id = self.bundle_id()?;
        let predicate = format!("subsystem == \"{}\"", bundle_id);

        match options.device.as_ref().map(|d| &d.device_type) {
            Some(DeviceType::Simulator) => {
                let device = options.device.as_ref().unwrap();
                // xcrun simctl spawn <device-id> log stream --predicate '...'
                Some(
                    Command::new("xcrun")
                        .arg("simctl")
                        .arg("spawn")
                        .arg(&device.id)
                        .arg("log")
                        .arg("stream")
                        .arg("--predicate")
                        .arg(&predicate),
                )
            }
            Some(DeviceType::Physical) => {
                let device = options.device.as_ref().unwrap();
                // xcrun devicectl device syslog --device <id>
                // Note: devicectl syslog doesn't support predicate filtering,
                // filtering needs to be done in the app layer
                Some(
                    Command::new("xcrun")
                        .arg("devicectl")
                        .arg("device")
                        .arg("syslog")
                        .arg("--device")
                        .arg(&device.id),
                )
            }
            Some(DeviceType::Mac) | None => {
                // macOS: log stream --predicate '...'
                Some(
                    Command::new("log")
                        .arg("stream")
                        .arg("--predicate")
                        .arg(&predicate),
                )
            }
        }
    }

    fn install_cmd(&self, _target: &str, options: &RunOptions) -> Option<Command> {
        use crate::adapter::DeviceType;

        let device = options.device.as_ref()?;
        let config = "Debug"; // TODO: Get from options

        match device.device_type {
            DeviceType::Simulator => {
                let app_path = self.app_bundle_path(config, device)?;
                // xcrun simctl install <device-id> <path-to.app>
                Some(
                    Command::new("xcrun")
                        .arg("simctl")
                        .arg("install")
                        .arg(&device.id)
                        .arg(app_path.to_string_lossy().to_string()),
                )
            }
            DeviceType::Physical => {
                let app_path = self.app_bundle_path(config, device)?;
                // xcrun devicectl device install app --device <id> <path-to.app>
                Some(
                    Command::new("xcrun")
                        .arg("devicectl")
                        .arg("device")
                        .arg("install")
                        .arg("app")
                        .arg("--device")
                        .arg(&device.id)
                        .arg(app_path.to_string_lossy().to_string()),
                )
            }
            DeviceType::Mac => {
                // macOS apps don't need installation
                None
            }
        }
    }

    fn devices(&self) -> Vec<Device> {
        let mut devices = Vec::new();

        // For macOS projects, only show Mac
        if self.platform == Platform::MacOS {
            devices.push(Device {
                id: "mac".to_string(),
                name: "My Mac".to_string(),
                device_type: crate::adapter::DeviceType::Mac,
                os_version: None,
                state: crate::adapter::DeviceState::Available,
            });
            return devices;
        }

        // For iOS projects, only add Mac if the project supports Mac Catalyst
        if self.platform == Platform::IOS && self.supports_catalyst() {
            devices.push(Device {
                id: "mac".to_string(),
                name: "My Mac".to_string(),
                device_type: crate::adapter::DeviceType::Mac,
                os_version: None,
                state: crate::adapter::DeviceState::Available,
            });
        }

        // Add simulators for the target platform
        devices.extend(Self::list_simulators(self.platform));

        // Add physical devices for the target platform
        devices.extend(Self::list_physical_devices(self.platform));

        devices
    }

    fn bundle_id(&self) -> Option<String> {
        self.parse_bundle_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_detect_no_project() {
        let dir = tempdir().unwrap();
        let result = XcodeAdapter::detect(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_detect_with_xcodeproj() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        let result = XcodeAdapter::detect(dir.path());
        assert!(result.is_some());

        let adapter = result.unwrap();
        assert_eq!(adapter.adapter_type(), "xcode");
        assert_eq!(adapter.name(), "MyApp");
    }

    #[test]
    fn test_build_cmd() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let options = BuildOptions {
            config: "Debug".to_string(),
            clean: false,
            ..Default::default()
        };

        let cmd = adapter.build_cmd("MyApp", &options).unwrap();
        assert_eq!(cmd.program, "xcodebuild");
        assert!(cmd.args.contains(&"-scheme".to_string()));
        assert!(cmd.args.contains(&"MyApp".to_string()));
    }

    /// Helper: create a minimal .xcscheme file referencing the given project container
    fn write_scheme(path: &Path, project_name: &str) {
        let content = format!(
            r#"<Scheme><BuildAction><BuildActionEntries><BuildActionEntry>
            <BuildableReference BuildableIdentifier="primary"
                BuildableName="{name}.app" BlueprintName="{name}"
                ReferencedContainer="container:{name}.xcodeproj">
            </BuildableReference>
            </BuildActionEntry></BuildActionEntries></BuildAction></Scheme>"#,
            name = project_name
        );
        std::fs::write(path, content).unwrap();
    }

    /// Helper: create an SPM dependency .xcscheme file (different container)
    fn write_spm_scheme(path: &Path, package_name: &str) {
        let content = format!(
            r#"<Scheme><BuildAction><BuildActionEntries><BuildActionEntry>
            <BuildableReference BuildableIdentifier="primary"
                BuildableName="{name}.framework" BlueprintName="{name}"
                ReferencedContainer="container:SourcePackages/checkouts/{name}">
            </BuildableReference>
            </BuildActionEntry></BuildActionEntries></BuildAction></Scheme>"#,
            name = package_name
        );
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn test_parse_schemes_shared() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        let schemes_dir = xcodeproj.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes_dir).unwrap();

        // Create scheme files referencing this project
        write_scheme(&schemes_dir.join("MyApp.xcscheme"), "MyApp");
        write_scheme(&schemes_dir.join("MyAppTests.xcscheme"), "MyApp");

        let adapter = XcodeAdapter::new(xcodeproj);
        let schemes = adapter.parse_schemes();

        assert_eq!(schemes.len(), 2);
        assert!(schemes.contains(&"MyApp".to_string()));
        assert!(schemes.contains(&"MyAppTests".to_string()));
    }

    #[test]
    fn test_parse_schemes_user() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        let user_schemes_dir = xcodeproj.join("xcuserdata/testuser.xcuserdatad/xcschemes");
        std::fs::create_dir_all(&user_schemes_dir).unwrap();

        // Create user scheme file referencing this project
        write_scheme(&user_schemes_dir.join("UserScheme.xcscheme"), "MyApp");

        let adapter = XcodeAdapter::new(xcodeproj);
        let schemes = adapter.parse_schemes();

        assert_eq!(schemes.len(), 1);
        assert!(schemes.contains(&"UserScheme".to_string()));
    }

    #[test]
    fn test_parse_schemes_no_duplicates() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");

        // Create shared scheme
        let shared_dir = xcodeproj.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&shared_dir).unwrap();
        write_scheme(&shared_dir.join("MyApp.xcscheme"), "MyApp");

        // Create same scheme in user data
        let user_dir = xcodeproj.join("xcuserdata/test.xcuserdatad/xcschemes");
        std::fs::create_dir_all(&user_dir).unwrap();
        write_scheme(&user_dir.join("MyApp.xcscheme"), "MyApp");

        let adapter = XcodeAdapter::new(xcodeproj);
        let schemes = adapter.parse_schemes();

        // Should not have duplicates
        assert_eq!(schemes.len(), 1);
        assert!(schemes.contains(&"MyApp".to_string()));
    }

    #[test]
    fn test_parse_schemes_filters_spm() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        let schemes_dir = xcodeproj.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes_dir).unwrap();

        // Project scheme (should be kept)
        write_scheme(&schemes_dir.join("MyApp.xcscheme"), "MyApp");
        // SPM dependency scheme (should be filtered out)
        write_spm_scheme(&schemes_dir.join("HighlightSwift.xcscheme"), "HighlightSwift");

        let adapter = XcodeAdapter::new(xcodeproj);
        let schemes = adapter.parse_schemes_from_files();

        assert_eq!(schemes.len(), 1);
        assert!(schemes.contains(&"MyApp".to_string()));
        assert!(!schemes.contains(&"HighlightSwift".to_string()));
    }

    #[test]
    fn test_parse_bundle_id() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        // Create a minimal project.pbxproj
        let pbxproj_content = r#"
            /* Begin PBXNativeTarget section */
            buildSettings = {
                PRODUCT_BUNDLE_IDENTIFIER = com.example.MyApp;
                PRODUCT_NAME = "$(TARGET_NAME)";
            };
            /* End PBXNativeTarget section */
        "#;
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let bundle_id = adapter.parse_bundle_id();

        assert_eq!(bundle_id, Some("com.example.MyApp".to_string()));
    }

    #[test]
    fn test_parse_bundle_id_with_quotes() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        let pbxproj_content = r#"
            PRODUCT_BUNDLE_IDENTIFIER = "com.example.MyApp";
        "#;
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let bundle_id = adapter.parse_bundle_id();

        assert_eq!(bundle_id, Some("com.example.MyApp".to_string()));
    }

    #[test]
    fn test_parse_bundle_id_skips_tests() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        // Test bundle ID comes first, but should be skipped
        let pbxproj_content = r#"
            PRODUCT_BUNDLE_IDENTIFIER = com.example.MyAppTests;
            PRODUCT_BUNDLE_IDENTIFIER = com.example.MyApp;
        "#;
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let bundle_id = adapter.parse_bundle_id();

        assert_eq!(bundle_id, Some("com.example.MyApp".to_string()));
    }

    #[test]
    fn test_targets_returns_schemes() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        let schemes_dir = xcodeproj.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes_dir).unwrap();

        write_scheme(&schemes_dir.join("MyApp.xcscheme"), "MyApp");

        let adapter = XcodeAdapter::new(xcodeproj);
        let targets = adapter.targets();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "MyApp");
        assert_eq!(targets[0].target_type, "scheme");
    }

    #[test]
    fn test_build_cmd_with_device() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let device = Device {
            id: "ABC-123".to_string(),
            name: "iPhone 15 Pro".to_string(),
            device_type: crate::adapter::DeviceType::Simulator,
            os_version: Some("17.0".to_string()),
            state: crate::adapter::DeviceState::Available,
        };
        let options = BuildOptions {
            config: "Debug".to_string(),
            clean: false,
            device: Some(device),
            ..Default::default()
        };

        let cmd = adapter.build_cmd("MyApp", &options).unwrap();
        assert!(cmd.args.contains(&"-destination".to_string()));
        assert!(cmd.args.iter().any(|a| a.contains("iOS Simulator")));
    }

    #[test]
    fn test_run_cmd_simulator() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();
        std::fs::write(
            xcodeproj.join("project.pbxproj"),
            "PRODUCT_BUNDLE_IDENTIFIER = com.example.MyApp;",
        )
        .unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let device = Device {
            id: "ABC-123".to_string(),
            name: "iPhone 15 Pro".to_string(),
            device_type: crate::adapter::DeviceType::Simulator,
            os_version: Some("17.0".to_string()),
            state: crate::adapter::DeviceState::Available,
        };
        let options = RunOptions {
            device: Some(device),
            ..Default::default()
        };

        let cmd = adapter.run_cmd("MyApp", &options);
        assert_eq!(cmd.program, "xcrun");
        assert!(cmd.args.contains(&"simctl".to_string()));
        assert!(cmd.args.contains(&"launch".to_string()));
        assert!(cmd.args.contains(&"ABC-123".to_string()));
        assert!(cmd.args.contains(&"com.example.MyApp".to_string()));
    }

    #[test]
    fn test_run_cmd_physical() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();
        std::fs::write(
            xcodeproj.join("project.pbxproj"),
            "PRODUCT_BUNDLE_IDENTIFIER = com.example.MyApp;",
        )
        .unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let device = Device {
            id: "DEF-456".to_string(),
            name: "My iPhone".to_string(),
            device_type: crate::adapter::DeviceType::Physical,
            os_version: Some("17.2".to_string()),
            state: crate::adapter::DeviceState::Available,
        };
        let options = RunOptions {
            device: Some(device),
            ..Default::default()
        };

        let cmd = adapter.run_cmd("MyApp", &options);
        assert_eq!(cmd.program, "xcrun");
        assert!(cmd.args.contains(&"devicectl".to_string()));
        assert!(cmd.args.contains(&"launch".to_string()));
    }

    #[test]
    fn test_install_cmd_simulator() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let device = Device {
            id: "ABC-123".to_string(),
            name: "iPhone 15 Pro".to_string(),
            device_type: crate::adapter::DeviceType::Simulator,
            os_version: Some("17.0".to_string()),
            state: crate::adapter::DeviceState::Available,
        };
        let options = RunOptions {
            device: Some(device),
            ..Default::default()
        };

        let cmd = adapter.install_cmd("MyApp", &options).unwrap();
        assert_eq!(cmd.program, "xcrun");
        assert!(cmd.args.contains(&"simctl".to_string()));
        assert!(cmd.args.contains(&"install".to_string()));
    }

    #[test]
    fn test_log_cmd_simulator() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();
        std::fs::write(
            xcodeproj.join("project.pbxproj"),
            "PRODUCT_BUNDLE_IDENTIFIER = com.example.MyApp;",
        )
        .unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let device = Device {
            id: "ABC-123".to_string(),
            name: "iPhone 15 Pro".to_string(),
            device_type: crate::adapter::DeviceType::Simulator,
            os_version: Some("17.0".to_string()),
            state: crate::adapter::DeviceState::Available,
        };
        let options = RunOptions {
            device: Some(device),
            ..Default::default()
        };

        let cmd = adapter.log_cmd("MyApp", &options).unwrap();
        assert_eq!(cmd.program, "xcrun");
        assert!(cmd.args.contains(&"simctl".to_string()));
        assert!(cmd.args.contains(&"spawn".to_string()));
        assert!(cmd.args.contains(&"log".to_string()));
        assert!(cmd.args.contains(&"stream".to_string()));
    }

    #[test]
    fn test_detect_platform_multi_sdk() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        // pbxproj with both macOS and iOS SDKROOTs — iOS should win
        let pbxproj_content = r#"
            buildSettings = {
                SDKROOT = macosx;
            };
            buildSettings = {
                SDKROOT = iphoneos;
            };
        "#;
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let platform = XcodeAdapter::detect_platform(&xcodeproj);
        assert_eq!(platform, Platform::IOS);
    }

    #[test]
    fn test_detect_platform_macos_only() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        let pbxproj_content = r#"
            buildSettings = {
                SDKROOT = macosx;
            };
        "#;
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let platform = XcodeAdapter::detect_platform(&xcodeproj);
        assert_eq!(platform, Platform::MacOS);
    }

    #[test]
    fn test_detect_platform_supported_platforms_fallback() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        let pbxproj_content = r#"
            buildSettings = {
                SUPPORTED_PLATFORMS = "iphoneos iphonesimulator macosx";
            };
        "#;
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let platform = XcodeAdapter::detect_platform(&xcodeproj);
        assert_eq!(platform, Platform::IOS);
    }

    #[test]
    fn test_devices_ios_no_catalyst() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        // iOS project without Catalyst
        let pbxproj_content = "SDKROOT = iphoneos;";
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        assert_eq!(adapter.platform, Platform::IOS);

        let devices = adapter.devices();
        // Should NOT contain "My Mac" (no Catalyst)
        assert!(!devices.iter().any(|d| d.name == "My Mac"));
    }

    #[test]
    fn test_devices_ios_catalyst() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        // iOS project with Catalyst support
        let pbxproj_content = r#"
            SDKROOT = iphoneos;
            SUPPORTS_MACCATALYST = YES;
        "#;
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        assert_eq!(adapter.platform, Platform::IOS);

        let devices = adapter.devices();
        // Should contain "My Mac" (Catalyst enabled)
        assert!(devices.iter().any(|d| d.name == "My Mac"));
    }

    #[test]
    fn test_devices_macos_only_mac() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        std::fs::create_dir(&xcodeproj).unwrap();

        let pbxproj_content = "SDKROOT = macosx;";
        std::fs::write(xcodeproj.join("project.pbxproj"), pbxproj_content).unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        assert_eq!(adapter.platform, Platform::MacOS);

        let devices = adapter.devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "My Mac");
    }
}
