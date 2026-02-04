//! MCP tool definitions and dispatcher

use serde_json::{json, Value};

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
            "name": "start",
            "description": "Build and run a project (combined build + install + run). The command runs in a visible terminal within DevRunner UI. Idempotent: if already running, returns current status without restarting.",
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
            "description": "Get the current status of a project: running, stopped, or crashed. Also returns pid, uptime, and exit code if applicable.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory."
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "logs",
            "description": "Get terminal output logs from a running project. Supports incremental polling via 'since' parameter: pass the 'next_seq' value from the previous response as 'since' to get only new lines. Supports text search filtering.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project directory."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of log lines to return. Defaults to 200.",
                        "default": 200
                    },
                    "since": {
                        "type": "integer",
                        "description": "Sequence number to start from (exclusive). Use 'next_seq' from a previous response for incremental polling. Defaults to 0 (all available logs)."
                    },
                    "search": {
                        "type": "string",
                        "description": "Filter logs to lines containing this text (case-sensitive)."
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

        "start" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            let mut body = json!({ "path": path });
            copy_optional_str(&args, &mut body, "target");
            copy_optional_str(&args, &mut body, "device");
            copy_optional_str(&args, &mut body, "config");
            copy_optional_bool(&args, &mut body, "clean");
            client.post("/api/v1/projects/start", &body)
        }

        "stop" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            let mut body = json!({ "path": path });
            copy_optional_bool(&args, &mut body, "force");
            client.post("/api/v1/projects/stop", &body)
        }

        "status" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);
            client.get("/api/v1/projects/status", &[("path", path.as_str())])
        }

        "logs" => {
            let path = require_string(&args, "path")?;
            let path = expand_tilde(&path);

            let mut query: Vec<(&str, String)> = vec![("path", path)];

            if let Some(limit) = args.get("limit").and_then(|v| v.as_u64()) {
                query.push(("limit", limit.to_string()));
            }
            if let Some(since) = args.get("since").and_then(|v| v.as_u64()) {
                query.push(("since", since.to_string()));
            }
            if let Some(search) = args.get("search").and_then(|v| v.as_str()) {
                query.push(("search", search.to_string()));
            }

            let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            client.get("/api/v1/projects/logs", &query_refs)
        }

        "list_devices" => client.get("/api/v1/devices", &[]),

        "boot_simulator" => {
            let id = require_string(&args, "id")?;
            client.post("/api/v1/devices/boot", &json!({ "id": id }))
        }

        _ => Err(format!("Unknown tool: {}", name)),
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
