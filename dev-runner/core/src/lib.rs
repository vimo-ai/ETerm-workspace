//! DevRunner Core
//!
//! Shared library for DevRunner - App and ETerm plugin.
//!
//! Core responsibilities:
//! - Project detection and parsing (adapter/)
//! - Command generation (build_cmd, run_cmd, log_cmd)
//! - Output formatting (output/)
//! - Device listing (adapter/xcode/devices)
//! - Configuration management (config/)
//!
//! Note: Process management is in dev-runner-app (App-only layer).

pub mod adapter;
pub mod config;
pub mod ffi;
pub mod output;

#[cfg(test)]
mod tests {
    #[test]
    fn test_lib_compiles() {
        // Basic sanity check
        assert!(true);
    }
}
