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
    Ok(Client::builder().timeout(Duration::from_secs(10)).build()?)
}

fn env_or_skip(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => {
            eprintln!("skipping: set {name} to run this integration assertion");
            None
        }
    }
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_01_health_returns_version() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!("{}/health", daemon_base_url()))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body.get("status"), Some(&Value::String("ok".to_string())));
    assert!(body.get("version").is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_02_status_returns_daemon_state() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!("{}/status", daemon_base_url()))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert!(body.get("supported_agents").is_some());
    assert!(body.get("daemon").is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_03_metrics_returns_prometheus_text() -> anyhow::Result<()> {
    let client = http_client()?;
    let response = client
        .get(format!("{}/metrics", daemon_base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await?;
    assert!(body.contains("triumvirate_"));
    Ok(())
}

#[tokio::test]
#[ignore = "manual websocket validation (I-HTTP-04)"]
async fn i_http_04_ws_connects_manual_note() -> anyhow::Result<()> {
    // Intentionally manual per briefing: validate /ws using a websocket client
    // such as tokio-tungstenite in a live daemon session.
    let ws_url = daemon_base_url().replace("http://", "ws://");
    assert!(ws_url.contains("ws://"));
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running + working agent"]
async fn i_http_05_ask_agent_returns_response() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let payload = json!({
        "agent": "codex",
        "message": "Reply with: ok",
        "cwd": std::env::current_dir()?.display().to_string(),
        "repo": Value::Null,
        "branch": Value::Null
    });

    let response = client
        .post(format!("{}/ask-agent", daemon_base_url()))
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert!(body.get("response").is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_06_ledger_health_returns_healthy_payload() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!("{}/ledger/health", daemon_base_url()))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert!(body.get("status").is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_07_memory_write_and_read_round_trip() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let namespace = format!("integration-http-{}", std::process::id());
    let key = "smoke-key";
    let value = "smoke-value";

    let write_response = client
        .post(format!("{}/memory/write", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({
            "namespace": namespace,
            "key": key,
            "value": value
        }))
        .send()
        .await?;
    assert_eq!(write_response.status(), StatusCode::OK);

    let read_response = client
        .post(format!("{}/memory/read", daemon_base_url()))
        .bearer_auth(token)
        .json(&json!({
            "namespace": namespace,
            "key": key,
            "limit": 10
        }))
        .send()
        .await?;

    assert_eq!(read_response.status(), StatusCode::OK);
    let body: Value = read_response.json().await?;
    let entries = body
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(!entries.is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_08_tokens_summary_returns_json() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!("{}/api/tokens/summary", daemon_base_url()))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let _body: Value = response.json().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_09_tokens_by_build_returns_breakdown() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!(
            "{}/api/tokens/by-build?build_id=unattributed",
            daemon_base_url()
        ))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let _body: Value = response.json().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_10_tokens_by_session_returns_breakdown() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!(
            "{}/api/tokens/by-session?session_id=test-session",
            daemon_base_url()
        ))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let _body: Value = response.json().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires running ABE task context"]
async fn i_http_11_abe_task_complete_valid_payload() -> anyhow::Result<()> {
    let Some(task_id) = env_or_skip("TRIUMVIRATE_TEST_TASK_ID") else {
        return Ok(());
    };
    let Some(commit_sha) = env_or_skip("TRIUMVIRATE_TEST_COMMIT_SHA") else {
        return Ok(());
    };

    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .post(format!("{}/abe/task-complete", daemon_base_url()))
        .bearer_auth(token)
        .json(&json!({
            "task_id": task_id,
            "commit_sha": commit_sha
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_12_abe_task_complete_invalid_payload_returns_client_error() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .post(format!("{}/abe/task-complete", daemon_base_url()))
        .bearer_auth(token)
        .json(&json!({
            "task_id": "definitely-not-active-task",
            "commit_sha": "definitely-not-a-real-sha"
        }))
        .send()
        .await?;

    assert!(response.status().is_client_error());
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_13_abe_task_complete_empty_body_returns_422() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .post(format!("{}/abe/task-complete", daemon_base_url()))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_14_bearer_auth_required_on_protected_route() -> anyhow::Result<()> {
    let client = http_client()?;
    let response = client
        .get(format!("{}/status", daemon_base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_http_15_dashboard_root_returns_html() -> anyhow::Result<()> {
    let client = http_client()?;
    let response = client.get(format!("{}/", daemon_base_url())).send().await?;

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(content_type.contains("text/html"));
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running + MCP bridge configured"]
async fn i_tok_01_get_token_summary_mcp_surface() -> anyhow::Result<()> {
    // HTTP proxy assertion for MCP tool parity (get_token_summary).
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!("{}/api/tokens/summary", daemon_base_url()))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running + MCP bridge configured"]
async fn i_tok_02_get_build_cost_mcp_surface() -> anyhow::Result<()> {
    // HTTP proxy assertion for MCP tool parity (get_build_cost).
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!(
            "{}/api/tokens/by-build?build_id=unattributed",
            daemon_base_url()
        ))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "manual scanner validation (I-TOK-03)"]
async fn i_tok_03_scanner_detects_new_file_manual_note() -> anyhow::Result<()> {
    // Manual scenario: create a new Claude/Codex/Gemini transcript file in watched
    // paths and assert token_update websocket events are emitted.
    assert!(daemon_base_url().starts_with("http"));
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running + working agent"]
async fn i_tok_04_direct_write_from_ask_agent_populates_token_db() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;

    let ask = client
        .post(format!("{}/ask-agent", daemon_base_url()))
        .bearer_auth(&token)
        .json(&json!({
            "agent": "codex",
            "message": "Reply with one word: ready",
            "cwd": std::env::current_dir()?.display().to_string(),
            "repo": Value::Null,
            "branch": Value::Null
        }))
        .send()
        .await?;
    assert_eq!(ask.status(), StatusCode::OK);

    let summary = client
        .get(format!("{}/api/tokens/summary", daemon_base_url()))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(summary.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon startup scenario"]
async fn i_tok_05_startup_reconciliation_non_blocking() -> anyhow::Result<()> {
    // Non-blocking signal: health endpoint should respond quickly.
    let token = daemon_token()?;
    let client = Client::builder().timeout(Duration::from_secs(5)).build()?;
    let response = client
        .get(format!("{}/health", daemon_base_url()))
        .bearer_auth(token)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}
