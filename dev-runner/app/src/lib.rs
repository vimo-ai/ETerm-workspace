//! DevRunner App Layer
//!
//! App-specific functionality for the standalone DevRunner application.
//!
//! Responsibilities:
//! - Process management (start, stop, monitor)
//! - Output capture and streaming
//! - FFI exports for Swift integration
//!
//! Note: ETerm plugin does NOT use this layer - it uses dev-runner-core directly.

pub mod ffi;
pub mod process;

// Re-export core types for convenience
pub use dev_runner_core::adapter::{
    BuildOptions, Command, Device, DeviceState, DeviceType, RunOptions, RunTarget, RunnerAdapter,
};
pub use dev_runner_core::output::{LogLevel, OutputEvent};

use std::sync::OnceLock;
use tokio::runtime::Runtime;

/// Global runtime instance for FFI calls
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Get or create the global tokio runtime
pub fn get_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_init() {
        let rt = get_runtime();
        rt.block_on(async {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        });
    }
}
