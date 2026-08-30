//! Pantheon v3.9.0 integration smoke tests.
//!
//! FEAT-012 (REQ-017), FEAT-013 (REQ-020), FEAT-011 (REQ-010, REQ-033),
//! FEAT-014 (REQ-020). T-011 of the Pantheon v3.9.0 sprint.
//!
//! These tests drive a REAL running daemon via HTTP + WebSocket and prove
//! that the Wave 1 + Wave 2 surfaces Pantheon's Tauri app consumes actually
//! compose end-to-end. Unlike the in-process `#[cfg(test)] mod` reality
//! tests in `main.rs`, these talk to the daemon binary over the loopback
//! bind address at the real token from `~/.triumvirate/daemon.token`.
//!
//! Running:
//!   1. Start a daemon: `triumvirate daemon` (or whatever launcher you use).
//!   2. `cargo test -p triumvirate --test integration_pantheon -- --ignored`
//!
//! Tests are `#[ignore]` by default because they require a running daemon;
//! the normal `cargo test` path skips them. This matches the pattern used
//! by the existing `integration_http.rs` / `integration_abe.rs` /
//! `integration_mcp.rs` files.
//!
//! Coverage:
//!   - GET /api/state        — StateResponse shape, version, uptime, auth
//!   - GET /api/workers      — WorkersResponse shape + empty-array semantics
//!   - GET /api/fleet        — FleetResponse shape + empty-array semantics
//!   - GET /api/fleet/{id}   — 404 on missing build, axum 0.8 path syntax
//!   - GET /ws/v2            — subscribe-before-read handshake, envelope wire
//!     format, out_of_range close frame
//!   - GET /ws               — legacy bootstrap frames unchanged
//!     (backwards-compat for `triumvirate watch`)

use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use shared_types::{FleetResponse, ReplayResponse, StateResponse, WorkersResponse};
use std::{fs, path::PathBuf, time::Duration};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

// --- helpers ---------------------------------------------------------------

