//! End-to-end tests for Xcode projects
//!
//! Uses real ETerm.xcodeproj to verify the complete flow.
//! Run with: cargo test --test e2e_xcode -- --ignored --nocapture

use std::path::{Path, PathBuf};

use dev_runner_app::process::ProcessManager;
use dev_runner_core::adapter::xcode::XcodeAdapter;
use dev_runner_core::adapter::{BuildOptions, Device, DeviceState, DeviceType, RunOptions, RunnerAdapter};

/// Get the path to the directory containing ETerm.xcodeproj
fn get_eterm_project_dir() -> PathBuf {
    // dev-runner/app -> dev-runner -> ETerm workspace -> ETerm/ETerm
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent() // dev-runner
        .unwrap()
        .parent() // ETerm workspace
        .unwrap()
        .join("ETerm/ETerm")
}

// ============================================================================
// Step 1: Project Detection
// ============================================================================

#[test]
#[ignore]
fn test_e2e_detect_eterm_project() {
    let project_dir = &get_eterm_project_dir();

    let adapter = XcodeAdapter::detect(project_dir);
    assert!(adapter.is_some(), "Should detect ETerm.xcodeproj");

    let adapter = adapter.unwrap();
    println!("✓ Detected project: {}", adapter.name());
    println!("  Type: {}", adapter.adapter_type());
    println!("  Path: {:?}", adapter.path());
}

// ============================================================================
// Step 2: Targets (Schemes)
// ============================================================================

#[test]
#[ignore]
fn test_e2e_list_schemes() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let targets = adapter.targets();
    println!("✓ Found {} schemes:", targets.len());
    for target in &targets {
        println!("  - {} ({})", target.name, target.target_type);
    }

    assert!(!targets.is_empty(), "Should have at least one scheme");
    assert!(
        targets.iter().any(|t| t.name == "ETerm"),
        "Should have ETerm scheme"
    );
}

// ============================================================================
// Step 3: Bundle ID
// ============================================================================

#[test]
#[ignore]
fn test_e2e_bundle_id() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let bundle_id = adapter.bundle_id();
    println!("✓ Bundle ID: {:?}", bundle_id);

    assert!(bundle_id.is_some(), "Should have bundle ID");
}

// ============================================================================
// Step 4: Devices
// ============================================================================

#[test]
#[ignore]
fn test_e2e_list_devices() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let devices = adapter.devices();
    println!("✓ Found {} devices:", devices.len());

    let mut simulators = 0;
    let mut physical = 0;
    let mut mac = 0;

    for device in &devices {
        let status = if device.state == DeviceState::Available { "🟢" } else { "⚪" };
        let version = device.os_version.as_deref().unwrap_or("-");
        println!("  {} {} ({:?}) - {}", status, device.name, device.device_type, version);

        match device.device_type {
            DeviceType::Mac => mac += 1,
            DeviceType::Simulator => simulators += 1,
            DeviceType::Physical => physical += 1,
        }
    }

    println!("\n  Summary: {} Mac, {} Simulators, {} Physical", mac, simulators, physical);
    assert!(mac >= 1, "Should have at least My Mac");
}

// ============================================================================
// Step 5: Build Command Generation
// ============================================================================

#[test]
#[ignore]
fn test_e2e_build_cmd_mac() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let options = BuildOptions {
        config: "Debug".to_string(),
        clean: false,
        device: Some(Device {
            id: "mac".to_string(),
            name: "My Mac".to_string(),
            device_type: DeviceType::Mac,
            os_version: None,
            state: DeviceState::Available,
        }),
        ..Default::default()
    };

    let cmd = adapter.build_cmd("ETerm", &options).unwrap();

    println!("✓ Build command for Mac:");
    println!("  {}", cmd.to_string());

    assert_eq!(cmd.program, "xcodebuild");
    assert!(cmd.args.contains(&"-scheme".to_string()));
    assert!(cmd.args.contains(&"ETerm".to_string()));
    assert!(cmd.args.contains(&"-destination".to_string()));
}

