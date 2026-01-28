//! FFI Types
//!
//! C-compatible types for Swift interop.

use std::collections::HashMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::process::{ProcessManager, ProcessMonitor};
use dev_runner_core::adapter::RunnerAdapter;

/// DevRunner handle - opaque pointer for Swift
pub struct DevRunnerHandle {
    /// Multiple projects, keyed by path
    pub projects: RwLock<HashMap<String, Box<dyn RunnerAdapter>>>,
    /// Process manager
    pub process_manager: ProcessManager,
    /// Process monitor
    pub process_monitor: RwLock<ProcessMonitor>,
}

impl DevRunnerHandle {
    pub fn new() -> Self {
        Self {
            projects: RwLock::new(HashMap::new()),
            process_manager: ProcessManager::new(),
            process_monitor: RwLock::new(ProcessMonitor::new()),
        }
    }

    /// Add a project
    pub fn add_project(&self, adapter: Box<dyn RunnerAdapter>) -> String {
        let key = adapter.path().to_string_lossy().to_string();
        self.projects.write().insert(key.clone(), adapter);
        key
    }

    /// Get a project by path
    pub fn get_project(&self, path: &str) -> Option<std::sync::Arc<Box<dyn RunnerAdapter>>> {
        // Can't return reference directly due to RwLock, clone the adapter info instead
        None // We'll access directly via the lock
    }

    /// Remove a project
    pub fn remove_project(&self, path: &str) -> bool {
        self.projects.write().remove(path).is_some()
    }
}

impl Default for DevRunnerHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// FFI Error codes
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevRunnerError {
    Success = 0,
    NullPointer = 1,
    InvalidUtf8 = 2,
    InvalidJson = 3,
    NoProject = 4,
    NoAdapter = 5,
    ProcessError = 6,
    RuntimeError = 7,
    InternalPanic = 99,
}

// ============================================================================
// JSON Response Types (returned to Swift as JSON strings)
// ============================================================================

/// Project detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Adapter type (e.g., "xcode", "node")
    pub adapter_type: String,
    /// Project name
    pub name: String,
    /// Project path
    pub path: String,
    /// Bundle identifier (Xcode only)
    pub bundle_id: Option<String>,
}

/// Run target (scheme, script, etc.)
#[derive(Debug, Clone, Serialize)]
pub struct TargetInfo {
    /// Target name
    pub name: String,
    /// Target type (e.g., "scheme", "script")
    pub target_type: String,
    /// Description
    pub description: Option<String>,
}

/// Device info
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    /// Device identifier
    pub id: String,
    /// Device name
    pub name: String,
    /// Device type: "mac", "simulator", "physical"
    pub device_type: String,
    /// OS version
    pub os_version: Option<String>,
    /// State: "available", "unavailable", "unknown"
    pub state: String,
}

/// Command info (for execution)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    /// Program to run
    pub program: String,
    /// Arguments
    pub args: Vec<String>,
    /// Working directory
    pub cwd: Option<String>,
    /// Environment variables
    pub env: std::collections::HashMap<String, String>,
    /// Display string
    pub display: String,
}

impl From<&dev_runner_core::adapter::Command> for CommandInfo {
    fn from(cmd: &dev_runner_core::adapter::Command) -> Self {
        Self {
            program: cmd.program.clone(),
            args: cmd.args.clone(),
            cwd: cmd.cwd.as_ref().map(|p| p.to_string_lossy().to_string()),
            env: cmd.env.clone(),
            display: cmd.to_string(),
        }
    }
}

/// Process start result
#[derive(Debug, Clone, Serialize)]
pub struct ProcessStartResult {
    /// Process ID (UUID)
    pub process_id: String,
}

/// Process metrics
#[derive(Debug, Clone, Serialize)]
pub struct MetricsInfo {
    /// Process ID (system PID)
    pub pid: u32,
    /// CPU usage percentage
    pub cpu_percent: f32,
    /// Memory usage in bytes
    pub memory_bytes: u64,
    /// Timestamp (Unix epoch seconds)
    pub timestamp: i64,
}

/// Process status
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    /// Process ID (UUID)
    pub id: String,
    /// Project path
    pub project_path: String,
    /// Adapter type
    pub adapter_type: String,
    /// Target name
    pub target: String,
    /// System PID
    pub pid: Option<u32>,
    /// Status: "running", "stopped", "failed"
    pub status: String,
    /// Error message (if failed)
    pub error_message: Option<String>,
    /// Started at (Unix epoch seconds)
    pub started_at: i64,
    /// Ended at (Unix epoch seconds)
    pub ended_at: Option<i64>,
}

// ============================================================================
// Build/Run Options (passed from Swift as JSON)
// ============================================================================

/// Build options (from Swift JSON)
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct BuildOptionsInput {
    /// Build configuration
    #[serde(default)]
    pub config: Option<String>,
    /// Clean build
    #[serde(default)]
    pub clean: bool,
    /// Device ID
    #[serde(default)]
    pub device_id: Option<String>,
    /// Environment variables
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Run options (from Swift JSON)
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RunOptionsInput {
    /// Device ID
    #[serde(default)]
    pub device_id: Option<String>,
    /// Environment variables
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Arguments
    #[serde(default)]
    pub args: Vec<String>,
}

/// Output read result
#[derive(Debug, Clone, Serialize)]
pub struct OutputReadResult {
    /// Lines read
    pub lines: Vec<String>,
    /// Next line number to read from
    pub next_line: usize,
}
