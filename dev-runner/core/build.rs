//! Build script for generating C header files

fn main() {
    // Generate C header file using cbindgen
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    let config = cbindgen::Config::from_file("cbindgen.toml").unwrap_or_default();

    if let Ok(bindings) = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        let out_dir = std::path::Path::new(&crate_dir).join("include");
        std::fs::create_dir_all(&out_dir).ok();
        bindings.write_to_file(out_dir.join("dev_runner.h"));
    }
}
