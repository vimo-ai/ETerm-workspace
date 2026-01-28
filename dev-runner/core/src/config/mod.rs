//! Configuration module
//!
//! Handles loading and saving configuration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Global configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Version for migration
    pub version: u32,
    /// Xcode-specific settings
    pub xcode: XcodeConfig,
    /// Node-specific settings
    pub node: NodeConfig,
    /// Log settings
    pub log: LogConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            xcode: XcodeConfig::default(),
            node: NodeConfig::default(),
            log: LogConfig::default(),
        }
    }
}

/// Xcode-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XcodeConfig {
    /// Default device name
    pub default_device: Option<String>,
    /// Default build configuration
    pub default_config: String,
    /// Output format (pretty or raw)
    pub output_format: String,
}

impl Default for XcodeConfig {
    fn default() -> Self {
        Self {
            default_device: None,
            default_config: "Debug".to_string(),
            output_format: "pretty".to_string(),
        }
    }
}

/// Node-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Default package manager
    pub default_package_manager: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            default_package_manager: "npm".to_string(),
        }
    }
}

/// Log configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Days to retain logs
    pub retention_days: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self { retention_days: 7 }
    }
}

/// Project list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectList {
    /// Version for migration
    pub version: u32,
    /// List of projects
    pub projects: Vec<ProjectEntry>,
}

impl Default for ProjectList {
    fn default() -> Self {
        Self {
            version: 1,
            projects: Vec::new(),
        }
    }
}

/// A project entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    /// Project path
    pub path: PathBuf,
    /// Project type (xcode, node, etc.)
    #[serde(rename = "type")]
    pub project_type: String,
    /// When project was added
    pub added_at: chrono::DateTime<chrono::Utc>,
    /// When project was last used
    pub last_used_at: chrono::DateTime<chrono::Utc>,
    /// Project-specific settings
    pub settings: HashMap<String, serde_json::Value>,
}

/// Configuration manager
pub struct ConfigManager {
    /// Base directory for config files
    config_dir: PathBuf,
}

impl ConfigManager {
    /// Create a new config manager
    pub fn new() -> Self {
        let config_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".vimo")
            .join("dev-runner");

        Self { config_dir }
    }

    /// Create with custom config directory
    pub fn with_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Ensure config directory exists
    pub fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(self.config_dir.join("logs"))?;
        Ok(())
    }

    /// Get config directory path
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Load global config
    pub fn load_config(&self) -> Config {
        let path = self.config_dir.join("config.json");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str(&content) {
                    return config;
                }
            }
        }
        Config::default()
    }

    /// Save global config
    pub fn save_config(&self, config: &Config) -> std::io::Result<()> {
        self.ensure_dir()?;
        let path = self.config_dir.join("config.json");
        let content = serde_json::to_string_pretty(config)?;
        std::fs::write(path, content)
    }

    /// Load project list
    pub fn load_projects(&self) -> ProjectList {
        let path = self.config_dir.join("projects.json");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(list) = serde_json::from_str(&content) {
                    return list;
                }
            }
        }
        ProjectList::default()
    }

    /// Save project list
    pub fn save_projects(&self, projects: &ProjectList) -> std::io::Result<()> {
        self.ensure_dir()?;
        let path = self.config_dir.join("projects.json");
        let content = serde_json::to_string_pretty(projects)?;
        std::fs::write(path, content)
    }

    /// Get log directory for today
    pub fn log_dir_today(&self) -> PathBuf {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        self.config_dir.join("logs").join(today)
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.version, 1);
        assert_eq!(config.xcode.default_config, "Debug");
    }

    #[test]
    fn test_config_manager() {
        let dir = tempdir().unwrap();
        let manager = ConfigManager::with_dir(dir.path().to_path_buf());

        // Save and load config
        let config = Config::default();
        manager.save_config(&config).unwrap();

        let loaded = manager.load_config();
        assert_eq!(loaded.version, config.version);
    }
}