fn daemon_base_url() -> String {
    std::env::var("TRIUMVIRATE_TEST_DAEMON_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string())
}

fn daemon_ws_base_url() -> String {
    daemon_base_url().replacen("http://", "ws://", 1)
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

// --- /api/state ------------------------------------------------------------

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_01_api_state_returns_pantheon_snapshot() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!("{}/api/state", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: StateResponse = response.json().await?;

    // version must match the compile-time daemon-core constant. The test
    // binary is compiled against the same workspace so this pins the two.
    assert_eq!(body.version, daemon_core::VERSION.to_string());
    assert!(body.uptime_ms > 0, "uptime_ms must be populated by started_at");
    // workers + fleet are Vec, never null (critical for the Tauri client).
    let _: &Vec<shared_types::WorkerInfo> = &body.workers;
    let _: &Vec<shared_types::FleetBuild> = &body.fleet;
    let _: u64 = body.last_event_seq;

    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_02_api_state_rejects_missing_bearer() -> anyhow::Result<()> {
    let client = http_client()?;
    let response = client
        .get(format!("{}/api/state", daemon_base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

// --- /api/workers ----------------------------------------------------------

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_03_api_workers_returns_array_never_null() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!("{}/api/workers", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    // Raw body check first — the Tauri client would crash on `null`.
    let raw = response.text().await?;
    assert!(
        raw.contains("\"workers\":["),
        "workers must serialize as an array, got: {raw}"
    );
    assert!(
        !raw.contains("\"workers\":null"),
        "workers must never serialize as null, got: {raw}"
    );
    // Then re-parse into the typed shape.
    let body: WorkersResponse = serde_json::from_str(&raw)?;
    let _: &Vec<shared_types::WorkerInfo> = &body.workers;
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_04_api_workers_rejects_missing_bearer() -> anyhow::Result<()> {
    let client = http_client()?;
    let response = client
        .get(format!("{}/api/workers", daemon_base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

// --- /api/fleet ------------------------------------------------------------

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_05_api_fleet_returns_array_never_null() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    let response = client
        .get(format!("{}/api/fleet", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let raw = response.text().await?;
    assert!(raw.contains("\"builds\":["), "builds must be an array: {raw}");
    assert!(!raw.contains("\"builds\":null"), "builds must never be null: {raw}");
    let body: FleetResponse = serde_json::from_str(&raw)?;
    let _: &Vec<shared_types::FleetBuild> = &body.builds;
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_06_api_fleet_by_id_returns_404_for_unknown_build() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let client = http_client()?;
    // axum 0.8 `{build_id}` path syntax — if the daemon panicked at router
    // construction on the old `:build_id` form, this test couldn't reach it.
    let response = client
        .get(format!(
            "{}/api/fleet/nonexistent-build-id-{}",
            daemon_base_url(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

// --- /ws/v2 ----------------------------------------------------------------

/// Connect to /ws/v2 with a bearer header.
async fn connect_ws_v2(
    token: &str,
) -> anyhow::Result<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
> {
    let url = format!("{}/ws/v2", daemon_ws_base_url());
    let mut req = url.into_client_request()?;
    req.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}"))?,
    );
    let (stream, _resp) = tokio_tungstenite::connect_async(req).await?;
    Ok(stream)
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_07_ws_v2_handshake_returns_ok_ack() -> anyhow::Result<()> {
    let token = daemon_token()?;
    let mut stream = connect_ws_v2(&token).await?;
    // Subscribe with last_seq=0 — the daemon will either return an
    // "ok" ack (if the buffer currently has any events <= last_seq which
    // means last_seq=0 is in-range trivially) or an out_of_range frame
    // (very unlikely on a fresh daemon with <1000 events in the buffer).
    stream
        .send(WsMessage::Text(
            serde_json::json!({"action": "subscribe", "last_seq": 0})
                .to_string()
                .into(),
        ))
        .await?;

    // First frame must be a bare ReplayResponse (either "ok" or "out_of_range").
    let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("no frame"))??;
    let text = match frame {
        WsMessage::Text(t) => t.to_string(),
        other => anyhow::bail!("expected text handshake frame, got {other:?}"),
    };
    let value: Value = serde_json::from_str(&text)?;
    // Bare ReplayResponse: top-level `replay` field, no `type`/`payload`.
    assert!(
        value.get("replay").is_some(),
        "handshake frame must carry top-level `replay` field: {text}"
    );
    assert!(
        value.get("type").is_none(),
        "handshake frame must NOT be envelope-wrapped: {text}"
    );
    let resp: ReplayResponse = serde_json::from_str(&text)?;
    assert!(
        resp.replay == "ok" || resp.replay == "out_of_range",
        "unexpected replay value: {}",
        resp.replay
    );

    let _ = stream.close(None).await;
    Ok(())
}

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_08_ws_v2_rejects_missing_bearer_on_upgrade() -> anyhow::Result<()> {
    let url = format!("{}/ws/v2", daemon_ws_base_url());
    let req = url.into_client_request()?;
    let result = tokio_tungstenite::connect_async(req).await;
    assert!(
        result.is_err(),
        "/ws/v2 must fail upgrade with 401 when bearer is missing"
    );
    Ok(())
}

// --- legacy /ws (regression check for `triumvirate watch`) -----------------

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_09_legacy_ws_bootstrap_frames_unchanged() -> anyhow::Result<()> {
    // The legacy /ws route lives in daemon_http::ws_route and does NOT
    // require bearer auth on the upgrade. It emits 4 hardcoded bootstrap
    // frames on connect. Pantheon v3.9.0's T-009 work must not regress
    // this — `triumvirate watch` connects without an Authorization header.
    let url = format!("{}/ws", daemon_ws_base_url());
    let (mut stream, _resp) = tokio_tungstenite::connect_async(url).await?;

    let expected_types = [
        "agent_state",
        "fleet_progress",
        "ledger_health",
        "review_completed",
    ];
    for expected in expected_types {
        let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("bootstrap frame missing"))??;
        let text = match frame {
            WsMessage::Text(t) => t.to_string(),
            other => anyhow::bail!("expected text bootstrap, got {other:?}"),
        };
        let value: Value = serde_json::from_str(&text)?;
        assert_eq!(
            value.get("type").and_then(|v| v.as_str()),
            Some(expected),
            "legacy /ws bootstrap order must match pre-T-009 behavior"
        );
    }

    let _ = stream.close(None).await;
    Ok(())
}

// --- composition smoke test ------------------------------------------------

#[tokio::test]
#[ignore = "integration: requires daemon running"]
async fn i_pantheon_10_api_state_and_api_workers_agree_on_worker_count() -> anyhow::Result<()> {
    // Cross-endpoint consistency check: /api/state.workers and /api/workers
    // pull from the same TaskTracker::snapshot_workers source. They must
    // return the same set for the same instant. This is the simplest
    // composition test for Wave 1 + Wave 2 integration.
    let token = daemon_token()?;
    let client = http_client()?;

    let state: StateResponse = client
        .get(format!("{}/api/state", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await?
        .json()
        .await?;
    let workers: WorkersResponse = client
        .get(format!("{}/api/workers", daemon_base_url()))
        .bearer_auth(&token)
        .send()
        .await?
        .json()
        .await?;

    // Both endpoints snapshot the same tracker. Counts can only differ if
    // a task registered/completed in the nanoseconds between the two HTTP
    // round-trips — acceptable in a running daemon — so we assert they're
    // within 2 of each other rather than exactly equal.
    let state_count = state.workers.len() as isize;
    let workers_count = workers.workers.len() as isize;
    assert!(
        (state_count - workers_count).abs() <= 2,
        "/api/state.workers ({state_count}) and /api/workers ({workers_count}) must agree within a small race window"
    );
    Ok(())
}
