//! Stdio-to-HTTP proxy for MCP.
//!
//! Bridges newline-delimited JSON-RPC on stdin/stdout to the daemon's
//! Streamable HTTP MCP endpoint at /mcp. Designed to be used as the
//! `command` in Claude Code's MCP server configuration so that Claude
//! can reach the daemon without a long-running SSE connection.
//!
//! T-308 (REQ-P01, REQ-P02, REQ-P03)

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, bail};
use reqwest::Client;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info, warn};

use daemon_core::{daemon_bind_addr, triumvirate_home_dir};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum backoff between reconnect attempts.
const MAX_BACKOFF: Duration = Duration::from_millis(5_000);
/// Initial backoff between reconnect attempts.
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
/// How long to wait for the daemon to become reachable at startup.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the stdio proxy. Reads JSON-RPC lines from stdin, forwards each to
/// the daemon at `http://<addr>/mcp`, and writes responses to stdout.
pub async fn run_proxy() -> anyhow::Result<()> {
    let addr = daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());
    let base_url = format!("http://{addr}/mcp");
    let token = load_bearer_token()?;

    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("failed to build HTTP client")?;

    // REQ-P03: startup connectivity check with retries.
    wait_for_daemon(&client, &base_url, &token).await?;

    info!(url = %base_url, "proxy connected to daemon");

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line_buf = String::new();

    // Backoff state for mid-session reconnect (REQ-P02).
    let mut backoff = INITIAL_BACKOFF;

    loop {
        line_buf.clear();
        let n = reader.read_line(&mut line_buf).await?;
        if n == 0 {
            // EOF on stdin — caller closed the pipe.
            debug!("stdin EOF, proxy shutting down");
            break;
        }

        let trimmed = line_buf.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Validate that the line is valid JSON before sending.
        if serde_json::from_str::<serde_json::Value>(trimmed).is_err() {
            warn!(line = %trimmed, "ignoring non-JSON line on stdin");
            continue;
        }

        match forward_request(&client, &base_url, &token, trimmed).await {
            Ok(response_lines) => {
                // Reset backoff on success.
                backoff = INITIAL_BACKOFF;
                for response_line in &response_lines {
                    stdout.write_all(response_line.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                }
                stdout.flush().await?;
            }
            Err(err) => {
                // REQ-P02: Write JSON-RPC error for the failed in-flight call,
                // then attempt reconnect with bounded backoff for next call.
                error!(%err, "request to daemon failed");

                // Try to extract the request id so the error response correlates.
                let id = extract_request_id(trimmed);
                let error_response = make_jsonrpc_error(id, -32000, &format!("daemon unreachable: {err}"));
                stdout.write_all(error_response.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;

                // Attempt reconnect with exponential backoff.
                if let Err(reconnect_err) = reconnect_with_backoff(&client, &base_url, &token, &mut backoff).await {
                    error!(%reconnect_err, "reconnect failed, exiting proxy");
                    bail!("daemon lost and reconnect failed: {reconnect_err}");
                }
                info!("reconnected to daemon after backoff");
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Token loading
// ---------------------------------------------------------------------------

/// Read the bearer token from `~/.triumvirate/daemon.token`.
fn load_bearer_token() -> anyhow::Result<String> {
    let home = triumvirate_home_dir()?;
    let token_path = home.join("daemon.token");
    read_token_file(&token_path)
}

/// Read and trim a token file.
fn read_token_file(path: &PathBuf) -> anyhow::Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read bearer token at {}", path.display()))?;
    let token = raw.trim().to_string();
    if token.is_empty() {
        bail!("bearer token at {} is empty", path.display());
    }
    Ok(token)
}

// ---------------------------------------------------------------------------
// HTTP forwarding
// ---------------------------------------------------------------------------

/// Forward a single JSON-RPC line to the daemon. Returns one or more response
/// lines depending on whether the daemon replies with `application/json` or
/// `text/event-stream` (SSE).
async fn forward_request(
    client: &Client,
    url: &str,
    token: &str,
    body: &str,
) -> anyhow::Result<Vec<String>> {
    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .bearer_auth(token)
        .body(body.to_string())
        .send()
        .await
        .context("HTTP POST to daemon failed")?;

    let status = resp.status();
    if !status.is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        bail!("daemon returned HTTP {status}: {err_body}");
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if content_type.contains("text/event-stream") {
        parse_sse_response(resp).await
    } else {
        // application/json or anything else: treat as single JSON body.
        let text = resp.text().await.context("failed to read response body")?;
        Ok(vec![text])
    }
}

/// Parse an SSE response stream, extracting JSON-RPC messages from `data:` lines.
async fn parse_sse_response(resp: reqwest::Response) -> anyhow::Result<Vec<String>> {
    let full_body = resp.text().await.context("failed to read SSE body")?;
    let mut messages = Vec::new();

    for line in full_body.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            // Only forward lines that look like JSON objects.
            if data.starts_with('{') {
                messages.push(data.to_string());
            }
        }
    }

    Ok(messages)
}

// ---------------------------------------------------------------------------
// Startup check & reconnect (REQ-P02, REQ-P03)
// ---------------------------------------------------------------------------

/// Wait for the daemon to become reachable. Retries for `STARTUP_TIMEOUT`
/// then exits with a clear error if not reachable.
async fn wait_for_daemon(client: &Client, url: &str, token: &str) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    let mut interval = Duration::from_millis(200);

    loop {
        match probe_daemon(client, url, token).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                if tokio::time::Instant::now() + interval > deadline {
                    let addr = url.trim_start_matches("http://").trim_end_matches("/mcp");
                    bail!(
                        "daemon not reachable at {addr} -- run 'triumvirate daemon' first (last error: {err})"
                    );
                }
                debug!(%err, ?interval, "daemon not yet reachable, retrying");
                tokio::time::sleep(interval).await;
                interval = (interval * 2).min(Duration::from_secs(1));
            }
        }
    }
}

