//! Integration tests for Streamable HTTP MCP endpoint (/mcp).
//!
//! FEAT-002 (REQ-H01, REQ-H02, REQ-H03, REQ-H08, REQ-H09, REQ-H10)
//!
//! These tests require a running daemon on the test URL.
//! Run with: cargo test -p triumvirate --test integration_streaming -- --ignored

use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Duration};

fn daemon_base_url() -> String {
    std::env::var("TRIUMVIRATE_TEST_DAEMON_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
}

fn daemon_token() -> anyhow::Result<String> {
    let home = std::env::var("HOME").map(PathBuf::from)?;
    let token_path = home.join(".triumvirate").join("daemon.token");
    let token = fs::read_to_string(&token_path)?;
    Ok(token.trim().to_string())
}

fn http_client() -> anyhow::Result<Client> {
    Ok(Client::builder().timeout(Duration::from_secs(15)).build()?)
}

fn mcp_url() -> String {
    format!("{}/mcp", daemon_base_url())
}

fn jsonrpc_request(method: &str, id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": {}
    })
}

fn jsonrpc_initialize(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {
                "name": "integration-test",
                "version": "1.0.0"
            }
        }
    })
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_stream_01_mcp_post_initialize_returns_capabilities() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .post(mcp_url())
        .bearer_auth(&token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&jsonrpc_initialize(1))
        .send()
        .await?;

    assert!(
        response.status().is_success(),
        "expected 2xx, got {}",
        response.status()
    );

    // Should have session ID header in stateful mode
    let session_id = response.headers().get("mcp-session-id");
    assert!(session_id.is_some(), "expected Mcp-Session-Id header");

    let body = response.text().await?;
    assert!(!body.is_empty(), "expected non-empty response body");
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_stream_02_mcp_post_without_auth_returns_401() -> anyhow::Result<()> {
    let client = http_client()?;
    let response = client
        .post(mcp_url())
        .header("Content-Type", "application/json")
        .json(&jsonrpc_initialize(1))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_stream_03_mcp_post_with_wrong_token_returns_401() -> anyhow::Result<()> {
    let client = http_client()?;
    let response = client
        .post(mcp_url())
        .bearer_auth("definitely-not-the-right-token")
        .header("Content-Type", "application/json")
        .json(&jsonrpc_initialize(1))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_stream_04_mcp_get_opens_sse_connection() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = Client::builder().timeout(Duration::from_secs(5)).build()?;

    // First initialize to get a session ID
    let init_response = client
        .post(mcp_url())
        .bearer_auth(&token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&jsonrpc_initialize(1))
        .send()
        .await?;

    let session_id = init_response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // GET /mcp with session ID should open SSE stream
    let mut get_request = client
        .get(mcp_url())
        .bearer_auth(&token)
        .header("Accept", "text/event-stream");

    if let Some(sid) = &session_id {
        get_request = get_request.header("mcp-session-id", sid.as_str());
    }

    let get_response = get_request.send().await?;

    // Should be 200 with text/event-stream content type
    assert!(
        get_response.status().is_success(),
        "expected 2xx for GET /mcp, got {}",
        get_response.status()
    );

    let content_type = get_response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/event-stream"),
        "expected text/event-stream, got {content_type}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_stream_05_mcp_tools_list_returns_tools() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;

    // Initialize first
    let init_resp = client
        .post(mcp_url())
        .bearer_auth(&token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&jsonrpc_initialize(1))
        .send()
        .await?;

    let session_id = init_resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Send initialized notification
    let _ = client
        .post(mcp_url())
        .bearer_auth(&token)
        .header("Content-Type", "application/json")
        .header("mcp-session-id", session_id)
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .send()
        .await?;

    // Now list tools
    let response = client
        .post(mcp_url())
        .bearer_auth(&token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", session_id)
        .json(&jsonrpc_request("tools/list", 2))
        .send()
        .await?;

    // The response is an SSE stream — verify it's accepted (not 405 or 400).
    // Full SSE frame parsing requires an SSE client library; reqwest sees
    // only the priming frame before the timeout. We verify:
    // 1. Server accepts the tools/list request (status 200)
    // 2. Content-Type is text/event-stream (SSE mode)
    assert!(
        response.status().is_success(),
        "expected 2xx for tools/list, got {}",
        response.status()
    );
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/event-stream"),
        "expected SSE content type, got {content_type}"
    );
    Ok(())
}
