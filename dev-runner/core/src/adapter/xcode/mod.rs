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

    /// Detect platform from project.pbxproj (SDKROOT)
    fn detect_platform(project_path: &Path) -> Platform {
        let pbxproj_path = project_path.join("project.pbxproj");
        let content = match std::fs::read_to_string(&pbxproj_path) {
            Ok(c) => c,
            Err(_) => return Platform::MacOS, // Default to macOS
        };

        // Look for SDKROOT to determine platform
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("SDKROOT") {
                if line.contains("appletvos") || line.contains("appletvsimulator") {
                    return Platform::TvOS;
                }
                if line.contains("watchos") || line.contains("watchsimulator") {
                    return Platform::WatchOS;
                }
                if line.contains("iphoneos") || line.contains("iphonesimulator") {
                    return Platform::IOS;
                }
                if line.contains("xros") || line.contains("xrsimulator") {
                    return Platform::VisionOS;
                }
                if line.contains("macosx") {
                    return Platform::MacOS;
                }
            }
        }

        // Fallback: check SUPPORTED_PLATFORMS
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("SUPPORTED_PLATFORMS") {
                if line.contains("appletvos") {
                    return Platform::TvOS;
                }
                if line.contains("watchos") {
                    return Platform::WatchOS;
                }
                if line.contains("iphoneos") {
                    return Platform::IOS;
                }
                if line.contains("xros") {
                    return Platform::VisionOS;
                }
            }
        }

        // Default to macOS
        Platform::MacOS
    }

    /// Parse schemes using xcodebuild -list (primary) or from xcscheme files (fallback)
    fn parse_schemes(&self) -> Vec<String> {
        // Try xcodebuild -list first (more reliable)
        if let Some(schemes) = self.parse_schemes_via_xcodebuild() {
            if !schemes.is_empty() {
                return schemes;
            }
        }

        // Fallback: parse xcscheme files directly
        self.parse_schemes_from_files()
    }

    /// Use xcodebuild -list to get schemes
    fn parse_schemes_via_xcodebuild(&self) -> Option<Vec<String>> {
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

        let schemes = json
            .get("project")?
            .get("schemes")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        Some(schemes)
    }

    /// Parse schemes from xcscheme files
    ///
    /// Looks for .xcscheme files in:
    /// - {project_path}/xcshareddata/xcschemes/*.xcscheme (shared schemes)
    /// - {project_path}/xcuserdata/*/xcschemes/*.xcscheme (user schemes)
    fn parse_schemes_from_files(&self) -> Vec<String> {
        let mut schemes = Vec::new();

        // Parse shared schemes
        let shared_schemes_dir = self.project_path.join("xcshareddata/xcschemes");
        if let Ok(entries) = std::fs::read_dir(&shared_schemes_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "xcscheme") {
                    if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                        schemes.push(name.to_string());
                    }
                }
            }
        }

        // Parse user schemes
        let userdata_dir = self.project_path.join("xcuserdata");
        if let Ok(users) = std::fs::read_dir(&userdata_dir) {
            for user_entry in users.filter_map(|e| e.ok()) {
                let user_schemes_dir = user_entry.path().join("xcschemes");
                if let Ok(entries) = std::fs::read_dir(&user_schemes_dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.extension().map_or(false, |ext| ext == "xcscheme") {
                            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                                // Avoid duplicates
                                if !schemes.contains(&name.to_string()) {
                                    schemes.push(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        schemes
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
                // xcrun simctl launch --console <device-id> <bundle-id> [args...]
                // --console captures app's stdout/stderr
                let mut cmd = Command::new("xcrun")
                    .arg("simctl")
                    .arg("launch")
                    .arg("--console")
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

        // For iOS projects that support Mac Catalyst, add Mac first
        if self.platform == Platform::IOS {
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

    #[test]
    fn test_parse_schemes_shared() {
        let dir = tempdir().unwrap();
        let xcodeproj = dir.path().join("MyApp.xcodeproj");
        let schemes_dir = xcodeproj.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes_dir).unwrap();

        // Create scheme files
        std::fs::write(schemes_dir.join("MyApp.xcscheme"), "<Scheme></Scheme>").unwrap();
        std::fs::write(schemes_dir.join("MyAppTests.xcscheme"), "<Scheme></Scheme>").unwrap();

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

        // Create user scheme file
        std::fs::write(user_schemes_dir.join("UserScheme.xcscheme"), "<Scheme></Scheme>").unwrap();

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
        std::fs::write(shared_dir.join("MyApp.xcscheme"), "<Scheme></Scheme>").unwrap();

        // Create same scheme in user data
        let user_dir = xcodeproj.join("xcuserdata/test.xcuserdatad/xcschemes");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(user_dir.join("MyApp.xcscheme"), "<Scheme></Scheme>").unwrap();

        let adapter = XcodeAdapter::new(xcodeproj);
        let schemes = adapter.parse_schemes();

        // Should not have duplicates
        assert_eq!(schemes.len(), 1);
        assert!(schemes.contains(&"MyApp".to_string()));
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

        std::fs::write(schemes_dir.join("MyApp.xcscheme"), "<Scheme></Scheme>").unwrap();

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
}
