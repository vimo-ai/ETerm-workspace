//! MCP tool definitions and dispatcher

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tracing::debug;

use crate::client::{expand_tilde, DevRunnerClient};

/// Return all MCP tool definitions
pub fn get_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "health",
            "description": "Check if DevRunner is running and responsive. Returns status, version, and uptime.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "list_projects",
            "description": "List all registered development projects with their current status (running/stopped/crashed), project type (xcode/node), name, path, and runtime info (pid, uptime).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "add_project",
            "description": "Register a project directory with DevRunner. The project type (Xcode, Node.js, etc.) is auto-detected from the directory contents. Must be called before using start/stop/logs on a project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory (e.g. /Users/me/MyApp). Supports ~ for home directory."
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "remove_project",
            "description": "Unregister a project directory from DevRunner. This does not delete any files, only removes it from the project list.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory to remove."
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "build",
            "description": "Build (compile) a project. Blocks until build completes. For iOS simulator targets, the app is automatically installed on the simulator after build. Does NOT launch the app.\n\nReturns: {success, duration_secs} on success, {success: false, error, duration_secs} on failure.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory."
                    },
                    "target": {
                        "type": "string",
                        "description": "Build target/scheme name. Optional, defaults to the first detected target."
                    },
                    "device": {
                        "type": "string",
                        "description": "Device name to build for (e.g. 'iPhone 16 Pro'). For Xcode projects only."
                    },
                    "config": {
                        "type": "string",
                        "description": "Build configuration. Defaults to 'Debug'.",
                        "default": "Debug"
                    },
                    "clean": {
                        "type": "boolean",
                        "description": "Whether to perform a clean build. Defaults to false.",
                        "default": false
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "run",
            "description": "Launch an already-built app on a device. Blocks until the app is running or launch fails. The app must be built first (use 'build' or 'start').\n\nReturns: {success, pid, duration_secs} on success, {success: false, error} on failure. Returns immediately with already_running: true if already running.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory."
                    },
                    "target": {
                        "type": "string",
                        "description": "Target/scheme name. Optional, defaults to the first detected target."
                    },
                    "device": {
                        "type": "string",
                        "description": "Device name to run on (e.g. 'iPhone 16 Pro'). For Xcode projects only."
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "start",
            "description": "Build, install, and launch a project — the primary tool for the 'code change → rebuild → test' workflow. Blocks until the entire chain completes. Equivalent to build + install + run in one call.\n\nReturns: {success, duration_secs} on success, {success: false, error} on failure with error logs included. If already running, automatically stops and restarts. Idempotent: safe to call repeatedly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory."
                    },
                    "target": {
                        "type": "string",
                        "description": "Build target/scheme name. Optional, defaults to the first detected target."
                    },
                    "device": {
                        "type": "string",
                        "description": "Device name to run on (e.g. 'iPhone 16 Pro'). For Xcode projects only."
                    },
                    "config": {
                        "type": "string",
                        "description": "Build configuration. Defaults to 'Debug'.",
                        "default": "Debug"
                    },
                    "clean": {
                        "type": "boolean",
                        "description": "Whether to perform a clean build. Defaults to false.",
                        "default": false
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "stop",
            "description": "Stop a running project. Sends SIGINT (or SIGKILL with force=true) to the running process.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["build", "run"],
                        "description": "Which task to stop: 'build' or 'run'. If omitted, stops the most active task (running > starting > any)."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Force kill with SIGKILL instead of SIGINT. Defaults to false.",
                        "default": false
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "status",
            "description": "Get the current status of a project: running, stopped, or crashed. Also returns action ('build'/'run'), pid, uptime, and exit code. Without 'action' filter, returns the most active task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["build", "run"],
                        "description": "Filter to a specific task type: 'build' or 'run'. If omitted, returns the most active task."
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "logs",
            "description": "Get terminal output logs from a running project. Supports incremental polling via 'since' parameter: pass the 'next_seq' value from the previous response as 'since' to get only new lines. Supports text search and regex filtering.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["build", "run"],
                        "description": "Which task's logs to read: 'build' or 'run'. If omitted, reads from the most active task."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of log lines to return. Defaults to 50.",
                        "default": 50
                    },
                    "since": {
                        "type": "integer",
                        "description": "Sequence number to start from (exclusive). Use 'next_seq' from a previous response for incremental polling. Defaults to 0 (all available logs)."
                    },
                    "search": {
                        "type": "string",
                        "description": "Filter logs to lines matching this text or regex pattern."
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "If true, treat 'search' as a regular expression pattern. Defaults to false (plain text search).",
                        "default": false
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "If true, search is case-insensitive. Defaults to true for backward compatibility.",
                        "default": true
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Maximum characters in the response. Lines are trimmed from the beginning to fit. Set to 0 for unlimited. Defaults to 4000.",
                        "default": 4000
                    },
                    "verbose": {
                        "type": "boolean",
                        "description": "If true, return full output without character limit. Overrides max_chars. Defaults to false.",
                        "default": false
                    },
                    "anchor": {
                        "type": "string",
                        "enum": ["tail", "head"],
                        "description": "Which end to keep when truncating: 'tail' keeps most recent lines (default), 'head' keeps earliest lines (useful for build start or initial errors).",
                        "default": "tail"
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "list_devices",
            "description": "List available iOS simulators and physical devices. Returns device id, name, type, OS version, and state (booted/shutdown).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "boot_simulator",
            "description": "Boot an iOS simulator by its device ID (UDID). Get the device ID from list_devices first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Device UDID from list_devices."
                    }
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "wait_ready",
            "description": "Wait for a service to become ready by polling a health check URL until it returns HTTP 200 or timeout is reached. Useful after starting a project to confirm the service is actually accepting requests, not just that the process has started.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory. Used to identify which project this readiness check is for."
                    },
                    "url": {
                        "type": "string",
                        "description": "Health check URL to poll (e.g. http://localhost:3000/health)."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Maximum seconds to wait before giving up. Defaults to 30.",
                        "default": 30
                    },
                    "interval": {
                        "type": "integer",
                        "description": "Polling interval in milliseconds. Defaults to 1000.",
                        "default": 1000
                    }
                },
                "required": ["path", "url"]
            }
        }),
    ]
}