/// Light-weight probe: send a JSON-RPC initialize to check connectivity.
/// We don't care about the response content, only that the daemon is listening.
async fn probe_daemon(client: &Client, url: &str, token: &str) -> anyhow::Result<()> {
    let probe_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "triumvirate-proxy", "version": "0.1.0" }
        }
    });

    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .bearer_auth(token)
        .json(&probe_body)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .context("probe failed")?;

    if resp.status().is_success() || resp.status().as_u16() == 400 {
        // 400 is acceptable — means daemon is listening but rejected the probe
        // (e.g. bad session state). The point is it's alive.
        Ok(())
    } else {
        bail!("probe returned HTTP {}", resp.status())
    }
}

/// Reconnect with bounded exponential backoff after a mid-session failure.
/// Updates `backoff` in place, capping at `MAX_BACKOFF`.
async fn reconnect_with_backoff(
    client: &Client,
    url: &str,
    token: &str,
    backoff: &mut Duration,
) -> anyhow::Result<()> {
    // Try up to 10 iterations (covers well beyond the 5s cap).
    for attempt in 1..=10 {
        warn!(attempt, backoff_ms = backoff.as_millis(), "attempting reconnect");
        tokio::time::sleep(*backoff).await;

        match probe_daemon(client, url, token).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                debug!(attempt, %err, "reconnect probe failed");
                *backoff = (*backoff * 2).min(MAX_BACKOFF);
            }
        }
    }

    bail!("exhausted reconnect attempts (10 tries, max backoff {}ms)", MAX_BACKOFF.as_millis())
}

// ---------------------------------------------------------------------------
// JSON-RPC helpers
// ---------------------------------------------------------------------------

/// Extract the `id` field from a JSON-RPC request string.
fn extract_request_id(json_line: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(json_line)
        .ok()
        .and_then(|v| v.get("id").cloned())
        .unwrap_or(serde_json::Value::Null)
}

