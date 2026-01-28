//! Bootstrap - DevRunner builds ETerm workspace
use dev_runner_core::adapter::{AdapterRegistry, BuildOptions, RunOptions};
use std::process::Command;

fn main() {
    let path = std::path::PathBuf::from("/Users/higuaifan/Desktop/vimo/ETerm");

    println!("🔄 DevRunner Bootstrap\n");
    println!("Detecting project(s) at: {}\n", path.display());

    let adapters = AdapterRegistry::detect(&path);

    if adapters.is_empty() {
        println!("✗ No projects found");
        return;
    }

    println!("Found {} project(s):\n", adapters.len());
    for (i, a) in adapters.iter().enumerate() {
        let targets = a.targets();
        let target_str = if targets.is_empty() {
            String::new()
        } else {
            format!(" [{}]", targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "))
        };
        println!("  {}. {} ({}){}", i + 1, a.name(), a.adapter_type(), target_str);
    }

    // Find DevRunner (self!)
    let adapter = adapters
        .into_iter()
        .find(|a| a.name() == "DevRunner" && a.adapter_type() == "xcode")
        .expect("DevRunner not found!");

    println!("\n✓ Selected: {} ({})", adapter.name(), adapter.adapter_type());
    println!("  Path: {}", adapter.path().display());
    println!("  Bundle ID: {:?}\n", adapter.bundle_id());

    let target = adapter.targets().first().expect("No targets").name.clone();
    println!("📦 Target: {}\n", target);

    // Build command
    let build_opts = BuildOptions {
        config: "Debug".to_string(),
        clean: false,
        device: None,
        env: Default::default(),
    };

    let build_cmd = adapter.build_cmd(&target, &build_opts).expect("No build cmd");
    println!("🔨 Build Command:");
    println!("   {}\n", build_cmd.to_string());

    println!("▶ Executing build...\n");

    let status = Command::new(&build_cmd.program)
        .args(&build_cmd.args)
        .current_dir(build_cmd.cwd.as_ref().unwrap_or(&adapter.path().to_path_buf()))
        .envs(&build_cmd.env)
        .status()
        .expect("Failed to execute");

    if status.success() {
        println!("\n✅ Build succeeded!\n");

        // Run command
        let run_opts = RunOptions {
            device: None,
            env: Default::default(),
            args: vec![],
        };

        let run_cmd = adapter.run_cmd(&target, &run_opts);
        println!("🚀 Run Command:");
        println!("   {}\n", run_cmd.to_string());

        println!("▶ Launching DevRunner...\n");

        let _ = Command::new(&run_cmd.program)
            .args(&run_cmd.args)
            .current_dir(run_cmd.cwd.as_ref().unwrap_or(&adapter.path().to_path_buf()))
            .envs(&run_cmd.env)
            .spawn()
            .expect("Failed to launch");

        println!("✅ DevRunner launched itself! 🔄 (true bootstrap)");
    } else {
        println!("\n❌ Build failed with: {:?}", status.code());
    }
}
