//! dev-runner-mcp: MCP Server for DevRunner
//!
//! Stdio proxy that translates MCP tool calls into HTTP API calls
//! to DevRunner.app running on localhost:9274.

mod client;
mod launcher;
mod protocol;
mod tools;

use std::io::{self, BufRead, BufReader, Read, Write};

use serde_json::json;
use tracing::{debug, error, info};

use client::DevRunnerClient;
use protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};

/// Transport protocol, auto-detected from first byte
#[derive(Debug, Clone, Copy, PartialEq)]
enum Protocol {
    /// Line-delimited JSON (one JSON per line, terminated by \n)
    Line,
    /// Content-Length header framing (LSP/MCP standard)
    ContentLength,
}

fn main() {
    // Initialize tracing to stderr (stdout is reserved for MCP protocol)
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("dev-runner-mcp v{} starting", env!("CARGO_PKG_VERSION"));

    let client = DevRunnerClient::new();
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    // Auto-detect protocol from first byte
    let protocol = match detect_protocol(&mut reader) {
        Some(p) => {
            info!("Detected protocol: {:?}", p);
            p
        }
        None => {
            info!("stdin closed before any data, shutting down");
            return;
        }
    };

    loop {
        let message = match protocol {
            Protocol::Line => read_line_message(&mut reader),
            Protocol::ContentLength => read_content_length_message(&mut reader),
        };

        let message = match message {
            Some(msg) => msg,
            None => {
                info!("stdin closed, shutting down");
                break;
            }
        };

        // Parse JSON-RPC request
        let request: JsonRpcRequest = match serde_json::from_str(&message) {
            Ok(req) => req,
            Err(e) => {
                error!("Failed to parse JSON-RPC request: {}", e);
                let resp =
                    JsonRpcResponse::error(None, JsonRpcError::parse_error(&e.to_string()));
                write_message(&mut writer, &resp, protocol);
                continue;
            }
        };

        debug!("← {} (id={:?})", request.method, request.id);

        // Handle notifications (no id) - no response needed
        if request.is_notification() {
            debug!("Notification: {}, ignoring", request.method);
            continue;
        }

        // Dispatch request
        let response = handle_request(&client, &request);

        debug!("→ response for {} (id={:?})", request.method, request.id);
        write_message(&mut writer, &response, protocol);
    }
}

/// Handle a JSON-RPC request and return a response
fn handle_request(client: &DevRunnerClient, request: &JsonRpcRequest) -> JsonRpcResponse {
    let id = request.id.clone();

    match request.method.as_str() {
        "initialize" => JsonRpcResponse::success(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "dev-runner-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),

        "tools/list" => JsonRpcResponse::success(
            id,
            json!({
                "tools": tools::get_tools()
            }),
        ),

        "tools/call" => {
            let params = match &request.params {
                Some(p) => p,
                None => {
                    return JsonRpcResponse::error(
                        id,
                        JsonRpcError::invalid_params("missing params"),
                    );
                }
            };

            let name = match params.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => {
                    return JsonRpcResponse::error(
                        id,
                        JsonRpcError::invalid_params("missing tool name"),
                    );
                }
            };

            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            debug!("Calling tool: {} with args: {}", name, args);

            match tools::call_tool(client, name, args) {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                    JsonRpcResponse::success(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": text }]
                        }),
                    )
                }
                Err(e) => JsonRpcResponse::success(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": e }],
                        "isError": true
                    }),
                ),
            }
        }

        _ => JsonRpcResponse::error(id, JsonRpcError::method_not_found(&request.method)),
    }
}

/// Auto-detect protocol by peeking at the first byte.
/// '{' → line-delimited JSON, 'C' → Content-Length framing.
fn detect_protocol(reader: &mut BufReader<impl Read>) -> Option<Protocol> {
    let buf = match reader.fill_buf() {
        Ok(buf) if buf.is_empty() => return None,
        Ok(buf) => buf,
        Err(e) => {
            error!("Failed to peek stdin: {}", e);
            return None;
        }
    };

    match buf[0] {
        b'{' => Some(Protocol::Line),
        _ => Some(Protocol::ContentLength),
    }
}

/// Read a line-delimited JSON message (one JSON object per line).
fn read_line_message(reader: &mut impl BufRead) -> Option<String> {
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return None, // EOF
            Ok(_) => {}
            Err(e) => {
                error!("Failed to read from stdin: {}", e);
                return None;
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue; // skip blank lines
        }

        return Some(trimmed.to_string());
    }
}

/// Read a Content-Length framed message (LSP/MCP standard).
fn read_content_length_message(reader: &mut impl BufRead) -> Option<String> {
    let mut content_length: usize = 0;

    // Read headers
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return None, // EOF
            Ok(_) => {}
            Err(e) => {
                error!("Failed to read from stdin: {}", e);
                return None;
            }
        }

        let trimmed = line.trim();

        // Empty line = end of headers
        if trimmed.is_empty() {
            break;
        }

        // Parse Content-Length header
        if let Some(len_str) = trimmed.strip_prefix("Content-Length:") {
            if let Ok(len) = len_str.trim().parse::<usize>() {
                content_length = len;
            }
        }
    }

    if content_length == 0 {
        error!("Missing or zero Content-Length");
        return None;
    }

    // Read body
    let mut buf = vec![0u8; content_length];
    match reader.read_exact(&mut buf) {
        Ok(_) => {}
        Err(e) => {
            error!("Failed to read message body: {}", e);
            return None;
        }
    }

    match String::from_utf8(buf) {
        Ok(s) => Some(s),
        Err(e) => {
            error!("Invalid UTF-8 in message body: {}", e);
            None
        }
    }
}

/// Write a JSON-RPC response using the detected protocol.
fn write_message(writer: &mut impl Write, response: &JsonRpcResponse, protocol: Protocol) {
    let json = match serde_json::to_string(response) {
        Ok(j) => j,
        Err(e) => {
            error!("Failed to serialize response: {}", e);
            return;
        }
    };

    let result = match protocol {
        Protocol::Line => {
            writeln!(writer, "{}", json)
        }
        Protocol::ContentLength => {
            write!(writer, "Content-Length: {}\r\n\r\n{}", json.len(), json)
        }
    };

    if let Err(e) = result {
        error!("Failed to write response: {}", e);
        return;
    }
    if let Err(e) = writer.flush() {
        error!("Failed to flush: {}", e);
    }
}
