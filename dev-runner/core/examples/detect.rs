//! Detect project example
use dev_runner_core::adapter::AdapterRegistry;
use std::env;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let path = std::path::PathBuf::from(&path);

    println!("Detecting project(s) at: {}\n", path.display());

    let adapters = AdapterRegistry::detect(&path);

    if adapters.is_empty() {
        println!("✗ No project detected");
        return;
    }

    println!("Found {} project(s):\n", adapters.len());

    for (i, adapter) in adapters.iter().enumerate() {
        println!("{}. {} ({})", i + 1, adapter.name(), adapter.adapter_type());
        println!("   Path: {}", adapter.path().display());
        if let Some(bundle_id) = adapter.bundle_id() {
            println!("   Bundle ID: {}", bundle_id);
        }

        let targets = adapter.targets();
        if !targets.is_empty() {
            println!("   Targets: {}", targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "));
        }
        println!();
    }
}
