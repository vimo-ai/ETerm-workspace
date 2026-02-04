//! DevRunner.app auto-detect and auto-launch

use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

/// Check if DevRunner is reachable via health endpoint
pub fn health_check(base_url: &str) -> bool {
    let url = format!("{}/health", base_url);
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(2)))
            .build(),
    );
    match agent.get(&url).call() {
        Ok(resp) => resp.status() == 200,
        Err(_) => false,
    }
}

/// Ensure DevRunner.app is running. If not, launch it and wait.
///
/// Returns Ok(()) if DevRunner is reachable, Err with message otherwise.
pub fn ensure_running(base_url: &str) -> Result<(), String> {
    // Already running?
    if health_check(base_url) {
        return Ok(());
    }

    info!("DevRunner not running, launching...");

    // Launch via macOS `open` command
    let status = std::process::Command::new("open")
        .args(["-a", "DevRunner"])
        .status()
        .map_err(|e| format!("Failed to launch DevRunner: {}", e))?;

    if !status.success() {
        return Err("'open -a DevRunner' failed. Is DevRunner.app installed?".to_string());
    }

    // Poll with exponential backoff, max 5 seconds
    let max_wait = Duration::from_secs(5);
    let start = Instant::now();
    let mut interval = Duration::from_millis(200);

    while start.elapsed() < max_wait {
        std::thread::sleep(interval);
        debug!("Checking DevRunner health... ({:.1}s elapsed)", start.elapsed().as_secs_f64());

        if health_check(base_url) {
            info!("DevRunner is ready");
            return Ok(());
        }

        interval = (interval * 2).min(Duration::from_millis(1000));
    }

    warn!("DevRunner did not become ready within {}s", max_wait.as_secs());
    Err(format!(
        "DevRunner launched but not responding after {}s. Check if it started correctly.",
        max_wait.as_secs()
    ))
}
