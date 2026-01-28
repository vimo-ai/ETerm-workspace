//! Process Manager
//!
//! Manages the lifecycle of running processes.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;

use dev_runner_core::adapter::Command;
use dev_runner_core::output::OutputEvent;

use super::{Process, ProcessStatus};

/// Maximum number of lines to keep in output buffer
const MAX_OUTPUT_LINES: usize = 500;

/// Output buffer for a single process
#[derive(Debug, Clone)]
struct OutputBuffer {
    /// Buffered lines (max 500)
    lines: VecDeque<String>,
    /// Next line number to be assigned
    next_line: usize,
}

impl OutputBuffer {
    fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            next_line: 0,
        }
    }

    fn push(&mut self, line: String) {
        if self.lines.len() >= MAX_OUTPUT_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.next_line += 1;
    }

    fn read_from(&self, since_line: usize) -> Vec<String> {
        let start_line = self.next_line.saturating_sub(self.lines.len());
        if since_line < start_line {
            // Requested line was already discarded, return all we have
            self.lines.iter().cloned().collect()
        } else if since_line >= self.next_line {
            // Already up to date
            Vec::new()
        } else {
            // Return lines from since_line to next_line
            let skip = since_line.saturating_sub(start_line);
            self.lines.iter().skip(skip).cloned().collect()
        }
    }

    fn next_line(&self) -> usize {
        self.next_line
    }
}

/// Process manager handles starting, stopping, and tracking processes
pub struct ProcessManager {
    /// Running processes
    processes: Arc<RwLock<HashMap<String, Process>>>,
    /// Output buffers for each process
    output_buffers: Arc<RwLock<HashMap<String, OutputBuffer>>>,
}

impl ProcessManager {
    /// Create a new process manager
    pub fn new() -> Self {
        Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
            output_buffers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get user's shell PATH
    ///
    /// GUI apps don't inherit shell environment, so we need to get PATH
    /// from the user's default shell.
    fn get_shell_path() -> Option<String> {
        // Try to get PATH from user's shell
        let output = std::process::Command::new("/bin/zsh")
            .args(["-l", "-c", "echo $PATH"])
            .output()
            .or_else(|_| {
                std::process::Command::new("/bin/bash")
                    .args(["-l", "-c", "echo $PATH"])
                    .output()
            })
            .ok()?;

        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }

        // Fallback: common paths
        Some("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin".to_string())
    }