/// Dispatch a tool call to the appropriate HTTP API endpoint
pub fn call_tool(client: &DevRunnerClient, name: &str, args: Value) -> Result<Value, String> {
    match name {
        "health" => client.get("/health", &[]),

        "list_projects" => client.get("/api/v1/projects", &[]),

        "add_project" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            client.post("/api/v1/projects", &json!({ "path": path }))
        }

        "remove_project" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            client.delete("/api/v1/projects", &[("path", path.as_str())])
        }

        "build" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            let mut body = json!({ "path": path });
            copy_optional_str(&args, &mut body, "target");
            copy_optional_str(&args, &mut body, "device");
            copy_optional_str(&args, &mut body, "config");
            copy_optional_bool(&args, &mut body, "clean");

            // Trigger build
            client.post("/api/v1/projects/build", &body)?;

            // Poll until completion (running means build is still in progress)
            let (status, elapsed) = poll_task_status(client, &path, Some("build"), false)?;

            // Format and return result
            format_task_result(client, &path, &status, elapsed, Some("build"))
        }

        "run" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            let mut body = json!({ "path": path });
            copy_optional_str(&args, &mut body, "target");
            copy_optional_str(&args, &mut body, "device");

            // Trigger run
            let response = client.post("/api/v1/projects/run", &body)?;

            // Check if already running
            if let Some(true) = response.get("already_running").and_then(|v| v.as_bool()) {
                let mut result = json!({
                    "success": true,
                    "already_running": true,
                });
                if let Some(pid) = response.get("pid") {
                    if !pid.is_null() {
                        result["pid"] = pid.clone();
                    }
                }
                return Ok(result);
            }

            // Poll until running (running means app launched successfully)
            let (status, elapsed) = poll_task_status(client, &path, Some("run"), true)?;

            // Format and return result
            format_task_result(client, &path, &status, elapsed, Some("run"))
        }

        "start" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            let mut body = json!({ "path": path });
            copy_optional_str(&args, &mut body, "target");
            copy_optional_str(&args, &mut body, "device");
            copy_optional_str(&args, &mut body, "config");
            copy_optional_bool(&args, &mut body, "clean");

            // Trigger start
            let response = client.post("/api/v1/projects/start", &body)?;

            // If already running, stop first then restart
            if let Some(true) = response.get("already_running").and_then(|v| v.as_bool()) {
                client.post("/api/v1/projects/stop", &json!({ "path": path }))?;
                std::thread::sleep(std::time::Duration::from_millis(500));
                let _retry = client.post("/api/v1/projects/start", &body)?;
            }

            // Poll with stability window: "running" for 15s+ = build succeeded, app is up.
            // Catches fast build failures (syntax errors, missing deps) within the window.
            let (status, elapsed) = poll_start_status(client, &path, 15)?;

            // Format and return result
            format_task_result(client, &path, &status, elapsed, Some("run"))
        }

        "stop" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            let mut body = json!({ "path": path });
            copy_optional_str(&args, &mut body, "action");
            copy_optional_bool(&args, &mut body, "force");
            client.post("/api/v1/projects/stop", &body)
        }

        "status" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            let mut query: Vec<(&str, String)> = vec![("path", path)];
            if let Some(action) = args.get("action").and_then(|v| v.as_str()) {
                query.push(("action", action.to_string()));
            }
            let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            client.get("/api/v1/projects/status", &query_refs)
        }

        "logs" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);

            let mut query: Vec<(&str, String)> = vec![("path", path)];

            if let Some(action) = args.get("action").and_then(|v| v.as_str()) {
                query.push(("action", action.to_string()));
            }

            // Default limit is 50 for MCP (lower than HTTP API default)
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            query.push(("limit", limit.to_string()));

            if let Some(since) = args.get("since").and_then(|v| v.as_u64()) {
                query.push(("since", since.to_string()));
            }
            if let Some(search) = args.get("search").and_then(|v| v.as_str()) {
                query.push(("search", search.to_string()));
            }
            if let Some(regex) = args.get("regex").and_then(|v| v.as_bool()) {
                if regex {
                    query.push(("regex", "true".to_string()));
                }
            }
            if let Some(ci) = args.get("case_insensitive").and_then(|v| v.as_bool()) {
                if !ci {
                    query.push(("case_insensitive", "false".to_string()));
                }
            }

            let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let result = client.get("/api/v1/projects/logs", &query_refs)?;

            // Apply max_chars truncation
            let verbose = args.get("verbose").and_then(|v| v.as_bool()).unwrap_or(false);
            let max_chars = if verbose {
                0 // unlimited
            } else {
                args.get("max_chars").and_then(|v| v.as_u64()).unwrap_or(4000) as usize
            };

            let anchor = args.get("anchor").and_then(|v| v.as_str()).unwrap_or("tail");

            Ok(truncate_log_response(result, max_chars, anchor))
        }

        "list_devices" => client.get("/api/v1/devices", &[]),

        "boot_simulator" => {
            let id = require_string(&args, "id")?;
            client.post("/api/v1/devices/boot", &json!({ "id": id }))
        }

        "wait_ready" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            let url = require_string(&args, "url")?;
            let timeout_secs = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(30);
            let interval_ms = args.get("interval").and_then(|v| v.as_u64()).unwrap_or(1000);
            let interval_ms = interval_ms.max(50); // prevent hot spin

            // Validate URL format upfront
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(format!("invalid URL '{}': must start with http:// or https://", url));
            }

            poll_until_ready(&path, &url, timeout_secs, interval_ms)
        }

        _ => Err(format!("Unknown tool: {}", name)),
    }
}

