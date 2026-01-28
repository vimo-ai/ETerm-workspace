//! Build script for dev-runner-app
//!
//! Generates C header file using cbindgen.

use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = PathBuf::from(&crate_dir).join("include");

    // Create include directory if it doesn't exist
    std::fs::create_dir_all(&out_dir).ok();

    // Generate C header
    let config = cbindgen::Config::from_file("cbindgen.toml")
        .unwrap_or_else(|_| cbindgen::Config::default());

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .map(|bindings| {
            bindings.write_to_file(out_dir.join("dev_runner.h"));
        })
        .ok();

    // Rebuild if FFI module changes
    println!("cargo:rerun-if-changed=src/ffi/mod.rs");
    println!("cargo:rerun-if-changed=src/ffi/types.rs");
    println!("cargo:rerun-if-changed=src/ffi/exports.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
}
