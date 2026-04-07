use daemon_core::{ensure_daemon_token as core_ensure_daemon_token, triumvirate_home_dir as core_triumvirate_home_dir};
use mcp_bridge::{
    daemon_ask_agent_url, daemon_fallback_ack_url, daemon_fallback_gc_url, daemon_fallback_list_url,
    daemon_autostart_enabled, daemon_health_url, daemon_memory_read_url, daemon_memory_write_url,
    daemon_outbox_recent_url, daemon_scratchpad_list_url, daemon_scratchpad_write_url,
    daemon_session_ask_url, daemon_session_dismiss_url, daemon_session_list_url, daemon_session_spawn_url,
    daemon_status_url, should_use_daemon_proxy,
};
use shared_types::{
    AskAgentRequest, AskAgentResponse, AskSessionRequest, DaemonHealthResponse, DaemonStatusSnapshot,
    DismissSessionRequest, FallbackAckRequest, FallbackGcRequest, FallbackGcResponse, FallbackListRequest,
    FallbackListResponse, MemoryReadRequest, MemoryReadResponse, MemoryWriteRequest, MemoryWriteResponse,
    OutboxRecentRequest, OutboxRecentResponse, ScratchpadListRequest, ScratchpadListResponse,
    ScratchpadWriteRequest, ScratchpadWriteResponse, SessionListResponse, SpawnSessionRequest,
};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{Duration, sleep};

static DAEMON_AUTOSTART_ATTEMPTED: AtomicBool = AtomicBool::new(false);

pub fn reset_daemon_autostart_flag_for_tests() {
    DAEMON_AUTOSTART_ATTEMPTED.store(false, Ordering::SeqCst);
}

pub fn attempt_daemon_autostart_once() -> anyhow::Result<bool> {
    if !daemon_autostart_enabled(std::env::var("TRIUMVIRATE_DAEMON_AUTOSTART").ok().as_deref()) {
        return Ok(false);
    }
    if DAEMON_AUTOSTART_ATTEMPTED.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }

    if should_use_daemon_proxy(std::env::var("TRIUMVIRATE_DAEMON_AUTOSTART_DRYRUN").ok().as_deref()) {
        return Ok(true);
    }

    let exe = std::env::current_exe()?;
    let _child = std::process::Command::new(exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(true)
}

async fn daemon_get_json<T: serde::de::DeserializeOwned>(url: String) -> anyhow::Result<T> {
    let token = core_ensure_daemon_token(&core_triumvirate_home_dir()?)?;
    let client = reqwest::Client::new();

    let first = client.get(&url).bearer_auth(&token).send().await;
    match first {
        Ok(response) => {
            if !response.status().is_success() {
                anyhow::bail!("daemon responded with HTTP {}", response.status());
            }
            Ok(response.json::<T>().await?)
        }
        Err(_) => {
            if attempt_daemon_autostart_once().unwrap_or(false) {
                sleep(Duration::from_millis(300)).await;
                let retry = client.get(&url).bearer_auth(token).send().await?;
                if !retry.status().is_success() {
                    anyhow::bail!("daemon responded with HTTP {}", retry.status());
                }
                return Ok(retry.json::<T>().await?);
            }
            anyhow::bail!("daemon request failed")
        }
    }
}

async fn daemon_post_json<TReq: serde::Serialize, TResp: serde::de::DeserializeOwned>(
    url: String,
    payload: &TReq,
) -> anyhow::Result<TResp> {
    let token = core_ensure_daemon_token(&core_triumvirate_home_dir()?)?;
    let client = reqwest::Client::new();

    let first = client.post(&url).bearer_auth(&token).json(payload).send().await;
    match first {
        Ok(response) => {
            if !response.status().is_success() {
                anyhow::bail!("daemon responded with HTTP {}", response.status());
            }
            Ok(response.json::<TResp>().await?)
        }
        Err(_) => {
            if attempt_daemon_autostart_once().unwrap_or(false) {
                sleep(Duration::from_millis(300)).await;
                let retry = client
                    .post(&url)
                    .bearer_auth(token)
                    .json(payload)
                    .send()
                    .await?;
                if !retry.status().is_success() {
                    anyhow::bail!("daemon responded with HTTP {}", retry.status());
                }
                return Ok(retry.json::<TResp>().await?);
            }
            anyhow::bail!("daemon request failed")
        }
    }
}