/// Poll a URL until HTTP 200 or timeout. Runs entirely in the MCP process.
fn poll_until_ready(path: &str, url: &str, timeout_secs: u64, interval_ms: u64) -> Result<Value, String> {
    let deadline = Duration::from_secs(timeout_secs);
    let interval = Duration::from_millis(interval_ms);
    let start = Instant::now();

    let mut last_status: u16 = 0;
    let mut last_error: Option<String> = None;

    loop {
        let elapsed = start.elapsed();
        if elapsed >= deadline {
            let mut result = json!({
                "ready": false,
                "path": path,
                "elapsed_ms": elapsed.as_millis() as u64,
                "status_code": last_status,
                "error": "timeout"
            });
            if let Some(err) = &last_error {
                result["last_error"] = json!(err);
            }
            return Ok(result);
        }

        // Per-request timeout bounded by remaining deadline
        let remaining = deadline.saturating_sub(elapsed);
        let req_timeout = remaining.min(Duration::from_secs(5));

        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_connect(Some(req_timeout.min(Duration::from_secs(3))))
                .timeout_global(Some(req_timeout))
                .http_status_as_error(false)
                .build(),
        );

        debug!("wait_ready: polling {}", url);

        match agent.get(url).call() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                last_status = status;
                last_error = None;
                if status == 200 {
                    return Ok(json!({
                        "ready": true,
                        "path": path,
                        "elapsed_ms": start.elapsed().as_millis() as u64,
                        "status_code": status
                    }));
                }
                debug!("wait_ready: got HTTP {}, retrying", status);
            }
            Err(e) => {
                let msg = e.to_string();
                debug!("wait_ready: connection error: {}, retrying", msg);
                last_error = Some(msg);
            }
        }

        let elapsed = start.elapsed();
        sleep_until(elapsed, deadline, interval);
    }
}

