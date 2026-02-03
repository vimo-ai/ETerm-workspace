//! Node Adapter
//!
//! Handles Node.js projects (package.json).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::adapter::{BuildOptions, Command, RunOptions, RunTarget, RunnerAdapter};

/// Package manager type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
}

impl PackageManager {
    /// Detect package manager from project directory
    ///
    /// Searches current directory and parent directories for lock files
    /// (supports monorepo where lock file is at root)
    pub fn detect(path: &Path) -> Self {
        let mut current = path.to_path_buf();

        loop {
            if current.join("pnpm-lock.yaml").exists() {
                return PackageManager::Pnpm;
            } else if current.join("yarn.lock").exists() {
                return PackageManager::Yarn;
            } else if current.join("package-lock.json").exists() {
                return PackageManager::Npm;
            }

            // Move to parent directory
            if !current.pop() {
                break;
            }
        }

        // Default to npm if no lock file found
        PackageManager::Npm
    }

    /// Get the run command prefix
    pub fn run_cmd(&self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
        }
    }
}

/// Node.js project adapter
pub struct NodeAdapter {
    /// Path to project directory (containing package.json)
    project_path: PathBuf,
    /// Project name (from package.json)
    name: String,
    /// Available scripts (from package.json)
    scripts: HashMap<String, String>,
    /// Detected package manager
    package_manager: PackageManager,
}

impl NodeAdapter {
    /// Detect if path contains a Node.js project
    pub fn detect(path: &Path) -> Option<Box<dyn RunnerAdapter>> {
        let package_json = path.join("package.json");
        if package_json.exists() {
            return Self::from_package_json(&package_json).map(|a| Box::new(a) as Box<dyn RunnerAdapter>);
        }
        None
    }

    /// Detect all Node.js projects in a directory
    pub fn detect_all(path: &Path) -> Vec<Box<dyn RunnerAdapter>> {
        let mut adapters: Vec<Box<dyn RunnerAdapter>> = Vec::new();

        let package_json = path.join("package.json");
        if package_json.exists() {
            if let Some(adapter) = Self::from_package_json(&package_json) {
                adapters.push(Box::new(adapter));
            }
        }

        adapters
    }

    /// Parse package.json and create adapter
    fn from_package_json(package_json: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(package_json).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;

        let name = json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let scripts = json
            .get("scripts")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let project_path = package_json.parent()?.to_path_buf();
        let package_manager = PackageManager::detect(&project_path);

        Some(Self {
            project_path,
            name,
            scripts,
            package_manager,
        })
    }
}

/// Script 优先级（数字越小越靠前）
fn script_priority(name: &str) -> u8 {
    let name_lower = name.to_lowercase();
    if name_lower.starts_with("dev") {
        1
    } else if name_lower.starts_with("start") {
        2
    } else if name_lower.starts_with("build") {
        3
    } else if name_lower.starts_with("test") {
        4
    } else if name_lower.starts_with("lint") {
        5
    } else if name_lower.starts_with("format") || name_lower.starts_with("fmt") {
        6
    } else {
        99
    }
}

impl RunnerAdapter for NodeAdapter {
    fn adapter_type(&self) -> &'static str {
        "node"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn path(&self) -> &Path {
        &self.project_path
    }

    fn targets(&self) -> Vec<RunTarget> {
        let mut targets: Vec<RunTarget> = self
            .scripts
            .keys()
            .map(|name| RunTarget {
                name: name.clone(),
                target_type: "script".to_string(),
                description: self.scripts.get(name).cloned(),
            })
            .collect();

        // 智能排序：常用 script 优先，其他字母序
        targets.sort_by(|a, b| {
            let priority_a = script_priority(&a.name);
            let priority_b = script_priority(&b.name);
            priority_a.cmp(&priority_b).then_with(|| a.name.cmp(&b.name))
        });

        targets
    }

    fn build_cmd(&self, target: &str, _options: &BuildOptions) -> Option<Command> {
        // Check if there's a build script
        if self.scripts.contains_key("build") || target == "build" {
            Some(
                Command::new(self.package_manager.run_cmd())
                    .arg("run")
                    .arg("build")
                    .cwd(&self.project_path),
            )
        } else {
            None
        }
    }

    fn run_cmd(&self, target: &str, _options: &RunOptions) -> Command {
        Command::new(self.package_manager.run_cmd())
            .arg("run")
            .arg(target)
            .cwd(&self.project_path)
    }

    fn log_cmd(&self, _target: &str, _options: &RunOptions) -> Option<Command> {
        // Node projects output to stdout directly
        // No separate log command needed
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_detect_no_project() {
        let dir = tempdir().unwrap();
        let result = NodeAdapter::detect(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_detect_with_package_json() {
        let dir = tempdir().unwrap();
        let package_json = dir.path().join("package.json");
        std::fs::write(
            &package_json,
            r#"{
                "name": "my-app",
                "scripts": {
                    "dev": "node index.js",
                    "build": "tsc"
                }
            }"#,
        )
        .unwrap();

        let result = NodeAdapter::detect(dir.path());
        assert!(result.is_some());

        let adapter = result.unwrap();
        assert_eq!(adapter.adapter_type(), "node");
        assert_eq!(adapter.name(), "my-app");

        let targets = adapter.targets();
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn test_package_manager_detection() {
        let dir = tempdir().unwrap();

        // Default to npm
        assert_eq!(PackageManager::detect(dir.path()), PackageManager::Npm);

        // Detect pnpm
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(PackageManager::detect(dir.path()), PackageManager::Pnpm);
    }

    #[test]
    fn test_run_cmd() {
        let dir = tempdir().unwrap();
        let package_json = dir.path().join("package.json");
        std::fs::write(
            &package_json,
            r#"{"name": "test", "scripts": {"dev": "node index.js"}}"#,
        )
        .unwrap();

        let adapter = NodeAdapter::from_package_json(&package_json).unwrap();
        let cmd = adapter.run_cmd("dev", &RunOptions::default());

        assert_eq!(cmd.program, "npm");
        assert!(cmd.args.contains(&"run".to_string()));
        assert!(cmd.args.contains(&"dev".to_string()));
    }
}
