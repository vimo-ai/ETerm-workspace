//! HTTP client for DevRunner Control API

use std::time::Duration;

use serde_json::Value;
use tracing::debug;

use crate::launcher;

const DEFAULT_BASE_URL: &str = "http://localhost:9274";

pub struct DevRunnerClient {
    base_url: String,
    agent: ureq::Agent,
}

impl DevRunnerClient {
    pub fn new() -> Self {
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_connect(Some(Duration::from_secs(3)))
                .timeout_global(Some(Duration::from_secs(30)))
                .http_status_as_error(false)
                .build(),
        );
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            agent,
        }
    }

    /// Ensure DevRunner is running before making a request
    fn ensure_running(&self) -> Result<(), String> {
        launcher::ensure_running(&self.base_url)
    }

    /// GET request, returns response body as Value
    pub fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        self.ensure_running()?;

        let mut url = format!("{}{}", self.base_url, path);
        if !query.is_empty() {
            url.push('?');
            let pairs: Vec<String> = query
                .iter()
                .map(|(k, v)| format!("{}={}", k, url_encode(v)))
                .collect();
            url.push_str(&pairs.join("&"));
        }

        debug!("GET {}", url);

        let resp = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| classify_error(e))?;

        read_json_response(resp)
    }

    /// POST request with JSON body
    pub fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.ensure_running()?;

        let url = format!("{}{}", self.base_url, path);
        debug!("POST {} body={}", url, body);

        let resp = self
            .agent
            .post(&url)
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| classify_error(e))?;

        read_json_response(resp)
    }

    /// DELETE request with query parameters
    pub fn delete(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        self.ensure_running()?;

        let mut url = format!("{}{}", self.base_url, path);
        if !query.is_empty() {
            url.push('?');
            let pairs: Vec<String> = query
                .iter()
                .map(|(k, v)| format!("{}={}", k, url_encode(v)))
                .collect();
            url.push_str(&pairs.join("&"));
        }

        debug!("DELETE {}", url);

        let resp = self
            .agent
            .delete(&url)
            .call()
            .map_err(|e| classify_error(e))?;

        read_json_response(resp)
    }
}

/// Expand ~ to home directory
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}/{}", home.display(), rest);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.display().to_string();
        }
    }
    path.to_string()
}

/// Read JSON from response body, handling HTTP error status codes
fn read_json_response(resp: ureq::http::Response<ureq::Body>) -> Result<Value, String> {
    let status = resp.status();

    let body: Value = resp
        .into_body()
        .read_json()
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if status.is_client_error() || status.is_server_error() {
        // Extract error message from {"error": "message"} if present
        if let Some(msg) = body.get("error").and_then(|e| e.as_str()) {
            return Err(msg.to_string());
        }
        return Err(format!("HTTP {}: {}", status.as_u16(), body));
    }

    Ok(body)
}

/// Classify ureq errors into user-friendly messages
fn classify_error(err: ureq::Error) -> String {
    match &err {
        ureq::Error::Timeout(_) => "Request timed out. DevRunner may be busy.".to_string(),
        _ => {
            let msg = err.to_string();
            if msg.contains("Connection refused") || msg.contains("connection refused") {
                "DevRunner is not running or not accepting connections.".to_string()
            } else {
                format!("HTTP error: {}", msg)
            }
        }
    }
}

/// Simple URL encoding for query parameter values
fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(c),
            ' ' => result.push_str("%20"),
            '/' => result.push_str("%2F"),
            _ => {
                for byte in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    result
}