/// Sleep for the polling interval, but not past the deadline.
fn sleep_until(elapsed: Duration, deadline: Duration, interval: Duration) {
    let remaining = deadline.saturating_sub(elapsed);
    if remaining.is_zero() {
        return;
    }
    std::thread::sleep(interval.min(remaining));
}

/// Truncate log response to fit within character budget.
/// `anchor` controls which end to keep: "tail" (default) or "head".
fn truncate_log_response(mut result: Value, max_chars: usize, anchor: &str) -> Value {
    if max_chars == 0 {
        return result;
    }

    let lines = match result.get_mut("lines").and_then(|v| v.as_array_mut()) {
        Some(lines) => lines,
        None => return result,
    };

    let total_lines = lines.len();

    // Calculate total character count
    let total_chars: usize = lines.iter()
        .filter_map(|v| v.as_str())
        .map(|s| s.len() + 1) // +1 for newline equivalent
        .sum();

    if total_chars <= max_chars {
        return result;
    }

    let (kept_lines, truncated_count) = if anchor == "head" {
        // Keep lines from the head that fit within budget
        let mut budget = max_chars;
        let mut keep_until = 0;
        for line in lines.iter() {
            let line_cost = line.as_str().map(|s| s.len() + 1).unwrap_or(1);
            if line_cost > budget {
                break;
            }
            budget -= line_cost;
            keep_until += 1;
        }
        let kept: Vec<Value> = lines.drain(..keep_until).collect();
        let truncated = total_lines - keep_until;
        (kept, truncated)
    } else {
        // Keep lines from the tail that fit within budget
        let mut budget = max_chars;
        let mut keep_from = lines.len();
        for (i, line) in lines.iter().enumerate().rev() {
            let line_cost = line.as_str().map(|s| s.len() + 1).unwrap_or(1);
            if line_cost > budget {
                break;
            }
            budget -= line_cost;
            keep_from = i;
        }
        let truncated = keep_from;
        let kept: Vec<Value> = lines.drain(keep_from..).collect();
        (kept, truncated)
    };

    *lines = kept_lines;

    // Add truncation metadata
    result["truncated"] = json!(true);
    result["truncated_lines"] = json!(truncated_count);
    result["total_lines"] = json!(total_lines);
    result["anchor"] = json!(anchor);
    result["hint"] = json!(format!(
        "Output truncated (showing {} from {}). Use 'verbose: true' for full output, 'anchor: \"{}\"' to see the other end, or 'search' to filter.",
        anchor,
        if anchor == "tail" { "head" } else { "tail" },
        if anchor == "tail" { "head" } else { "tail" }
    ));

    result
}

// -- Synchronous Task Polling Helpers --

/// Poll `/api/v1/projects/status` until a terminal state is reached.
/// Terminal states: "success", "failed", and "running" if `running_is_done` is true.
/// Returns the final status JSON and elapsed duration.
fn poll_task_status(
    client: &DevRunnerClient,
    path: &str,
    action: Option<&str>,
    running_is_done: bool,
) -> Result<(Value, Duration), String> {
    let start = Instant::now();
    let timeout = Duration::from_secs(30 * 60); // 30 minute safety cap

    loop {
        std::thread::sleep(Duration::from_secs(2));

        let elapsed = start.elapsed();
        if elapsed >= timeout {
            return Err(format!("timeout after {:.1}s", elapsed.as_secs_f64()));
        }

        let mut query: Vec<(&str, String)> = vec![("path", path.to_string())];
        if let Some(action_str) = action {
            query.push(("action", action_str.to_string()));
        }
        let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let status = client.get("/api/v1/projects/status", &query_refs)?;
        let state = status.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");

        debug!("poll_task_status: path={}, action={:?}, state={}, elapsed={:.1}s",
               path, action, state, elapsed.as_secs_f64());

        match state {
            "success" | "failed" => return Ok((status, elapsed)),
            "running" if running_is_done => return Ok((status, elapsed)),
            "not_found" => return Err(format!("project not found: {}", path)),
            "starting" | "sent" | "idle" | "running" => continue,
            _ => continue,
        }
    }
}