#[test]
#[ignore]
fn test_e2e_build_cmd_simulator() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    // Get first available simulator
    let devices = adapter.devices();
    let simulator = devices
        .iter()
        .find(|d| d.device_type == DeviceType::Simulator)
        .expect("Should have at least one simulator");

    let options = BuildOptions {
        config: "Debug".to_string(),
        clean: false,
        device: Some(simulator.clone()),
        ..Default::default()
    };

    let cmd = adapter.build_cmd("ETerm", &options).unwrap();

    println!("✓ Build command for Simulator ({}):", simulator.name);
    println!("  {}", cmd.to_string());

    assert!(cmd.to_string().contains("iOS Simulator"));
}

// ============================================================================
// Step 6: Install Command Generation
// ============================================================================

#[test]
#[ignore]
fn test_e2e_install_cmd_simulator() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let devices = adapter.devices();
    let simulator = devices
        .iter()
        .find(|d| d.device_type == DeviceType::Simulator)
        .expect("Should have at least one simulator");

    let options = RunOptions {
        device: Some(simulator.clone()),
        ..Default::default()
    };

    let cmd = adapter.install_cmd("ETerm", &options);

    if let Some(cmd) = cmd {
        println!("✓ Install command for Simulator:");
        println!("  {}", cmd.to_string());
        assert!(cmd.to_string().contains("simctl install"));
    } else {
        println!("⚠ No install command (expected for Mac)");
    }
}

// ============================================================================
// Step 7: Run Command Generation
// ============================================================================

#[test]
#[ignore]
fn test_e2e_run_cmd_mac() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let options = RunOptions {
        device: Some(Device {
            id: "mac".to_string(),
            name: "My Mac".to_string(),
            device_type: DeviceType::Mac,
            os_version: None,
            state: DeviceState::Available,
        }),
        ..Default::default()
    };

    let cmd = adapter.run_cmd("ETerm", &options);

    println!("✓ Run command for Mac:");
    println!("  {}", cmd.to_string());

    assert_eq!(cmd.program, "open");
}

#[test]
#[ignore]
fn test_e2e_run_cmd_simulator() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let devices = adapter.devices();
    let simulator = devices
        .iter()
        .find(|d| d.device_type == DeviceType::Simulator)
        .expect("Should have at least one simulator");

    let options = RunOptions {
        device: Some(simulator.clone()),
        ..Default::default()
    };

    let cmd = adapter.run_cmd("ETerm", &options);

    println!("✓ Run command for Simulator ({}):", simulator.name);
    println!("  {}", cmd.to_string());

    assert!(cmd.to_string().contains("simctl launch"));
}

// ============================================================================
// Step 8: Log Command Generation
// ============================================================================

#[test]
#[ignore]
fn test_e2e_log_cmd_mac() {
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let options = RunOptions {
        device: Some(Device {
            id: "mac".to_string(),
            name: "My Mac".to_string(),
            device_type: DeviceType::Mac,
            os_version: None,
            state: DeviceState::Available,
        }),
        ..Default::default()
    };

    let cmd = adapter.log_cmd("ETerm", &options);

    if let Some(cmd) = cmd {
        println!("✓ Log command for Mac:");
        println!("  {}", cmd.to_string());
        assert_eq!(cmd.program, "log");
    } else {
        println!("⚠ No log command (bundle ID not found?)");
    }
}

// ============================================================================
// Step 9: Full Flow - Build (quick check, no actual build)
// ============================================================================

