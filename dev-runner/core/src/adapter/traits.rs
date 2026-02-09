//! Core traits for RunnerAdapter

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A target that can be run (scheme, script, etc.)
#[derive(Debug, Clone)]
pub struct RunTarget {
    /// Target name (e.g., "MyApp", "dev", "build")
    pub name: String,
    /// Target type for display (e.g., "scheme", "script")
    pub target_type: String,
    /// Optional description
    pub description: Option<String>,
}

/// Options for building a project
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Build configuration (e.g., "Debug", "Release")
    pub config: String,
    /// Whether to clean before building
    pub clean: bool,
    /// Target device (Xcode only - affects architecture and SDK)
    pub device: Option<Device>,
    /// Additional environment variables
    pub env: HashMap<String, String>,
}

/// Options for running a project
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Target device (Xcode only)
    pub device: Option<Device>,
    /// Additional environment variables
    pub env: HashMap<String, String>,
    /// Additional arguments
    pub args: Vec<String>,
}

/// A device to run on (Xcode only)
#[derive(Debug, Clone)]
pub struct Device {
    /// Device identifier (UDID)
    pub id: String,
    /// Device name (e.g., "iPhone 15 Pro")
    pub name: String,
    /// Device type
    pub device_type: DeviceType,
    /// OS version (e.g., "17.2")
    pub os_version: Option<String>,
    /// Current state
    pub state: DeviceState,
}

/// Type of device
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceType {
    /// Local Mac
    Mac,
    /// iOS Simulator
    Simulator,
    /// Physical iOS device
    Physical,
}

/// State of a device
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceState {
    /// Device is available and running (simulator booted, device connected)
    Available,
    /// Device exists but not running (simulator shutdown)
    Unavailable,
    /// Unknown state
    Unknown,
}

/// A command to execute
#[derive(Debug, Clone)]
pub struct Command {
    /// Program to run
    pub program: String,
    /// Arguments
    pub args: Vec<String>,
    /// Working directory
    pub cwd: Option<PathBuf>,
    /// Environment variables
    pub env: HashMap<String, String>,
}

impl Command {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(|s| s.into()));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Convert to a displayable string (shell-safe)
    pub fn to_string(&self) -> String {
        let mut parts = vec![self.program.clone()];
        for arg in &self.args {
            if arg.contains(' ') || arg.contains('\'') || arg.contains('"') || arg.contains('$') || arg.contains('`') || arg.contains('\\') || arg.contains('(') || arg.contains(')') {
                // Shell-escape: wrap in single quotes, escape existing single quotes
                parts.push(format!("'{}'", arg.replace('\'', "'\\''")));
            } else {
                parts.push(arg.clone());
            }
        }
        parts.join(" ")
    }
}

/// Core trait for project adapters
///
/// Each adapter handles a specific project type (Xcode, Node, etc.)
pub trait RunnerAdapter: Send + Sync {
    /// Adapter type identifier (e.g., "xcode", "node")
    fn adapter_type(&self) -> &'static str;

    /// Project name
    fn name(&self) -> &str;

    /// Project path
    fn path(&self) -> &Path;

    /// List of runnable targets (schemes, scripts, etc.)
    fn targets(&self) -> Vec<RunTarget>;

    /// Get build command for a target (if applicable)
    fn build_cmd(&self, target: &str, options: &BuildOptions) -> Option<Command>;

    /// Get run command for a target
    fn run_cmd(&self, target: &str, options: &RunOptions) -> Command;

    /// Get log stream command for a target (if applicable)
    ///
    /// For Xcode projects, this generates a log stream command filtered by bundle ID.
    fn log_cmd(&self, target: &str, options: &RunOptions) -> Option<Command>;

    /// Get install command for a target (Xcode only)
    ///
    /// For simulators: simctl install
    /// For physical devices: devicectl install
    /// For macOS: None (no install needed)
    fn install_cmd(&self, target: &str, options: &RunOptions) -> Option<Command> {
        let _ = (target, options);
        None
    }

    /// List available devices (Xcode only, returns empty for others)
    fn devices(&self) -> Vec<Device> {
        Vec::new()
    }

    /// Format raw output (optional, default returns as-is)
    fn format_output(&self, raw: &str) -> String {
        raw.to_string()
    }

    /// Get bundle identifier (Xcode only)
    fn bundle_id(&self) -> Option<String> {
        None
    }
}
