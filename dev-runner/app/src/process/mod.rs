//! Process Management
//!
//! Handles process lifecycle: start, stop, monitor, and output capture.

mod manager;
mod monitor;

pub use manager::{ProcessError, ProcessManager};
pub use monitor::ProcessMonitor;

use chrono::{DateTime, Utc};
use std::path::PathBuf;

/// A running or completed process
#[derive(Debug, Clone)]
pub struct Process {
    /// Unique process ID (UUID)
    pub id: String,
    /// Project path
    pub project_path: PathBuf,
    /// Adapter type (e.g., "xcode", "node")
    pub adapter_type: String,
    /// Target name (e.g., scheme name, script name)
    pub target: String,
    /// OS process ID
    pub pid: Option<u32>,
    /// Current status
    pub status: ProcessStatus,
    /// When the process started
    pub started_at: DateTime<Utc>,
    /// When the process ended (if applicable)
    pub ended_at: Option<DateTime<Utc>>,
}

impl Process {
    /// Create a new process record
    pub fn new(
        id: String,
        project_path: PathBuf,
        adapter_type: String,
        target: String,
        pid: u32,
    ) -> Self {
        Self {
            id,
            project_path,
            adapter_type,
            target,
            pid: Some(pid),
            status: ProcessStatus::Running,
            started_at: Utc::now(),
            ended_at: None,
        }
    }

    /// Check if the process is still running
    pub fn is_running(&self) -> bool {
        matches!(self.status, ProcessStatus::Running)
    }
}

/// Process status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Process is running
    Running,
    /// Process stopped normally (exit code 0)
    Stopped,
    /// Process failed (non-zero exit code)
    Failed { code: i32, message: String },
}