    /// Start a process from a command
    ///
    /// Returns process ID and output channel
    pub async fn start(
        &self,
        command: Command,
        project_path: std::path::PathBuf,
        adapter_type: &str,
        target: &str,
    ) -> Result<(String, mpsc::Receiver<OutputEvent>), ProcessError> {
        let process_id = uuid::Uuid::new_v4().to_string();

        // Build shell command string
        let mut shell_cmd = String::new();

        // cd to working directory if specified
        if let Some(cwd) = &command.cwd {
            let cwd_str = cwd.to_string_lossy().replace("'", "'\\''");
            shell_cmd.push_str(&format!("cd '{}' && ", cwd_str));
        }

        // Add the actual command
        shell_cmd.push_str(&command.program);
        for arg in &command.args {
            let escaped = arg.replace("'", "'\\''");
            shell_cmd.push_str(&format!(" '{}'", escaped));
        }

        // Use `script` to create a pseudo-tty (forces line buffering)
        // script -q /dev/null zsh -l -c "command"
        let mut cmd = TokioCommand::new("script");
        cmd.arg("-q")
           .arg("/dev/null")
           .arg("/bin/zsh")
           .arg("-l")
           .arg("-c")
           .arg(&shell_cmd);

        // Set environment variables
        cmd.env("FORCE_COLOR", "1");

        for (key, value) in &command.env {
            cmd.env(key, value);
        }

        // Setup stdout/stderr capture
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Spawn the process
        let mut child = cmd.spawn().map_err(|e| ProcessError::SpawnFailed {
            command: command.to_string(),
            error: e.to_string(),
        })?;

        let pid = child.id().ok_or(ProcessError::NoPid)?;

        // Create process record
        let process = Process::new(
            process_id.clone(),
            project_path,
            adapter_type.to_string(),
            target.to_string(),
            pid,
        );

        // Store in our map
        self.processes.write().insert(process_id.clone(), process);

        // Create output buffer
        self.output_buffers.write().insert(process_id.clone(), OutputBuffer::new());

        // Create output channel
        let (tx, rx) = mpsc::channel(1000);

        // Take stdout and stderr
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Spawn task to handle output and wait for process
        let processes = self.processes.clone();
        let output_buffers = self.output_buffers.clone();
        let pid_clone = process_id.clone();
        let pid_for_stdout = process_id.clone();
        let pid_for_stderr = process_id.clone();

        tokio::spawn(async move {
            // Spawn stdout reader - read chunks instead of lines for immediate output
            let tx_stdout = tx.clone();
            let buffers_stdout = output_buffers.clone();
            let stdout_handle = if let Some(mut stdout) = stdout {
                Some(tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let mut partial_line = String::new();

                    loop {
                        match stdout.read(&mut buf).await {
                            Ok(0) => break, // EOF
                            Ok(n) => {
                                let chunk = String::from_utf8_lossy(&buf[..n]);
                                partial_line.push_str(&chunk);

                                // Split into lines and process
                                while let Some(newline_pos) = partial_line.find('\n') {
                                    let line = partial_line[..newline_pos].to_string();
                                    partial_line = partial_line[newline_pos + 1..].to_string();

                                    // Write to buffer
                                    if let Some(buffer) = buffers_stdout.write().get_mut(&pid_for_stdout) {
                                        buffer.push(line.clone());
                                    }

                                    // Send to channel
                                    let event = OutputEvent::stdout(line);
                                    if tx_stdout.send(event).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }

                    // Handle any remaining partial line
                    if !partial_line.is_empty() {
                        if let Some(buffer) = buffers_stdout.write().get_mut(&pid_for_stdout) {
                            buffer.push(partial_line.clone());
                        }
                        let _ = tx_stdout.send(OutputEvent::stdout(partial_line)).await;
                    }
                }))
            } else {
                None
            };

            // Spawn stderr reader - same chunk-based approach
            let tx_stderr = tx.clone();
            let buffers_stderr = output_buffers.clone();
            let stderr_handle = if let Some(mut stderr) = stderr {
                Some(tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let mut partial_line = String::new();

                    loop {
                        match stderr.read(&mut buf).await {
                            Ok(0) => break, // EOF
                            Ok(n) => {
                                let chunk = String::from_utf8_lossy(&buf[..n]);
                                partial_line.push_str(&chunk);

                                // Split into lines and process
                                while let Some(newline_pos) = partial_line.find('\n') {
                                    let line = partial_line[..newline_pos].to_string();
                                    partial_line = partial_line[newline_pos + 1..].to_string();

                                    // Write to buffer
                                    if let Some(buffer) = buffers_stderr.write().get_mut(&pid_for_stderr) {
                                        buffer.push(line.clone());
                                    }

                                    // Send to channel
                                    let event = OutputEvent::stderr(line);
                                    if tx_stderr.send(event).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }

                    // Handle any remaining partial line
                    if !partial_line.is_empty() {
                        if let Some(buffer) = buffers_stderr.write().get_mut(&pid_for_stderr) {
                            buffer.push(partial_line.clone());
                        }
                        let _ = tx_stderr.send(OutputEvent::stderr(partial_line)).await;
                    }
                }))
            } else {
                None
            };

            // Wait for process to exit
            let exit_status = child.wait().await;

            // Wait for output readers to finish
            if let Some(handle) = stdout_handle {
                let _ = handle.await;
            }
            if let Some(handle) = stderr_handle {
                let _ = handle.await;
            }

            // Update process status
            let status = match exit_status {
                Ok(status) if status.success() => ProcessStatus::Stopped,
                Ok(status) => ProcessStatus::Failed {
                    code: status.code().unwrap_or(-1),
                    message: format!("Process exited with code {}", status.code().unwrap_or(-1)),
                },
                Err(e) => ProcessStatus::Failed {
                    code: -1,
                    message: e.to_string(),
                },
            };

            // Update process record
            processes.write().entry(pid_clone).and_modify(|p| {
                p.status = status;
                p.ended_at = Some(Utc::now());
            });
        });

        Ok((process_id, rx))
    }

    /// Stop a running process
    pub async fn stop(&self, process_id: &str) -> Result<(), ProcessError> {
        let process = self
            .processes
            .read()
            .get(process_id)
            .cloned()
            .ok_or(ProcessError::NotFound(process_id.to_string()))?;

        if !process.is_running() {
            return Err(ProcessError::AlreadyStopped);
        }

        if let Some(pid) = process.pid {
            // Send SIGTERM
            #[cfg(unix)]
            {
                let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
                if result != 0 {
                    // Try SIGKILL if SIGTERM failed
                    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
            }

            #[cfg(not(unix))]
            {
                // On non-unix, we can't easily send signals
                // The process will be cleaned up when it exits
                tracing::warn!("Cannot send signal on non-unix platform");
            }
        }

        Ok(())
    }

    /// List all running processes
    pub fn list_running(&self) -> Vec<Process> {
        self.processes
            .read()
            .values()
            .filter(|p| p.is_running())
            .cloned()
            .collect()
    }

    /// List all processes (including stopped)
    pub fn list_all(&self) -> Vec<Process> {
        self.processes.read().values().cloned().collect()
    }

    /// Get a process by ID
    pub fn get(&self, process_id: &str) -> Option<Process> {
        self.processes.read().get(process_id).cloned()
    }

    /// Remove a stopped process from tracking
    pub fn remove(&self, process_id: &str) {
        self.processes.write().remove(process_id);
    }

    /// Clean up all stopped processes
    pub fn cleanup_stopped(&self) {
        self.processes
            .write()
            .retain(|_, p| p.is_running());
    }

    /// Read output for a process since a given line number
    ///
    /// Returns (lines, next_line_number)
    pub fn read_output(&self, process_id: &str, since_line: usize) -> (Vec<String>, usize) {
        let buffers = self.output_buffers.read();
        if let Some(buffer) = buffers.get(process_id) {
            let lines = buffer.read_from(since_line);
            let next_line = buffer.next_line();
            (lines, next_line)
        } else {
            (Vec::new(), since_line)
        }
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during process management
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("Failed to spawn process '{command}': {error}")]
    SpawnFailed { command: String, error: String },

    #[error("Process has no PID")]
    NoPid,

    #[error("Process not found: {0}")]
    NotFound(String),

    #[error("Process already stopped")]
    AlreadyStopped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_manager_new() {
        let manager = ProcessManager::new();
        assert!(manager.list_running().is_empty());
    }

    #[tokio::test]
    async fn test_start_and_wait_echo() {
        let manager = ProcessManager::new();

        let command = Command::new("echo").arg("hello world");

        let (process_id, mut rx) = manager
            .start(
                command,
                std::path::PathBuf::from("/tmp"),
                "test",
                "echo",
            )
            .await
            .unwrap();

        // Collect output
        let mut output = Vec::new();
        while let Some(event) = rx.recv().await {
            output.push(event.content);
        }

        // Wait a bit for status update
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Check output
        assert!(output.iter().any(|s| s.contains("hello world")));

        // Check process status
        let process = manager.get(&process_id).unwrap();
        assert_eq!(process.status, ProcessStatus::Stopped);
    }

    #[tokio::test]
    async fn test_start_and_stop_sleep() {
        let manager = ProcessManager::new();

        let command = Command::new("sleep").arg("10");

        let (process_id, _rx) = manager
            .start(
                command,
                std::path::PathBuf::from("/tmp"),
                "test",
                "sleep",
            )
            .await
            .unwrap();

        // Process should be running
        let process = manager.get(&process_id).unwrap();
        assert!(process.is_running());

        // Stop it
        manager.stop(&process_id).await.unwrap();

        // Wait a bit for status update
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Check process status - should be stopped or failed
        let process = manager.get(&process_id).unwrap();
        assert!(!process.is_running());
    }
}
