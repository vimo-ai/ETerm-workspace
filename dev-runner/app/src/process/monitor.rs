//! Process Monitor
//!
//! Monitors CPU and memory usage of running processes.

use chrono::{DateTime, Utc};
use sysinfo::{Pid, System};

/// CPU and memory metrics for a process
#[derive(Debug, Clone)]
pub struct ProcessMetrics {
    /// Process ID
    pub pid: u32,
    /// CPU usage percentage (0-100 per core, can exceed 100 on multi-core)
    pub cpu_percent: f32,
    /// Memory usage in bytes
    pub memory_bytes: u64,
    /// Timestamp when metrics were collected
    pub timestamp: DateTime<Utc>,
}

/// Process monitor for collecting CPU/memory metrics
pub struct ProcessMonitor {
    system: System,
}

impl ProcessMonitor {
    /// Create a new process monitor
    pub fn new() -> Self {
        Self {
            system: System::new_all(),
        }
    }

    /// Refresh system information
    pub fn refresh(&mut self) {
        self.system.refresh_all();
    }

    /// Get metrics for a specific process
    pub fn get_metrics(&mut self, pid: u32) -> Option<ProcessMetrics> {
        // Refresh process info
        self.system.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
        );

        let process = self.system.process(Pid::from_u32(pid))?;

        Some(ProcessMetrics {
            pid,
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
            timestamp: Utc::now(),
        })
    }

    /// Get metrics for multiple processes
    pub fn get_metrics_batch(&mut self, pids: &[u32]) -> Vec<ProcessMetrics> {
        // Refresh all specified processes
        let sysinfo_pids: Vec<Pid> = pids.iter().map(|&p| Pid::from_u32(p)).collect();
        self.system
            .refresh_processes(sysinfo::ProcessesToUpdate::Some(&sysinfo_pids), true);

        pids.iter()
            .filter_map(|&pid| {
                let process = self.system.process(Pid::from_u32(pid))?;
                Some(ProcessMetrics {
                    pid,
                    cpu_percent: process.cpu_usage(),
                    memory_bytes: process.memory(),
                    timestamp: Utc::now(),
                })
            })
            .collect()
    }

    /// Check if a process is still running
    pub fn is_running(&mut self, pid: u32) -> bool {
        self.system.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            false,
        );
        self.system.process(Pid::from_u32(pid)).is_some()
    }
}

impl Default for ProcessMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_new() {
        let _monitor = ProcessMonitor::new();
    }

    #[test]
    fn test_monitor_current_process() {
        let mut monitor = ProcessMonitor::new();
        let pid = std::process::id();

        let metrics = monitor.get_metrics(pid);
        assert!(metrics.is_some());

        let metrics = metrics.unwrap();
        assert_eq!(metrics.pid, pid);
        // Memory should be non-zero for our own process
        assert!(metrics.memory_bytes > 0);
    }

    #[test]
    fn test_monitor_nonexistent_process() {
        let mut monitor = ProcessMonitor::new();
        // Use a very high PID that's unlikely to exist
        let metrics = monitor.get_metrics(999999999);
        assert!(metrics.is_none());
    }

    #[test]
    fn test_is_running() {
        let mut monitor = ProcessMonitor::new();
        let pid = std::process::id();

        // Our own process should be running
        assert!(monitor.is_running(pid));

        // Non-existent process should not be running
        assert!(!monitor.is_running(999999999));
    }
}