pub async fn fetch_daemon_status() -> anyhow::Result<DaemonHealthResponse> {
    if let Ok(health) = daemon_get_json::<DaemonHealthResponse>(daemon_health_url()).await {
        return Ok(health);
    }

    let status_json = daemon_get_json::<serde_json::Value>(daemon_status_url()).await?;
    Ok(DaemonHealthResponse {
        status: status_json
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("ok")
            .to_string(),
        service: status_json
            .get("service")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        mode: status_json
            .get("mode")
            .or_else(|| status_json.get("daemon_mode"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        daemon: status_json
            .get("daemon")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        auth: status_json
            .get("auth")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        daemon_bind_addr: status_json
            .get("daemon_bind_addr")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
    })
}

pub async fn fetch_daemon_status_snapshot() -> anyhow::Result<DaemonStatusSnapshot> {
    daemon_get_json::<DaemonStatusSnapshot>(daemon_status_url()).await
}

pub async fn fetch_daemon_ask_agent(req: &AskAgentRequest) -> anyhow::Result<AskAgentResponse> {
    daemon_post_json::<AskAgentRequest, AskAgentResponse>(daemon_ask_agent_url(), req).await
}

pub async fn fetch_daemon_session_spawn(req: &SpawnSessionRequest) -> anyhow::Result<String> {
    let json =
        daemon_post_json::<SpawnSessionRequest, serde_json::Value>(daemon_session_spawn_url(), req).await?;
    Ok(json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("session spawned")
        .to_string())
}

pub async fn fetch_daemon_session_ask(req: &AskSessionRequest) -> anyhow::Result<String> {
    let json = daemon_post_json::<AskSessionRequest, serde_json::Value>(daemon_session_ask_url(), req).await?;
    Ok(json
        .get("response")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string())
}

pub async fn fetch_daemon_session_dismiss(req: &DismissSessionRequest) -> anyhow::Result<String> {
    let json =
        daemon_post_json::<DismissSessionRequest, serde_json::Value>(daemon_session_dismiss_url(), req).await?;
    Ok(json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("session dismissed")
        .to_string())
}

pub async fn fetch_daemon_session_list() -> anyhow::Result<SessionListResponse> {
    daemon_get_json::<SessionListResponse>(daemon_session_list_url()).await
}

pub async fn fetch_daemon_memory_write(req: &MemoryWriteRequest) -> anyhow::Result<MemoryWriteResponse> {
    daemon_post_json::<MemoryWriteRequest, MemoryWriteResponse>(daemon_memory_write_url(), req).await
}

pub async fn fetch_daemon_memory_read(req: &MemoryReadRequest) -> anyhow::Result<MemoryReadResponse> {
    daemon_post_json::<MemoryReadRequest, MemoryReadResponse>(daemon_memory_read_url(), req).await
}

pub async fn fetch_daemon_scratchpad_write(
    req: &ScratchpadWriteRequest,
) -> anyhow::Result<ScratchpadWriteResponse> {
    daemon_post_json::<ScratchpadWriteRequest, ScratchpadWriteResponse>(daemon_scratchpad_write_url(), req).await
}

pub async fn fetch_daemon_scratchpad_list(
    req: &ScratchpadListRequest,
) -> anyhow::Result<ScratchpadListResponse> {
    daemon_post_json::<ScratchpadListRequest, ScratchpadListResponse>(daemon_scratchpad_list_url(), req).await
}

pub async fn fetch_daemon_outbox_recent(
    req: &OutboxRecentRequest,
) -> anyhow::Result<OutboxRecentResponse> {
    daemon_post_json::<OutboxRecentRequest, OutboxRecentResponse>(daemon_outbox_recent_url(), req).await
}

pub async fn fetch_daemon_fallback_list(req: &FallbackListRequest) -> anyhow::Result<FallbackListResponse> {
    daemon_post_json::<FallbackListRequest, FallbackListResponse>(daemon_fallback_list_url(), req).await
}

pub async fn fetch_daemon_fallback_ack(req: &FallbackAckRequest) -> anyhow::Result<String> {
    let json = daemon_post_json::<FallbackAckRequest, serde_json::Value>(daemon_fallback_ack_url(), req).await?;
    Ok(json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("acknowledged")
        .to_string())
}

pub async fn fetch_daemon_fallback_gc(req: &FallbackGcRequest) -> anyhow::Result<FallbackGcResponse> {
    daemon_post_json::<FallbackGcRequest, FallbackGcResponse>(daemon_fallback_gc_url(), req).await
}