/// Poll for `start` command: the chain (build && install && launch --console)
/// keeps running indefinitely on success, so "running" IS the success state.
/// Uses a stability window: after first seeing "running", keep polling for
/// `stability_secs` more seconds to catch fast build failures before returning.
fn poll_start_status(
    client: &DevRunnerClient,
    path: &str,
    stability_secs: u64,
) -> Result<(Value, Duration), String> {
    let start = Instant::now();
    let timeout = Duration::from_secs(30 * 60);
    let stability = Duration::from_secs(stability_secs);
    let mut running_since: Option<Instant> = None;

    loop {
        std::thread::sleep(Duration::from_secs(2));

        let elapsed = start.elapsed();
        if elapsed >= timeout {
            return Err(format!("timeout after {:.1}s", elapsed.as_secs_f64()));
        }

        let query: Vec<(&str, String)> = vec![
            ("path", path.to_string()),
            ("action", "run".to_string()),
        ];
        let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let status = client.get("/api/v1/projects/status", &query_refs)?;
        let state = status.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");

        debug!("poll_start_status: state={}, running_for={:?}, elapsed={:.1}s",
               state,
               running_since.map(|t| t.elapsed()),
               elapsed.as_secs_f64());

        match state {
            "success" | "failed" => return Ok((status, elapsed)),
            "not_found" => return Err(format!("project not found: {}", path)),
            "running" => {
                let first = running_since.get_or_insert(Instant::now());
                if first.elapsed() >= stability {
                    // Stable running → build succeeded, app is up
                    return Ok((status, elapsed));
                }
                // Still in stability window, keep polling for failures
                continue;
            }
            _ => {
                running_since = None; // Reset on non-running states
                continue;
            }
        }
    }
}

/// Fetch the last 50 lines of logs for error context.
fn fetch_error_logs(
    client: &DevRunnerClient,
    path: &str,
    action: Option<&str>,
) -> String {
    let mut query: Vec<(&str, String)> = vec![
        ("path", path.to_string()),
        ("limit", "50".to_string()),
    ];
    if let Some(action_str) = action {
        query.push(("action", action_str.to_string()));
    }
    let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();

    match client.get("/api/v1/projects/logs", &query_refs) {
        Ok(logs) => {
            if let Some(lines) = logs.get("lines").and_then(|v| v.as_array()) {
                let texts: Vec<String> = lines
                    .iter()
                    .filter_map(|line| line.get("text").and_then(|t| t.as_str()))
                    .map(|s| s.to_string())
                    .collect();
                if texts.is_empty() {
                    "No log output available".to_string()
                } else {
                    texts.join("\n")
                }
            } else {
                "No log output available".to_string()
            }
        }
        Err(e) => format!("Failed to fetch logs: {}", e),
    }
}

/// Format the final success/failure response.
fn format_task_result(
    client: &DevRunnerClient,
    path: &str,
    status: &Value,
    elapsed: Duration,
    log_action: Option<&str>,
) -> Result<Value, String> {
    let state = status.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
    let duration_secs = elapsed.as_secs_f64();

    if state == "success" || state == "running" {
        let mut result = json!({
            "success": true,
            "duration_secs": duration_secs,
        });

        // Include pid if present and not null
        if let Some(pid) = status.get("pid") {
            if !pid.is_null() {
                result["pid"] = pid.clone();
            }
        }

        Ok(result)
    } else {
        let error = fetch_error_logs(client, path, log_action);
        let mut result = json!({
            "success": false,
            "error": error,
            "duration_secs": duration_secs,
        });

        // Check both "exitCode" and "exit_code" keys
        if let Some(exit_code) = status.get("exitCode").or_else(|| status.get("exit_code")) {
            if !exit_code.is_null() {
                result["exit_code"] = exit_code.clone();
            }
        }

        Ok(result)
    }
}

// -- Helpers --

fn require_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("'{}' is required", key))
}

fn copy_optional_str(from: &Value, to: &mut Value, key: &str) {
    if let Some(v) = from.get(key) {
        if v.is_string() {
            to[key] = v.clone();
        }
    }
}

fn copy_optional_bool(from: &Value, to: &mut Value, key: &str) {
    if let Some(v) = from.get(key) {
        if v.is_boolean() {
            to[key] = v.clone();
        }
    }
}
