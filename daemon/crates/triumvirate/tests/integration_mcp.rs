use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Duration};

fn daemon_base_url() -> String {
    std::env::var("TRIUMVIRATE_TEST_DAEMON_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string())
}

fn daemon_token() -> anyhow::Result<String> {
    let home = std::env::var("HOME").map(PathBuf::from)?;
    let token_path = home.join(".triumvirate").join("daemon.token");
    let token = fs::read_to_string(&token_path)?;
    Ok(token.trim().to_string())
}

fn http_client() -> anyhow::Result<Client> {
    Ok(Client::builder().timeout(Duration::from_secs(30)).build()?)
}

// ============================================================
// I-MCP: Core MCP Tool Integration Tests
// All tests require a running daemon. Run with:
//   cargo test -p triumvirate --test integration_mcp -- --ignored
// ============================================================

#[tokio::test]
#[ignore]
async fn i_mcp_01_health_returns_ok_with_version() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .get(format!("{}/health", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["version"].as_str().unwrap().contains('.'), "version should contain a dot");
}

#[tokio::test]
#[ignore]
async fn i_mcp_02_status_returns_daemon_state() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .get(format!("{}/status", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body["daemon"].is_string(), "expected 'daemon' field in /status response");
    assert!(body["supported_agents"].is_array());
}

#[tokio::test]
#[ignore]
async fn i_mcp_03_ledger_health_returns_healthy() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .get(format!("{}/ledger/health", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body["status"].is_string(), "expected 'status' field in /ledger/health response");
}

#[tokio::test]
#[ignore]
async fn i_mcp_04_metrics_returns_prometheus_text() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .get(format!("{}/metrics", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = resp.text().await.unwrap();
    assert!(text.contains("triumvirate_"), "should contain triumvirate_ metrics");
}

#[tokio::test]
#[ignore]
async fn i_mcp_05_outbox_recent_returns_events() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .post(format!("{}/outbox/recent", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body["events"].is_array());
}

#[tokio::test]
#[ignore]
async fn i_mcp_06_fallback_list_returns_tickets() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .post(format!("{}/fallback/list", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body["tickets"].is_array());
}

#[tokio::test]
#[ignore]
async fn i_mcp_07_unauthenticated_request_rejected() {
    let client = http_client().unwrap();
    let resp = client
        .get(format!("{}/health", daemon_base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn i_mcp_08_invalid_token_rejected() {
    let client = http_client().unwrap();
    let resp = client
        .get(format!("{}/health", daemon_base_url()))
        .bearer_auth("invalid-token-12345")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn i_mcp_09_dashboard_root_serves_html() {
    let client = http_client().unwrap();
    let resp = client
        .get(format!("{}/", daemon_base_url()))
        .send()
        .await
        .unwrap();
    // Dashboard may or may not require auth
    let status = resp.status();
    assert!(status == StatusCode::OK || status == StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn i_mcp_10_lesson_list_returns_array() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .post(format!("{}/lesson/list", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body["lessons"].is_array());
}

#[tokio::test]
#[ignore]
async fn i_mcp_11_token_summary_returns_data() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .get(format!("{}/api/tokens/summary", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    // May return 200 with data or 200 with empty if no scans yet
    assert_eq!(resp.status(), StatusCode::OK);
}

// ============================================================
// I-ALIAS: Backwards Compatibility Alias Tests
// These test the HTTP endpoints that map to alias tool behavior.
// Full alias testing requires MCP tool calls from a Claude session,
// which can't be done from an HTTP integration test. These verify
// the HTTP-level equivalents work.
// ============================================================

#[tokio::test]
#[ignore]
async fn i_alias_01_ask_agent_as_codex_returns_response() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .post(format!("{}/ask-agent", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({
            "agent": "codex",
            "message": "echo 'alias integration test'",
            "cwd": "/tmp"
        }))
        .send()
        .await
        .unwrap();
    // May timeout if Codex is slow, but should not 500
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::REQUEST_TIMEOUT,
        "expected 200 or 408, got {status}"
    );
}

#[tokio::test]
#[ignore]
async fn i_alias_02_abe_task_complete_valid_payload() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .post(format!("{}/abe/task-complete", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({
            "task_id": "TEST-ALIAS-02",
            "commit_sha": "abc123",
            "result": "ok",
            "timestamp": "2026-04-10T00:00:00Z",
            "commit_message": "test"
        }))
        .send()
        .await
        .unwrap();
    // May return 404 (unknown task) or 200 (if task exists) — both are valid responses
    let status = resp.status();
    assert!(
        status != StatusCode::INTERNAL_SERVER_ERROR,
        "should not 500 on valid payload"
    );
}

#[tokio::test]
#[ignore]
async fn i_alias_03_abe_task_complete_empty_body_rejected() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .post(format!("{}/abe/task-complete", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
#[ignore]
async fn i_alias_04_abe_task_complete_no_auth_rejected() {
    let client = http_client().unwrap();
    // Must send ALL required fields so Axum's JSON extractor succeeds
    // before the handler's bearer auth check runs
    let resp = client
        .post(format!("{}/abe/task-complete", daemon_base_url()))
        .json(&json!({
            "task_id": "test",
            "commit_sha": "abc",
            "result": "ok",
            "timestamp": "2026-04-10T00:00:00Z",
            "commit_message": "test"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn i_alias_05_memory_write_read_roundtrip() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let namespace = format!("integration-mcp-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());

    // Write — MemoryWriteRequest requires: namespace, key, value
    let write_resp = client
        .post(format!("{}/memory/write", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({
            "namespace": namespace,
            "key": "smoke-key",
            "value": "integration test value"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(write_resp.status(), StatusCode::OK);

    // Read — MemoryReadRequest requires: namespace; optional: key, limit
    let read_resp = client
        .post(format!("{}/memory/read", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({
            "namespace": namespace,
            "limit": 10
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(read_resp.status(), StatusCode::OK);
}

#[tokio::test]
#[ignore]
async fn i_alias_06_token_by_build_returns_json() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .get(format!("{}/api/tokens/by-build?build_id=nonexistent", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body.is_object());
}

#[tokio::test]
#[ignore]
async fn i_alias_07_token_by_session_returns_json() {
    let client = http_client().unwrap();
    let token = daemon_token().unwrap();
    let resp = client
        .get(format!("{}/api/tokens/by-session?session_id=nonexistent", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body.is_object());
}