/// Build a JSON-RPC error response string.
fn make_jsonrpc_error(id: serde_json::Value, code: i64, message: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Tests (REQ-P04)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_request_id_from_valid_json() {
        let line = r#"{"jsonrpc":"2.0","id":42,"method":"tools/list"}"#;
        let id = extract_request_id(line);
        assert_eq!(id, serde_json::json!(42));
    }

    #[test]
    fn extract_request_id_string_id() {
        let line = r#"{"jsonrpc":"2.0","id":"abc-123","method":"tools/call"}"#;
        let id = extract_request_id(line);
        assert_eq!(id, serde_json::json!("abc-123"));
    }

    #[test]
    fn extract_request_id_missing() {
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let id = extract_request_id(line);
        assert!(id.is_null());
    }

    #[test]
    fn extract_request_id_invalid_json() {
        let id = extract_request_id("not json at all");
        assert!(id.is_null());
    }

    #[test]
    fn make_jsonrpc_error_structure() {
        let err = make_jsonrpc_error(serde_json::json!(7), -32000, "daemon down");
        let parsed: serde_json::Value = serde_json::from_str(&err).expect("valid json");
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 7);
        assert_eq!(parsed["error"]["code"], -32000);
        assert_eq!(parsed["error"]["message"], "daemon down");
    }

    #[test]
    fn make_jsonrpc_error_null_id() {
        let err = make_jsonrpc_error(serde_json::Value::Null, -32603, "internal");
        let parsed: serde_json::Value = serde_json::from_str(&err).expect("valid json");
        assert!(parsed["id"].is_null());
    }

    #[test]
    fn backoff_sequence_respects_cap() {
        let mut b = INITIAL_BACKOFF;
        let mut steps = vec![b];
        for _ in 0..10 {
            b = (b * 2).min(MAX_BACKOFF);
            steps.push(b);
        }
        // First step is 100ms.
        assert_eq!(steps[0], Duration::from_millis(100));
        // Should double each step: 200, 400, 800, 1600, 3200, 5000 (cap).
        assert_eq!(steps[1], Duration::from_millis(200));
        assert_eq!(steps[2], Duration::from_millis(400));
        assert_eq!(steps[3], Duration::from_millis(800));
        assert_eq!(steps[4], Duration::from_millis(1600));
        assert_eq!(steps[5], Duration::from_millis(3200));
        assert_eq!(steps[6], Duration::from_millis(5000));
        // Everything after should stay at cap.
        for step in &steps[6..] {
            assert_eq!(*step, MAX_BACKOFF);
        }
    }

    #[test]
    fn read_token_file_returns_trimmed_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.token");
        std::fs::write(&path, "  secret-token-123  \n").unwrap();
        let token = read_token_file(&path.to_path_buf()).unwrap();
        assert_eq!(token, "secret-token-123");
    }

    #[test]
    fn read_token_file_rejects_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.token");
        std::fs::write(&path, "   \n").unwrap();
        let err = read_token_file(&path.to_path_buf()).unwrap_err();
        assert!(err.to_string().contains("empty"), "expected empty error, got: {err}");
    }

    #[test]
    fn read_token_file_rejects_missing() {
        let path = PathBuf::from("/tmp/nonexistent-proxy-test-token-file");
        let err = read_token_file(&path).unwrap_err();
        assert!(err.to_string().contains("cannot read"), "expected read error, got: {err}");
    }

    #[tokio::test]
    async fn parse_sse_extracts_data_lines() {
        // Simulate an SSE body with multiple data lines.
        let body = [
            "event: message",
            r#"data: {"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#,
            "",
            "event: message",
            r#"data: {"jsonrpc":"2.0","id":2,"result":{}}"#,
            "",
            "data: not-json-ignored",
            "data: ",
            "",
        ]
        .join("\n");

        let messages = parse_sse_body(&body);
        assert_eq!(messages.len(), 2);
        assert!(messages[0].contains(r#""id":1"#));
        assert!(messages[1].contains(r#""id":2"#));
    }

    /// Helper to test SSE parsing without needing a real reqwest::Response.
    fn parse_sse_body(body: &str) -> Vec<String> {
        let mut messages = Vec::new();
        for line in body.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data.is_empty() {
                    continue;
                }
                if data.starts_with('{') {
                    messages.push(data.to_string());
                }
            }
        }
        messages
    }
}
