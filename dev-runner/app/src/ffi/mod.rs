//! FFI Module
//!
//! C ABI exports for Swift integration.
//!
//! Design:
//! - Opaque handle pattern for state management
//! - JSON strings for complex data structures
//! - vimo-ffi for panic safety and string conversion

mod types;
mod exports;

pub use exports::*;
pub use types::*;