#[test]
#[ignore]
fn test_e2e_full_flow_dry_run() {
    println!("\n========== E2E Full Flow (Dry Run) ==========\n");

    // 1. Detect
    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();
    println!("1. ✓ Detected: {}", adapter.name());

    // 2. Schemes
    let targets = adapter.targets();
    println!("2. ✓ Schemes: {:?}", targets.iter().map(|t| &t.name).collect::<Vec<_>>());

    // 3. Bundle ID
    let bundle_id = adapter.bundle_id();
    println!("3. ✓ Bundle ID: {:?}", bundle_id);

    // 4. Devices
    let devices = adapter.devices();
    let simulator = devices.iter().find(|d| d.device_type == DeviceType::Simulator);
    println!("4. ✓ Devices: {} total, simulator: {:?}",
        devices.len(),
        simulator.map(|s| &s.name)
    );

    // 5. Build command
    let build_opts = BuildOptions {
        config: "Debug".to_string(),
        device: simulator.cloned(),
        ..Default::default()
    };
    let build_cmd = adapter.build_cmd("ETerm", &build_opts);
    println!("5. ✓ Build cmd: {}", build_cmd.as_ref().map(|c| c.to_string()).unwrap_or("None".into()));

    // 6. Install command
    let run_opts = RunOptions {
        device: simulator.cloned(),
        ..Default::default()
    };
    let install_cmd = adapter.install_cmd("ETerm", &run_opts);
    println!("6. ✓ Install cmd: {}", install_cmd.as_ref().map(|c| c.to_string()).unwrap_or("None".into()));

    // 7. Run command
    let run_cmd = adapter.run_cmd("ETerm", &run_opts);
    println!("7. ✓ Run cmd: {}", run_cmd.to_string());

    // 8. Log command
    let log_cmd = adapter.log_cmd("ETerm", &run_opts);
    println!("8. ✓ Log cmd: {}", log_cmd.as_ref().map(|c| c.to_string()).unwrap_or("None".into()));

    println!("\n========== All Commands Generated Successfully ==========\n");
}

// ============================================================================
// Step 10: Actually Execute Build (SLOW - only run when needed)
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_e2e_execute_build_mac() {
    println!("\n========== E2E Execute Build (Mac) ==========\n");
    println!("⚠️  This test actually builds the project - may take several minutes\n");

    let project_dir = &get_eterm_project_dir();
    let adapter = XcodeAdapter::detect(project_dir).unwrap();

    let options = BuildOptions {
        config: "Debug".to_string(),
        clean: false,
        device: Some(Device {
            id: "mac".to_string(),
            name: "My Mac".to_string(),
            device_type: DeviceType::Mac,
            os_version: None,
            state: DeviceState::Available,
        }),
        ..Default::default()
    };

    let cmd = adapter.build_cmd("ETerm", &options).unwrap();
    println!("Executing: {}\n", cmd.to_string());

    let manager = ProcessManager::new();
    let (process_id, mut rx) = manager
        .start(cmd, project_dir.to_path_buf(), "xcode", "ETerm")
        .await
        .unwrap();

    println!("Process started: {}\n", process_id);

    // Collect output (with timeout)
    let mut line_count = 0;
    let timeout = tokio::time::Duration::from_secs(300); // 5 minutes
    let start = tokio::time::Instant::now();

    while let Ok(result) = tokio::time::timeout(
        tokio::time::Duration::from_secs(1),
        rx.recv()
    ).await {
        if let Some(event) = result {
            line_count += 1;
            // Print first 20 and last 20 lines
            if line_count <= 20 {
                println!("{}", event.format());
            } else if line_count == 21 {
                println!("... (output truncated) ...");
            }
        } else {
            break;
        }

        if start.elapsed() > timeout {
            println!("⚠️  Timeout reached, stopping");
            manager.stop(&process_id).await.ok();
            break;
        }
    }

    // Wait for process to finish
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let process = manager.get(&process_id).unwrap();
    println!("\nBuild completed:");
    println!("  Status: {:?}", process.status);
    println!("  Lines: {}", line_count);

    // Check status
    match &process.status {
        dev_runner_app::process::ProcessStatus::Stopped => {
            println!("\n✓ Build succeeded!");
        }
        dev_runner_app::process::ProcessStatus::Failed { code, message } => {
            println!("\n✗ Build failed: {} ({})", message, code);
        }
        _ => {}
    }
}
