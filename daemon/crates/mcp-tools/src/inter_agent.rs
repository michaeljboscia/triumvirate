use crate::{ProgressEmitter, display_agent_name, next_heartbeat_offset};
use agent_worker::{WorkerAcquireMode, acquire_worker, dismiss_worker};
use daemon_core::{
    daemon_bind_addr as core_daemon_bind_addr,
    persist_json_file_if_enabled as core_persist_json_file_if_enabled,
};
use daemon_http::{
    fetch_daemon_ask_agent, fetch_daemon_session_ask, fetch_daemon_session_dismiss,
    fetch_daemon_session_list, fetch_daemon_session_spawn, fetch_daemon_status,
    fetch_daemon_status_snapshot,
};
use fallback_outbox::{count_pending_fallbacks, list_pending_fallback_paths};
use mcp_bridge::{caller_driver_identity, is_supported_agent_name};
use rmcp::{
    Json,
    service::{RequestContext, RoleServer},
};
use shared_types::{
    AskAgentRequest, AskAgentResponse, AskSessionRequest, DaemonHealthResponse,
    DismissSessionRequest, SessionInfo, SessionListResponse, SessionState, SpawnSessionRequest,
    StatusResponse,
};
use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
};
use tokio::{
    sync::Mutex,
    time::{Duration, Instant, sleep},
};

pub type Sessions = Arc<Mutex<HashMap<String, SessionState>>>;
pub type ExecuteAskAgentFn = for<'a> fn(
    &'a AskAgentRequest,
    Option<ProgressEmitter>,
) -> Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>>;

pub async fn ask_agent(
    req: &AskAgentRequest,
    context: &RequestContext<RoleServer>,
    local_test_execution_allowed: bool,
    execute_ask_agent: ExecuteAskAgentFn,
) -> Result<Json<AskAgentResponse>, String> {
    if let Some(driver) = caller_driver_identity() {
        if req.agent.to_lowercase() == driver.to_lowercase() {
            return Err("cannot ask yourself — caller identity matches target".to_string());
        }
    }

    let emitter = ProgressEmitter::from_context(context);
    if !local_test_execution_allowed {
        let display = display_agent_name(&req.agent);
        emitter.emit(format!("→ {display}: sent ✓")).await;
        let mut pending = Box::pin(fetch_daemon_ask_agent(req));
        let started = Instant::now();
        let mut next_heartbeat = Duration::from_secs(10);
        loop {
            let sleep_duration = next_heartbeat.saturating_sub(started.elapsed());
            tokio::select! {
                result = &mut pending => {
                    match result {
                        Ok(response) => {
                            emitter.emit(format!("→ {display}: responded ✓")).await;
                            return Ok(Json(response));
                        }
                        Err(err) => {
                            emitter.emit(format!("→ {display}: FAILED ✗ ({err})")).await;
                            return Err(format!(
                                "ask_agent requires triumvirate daemon; daemon request failed: {err}. \\
start it with: triumvirate daemon"
                            ));
                        }
                    }
                }
                _ = sleep(sleep_duration) => {
                    if started.elapsed() >= next_heartbeat {
                        emitter
                            .emit(format!("→ {display}: working... ({}s elapsed)", started.elapsed().as_secs()))
                            .await;
                        next_heartbeat = next_heartbeat_offset(next_heartbeat);
                    }
                }
            }
        }
    }

    execute_ask_agent(req, Some(emitter)).await.map(Json)
}

pub async fn spawn_session(
    sessions: &Sessions,
    sessions_file: Option<&PathBuf>,
    req: &SpawnSessionRequest,
    mcp_daemon_proxy_enabled: bool,
) -> Result<String, String> {
    if mcp_daemon_proxy_enabled {
        return fetch_daemon_session_spawn(req)
            .await
            .map_err(|e| format!("spawn_session via daemon failed: {e}"));
    }
    let agent = req.agent.to_lowercase();
    if !is_supported_agent_name(&agent) {
        return Err(
            "spawn_session supports only 'gemini', 'codex', 'deepseek', 'claude', or 'agy'"
                .to_string(),
        );
    }
    let cwd = req.cwd.clone().unwrap_or_else(|| ".".to_string());
    let worker = acquire_worker(&agent, &cwd).await;

    let mut sessions = sessions.lock().await;
    sessions.insert(
        req.name.clone(),
        SessionState {
            agent: agent.clone(),
            cwd: Some(cwd),
            history: Vec::new(),
            // FEAT-011 (REQ-010, REQ-033): lineage fields for Pantheon v4.0
            // sidebar hierarchy. Captured later during MCP dispatch (T-004)
            // from _meta.pantheon.session_id or X-Pantheon-Session-Id header.
            parent_session_id: None,
            root_session_id: None,
            pantheon_session_id: None,
        },
    );
    core_persist_json_file_if_enabled(sessions_file, &*sessions)
        .map_err(|e| format!("failed to persist sessions: {e}"))?;
    Ok(format!(
        "session '{}' {} for {}",
        req.name,
        if worker.mode == WorkerAcquireMode::Spawned {
            "spawned"
        } else {
            "reused"
        },
        agent
    ))
}

pub async fn ask_session(
    sessions: &Sessions,
    sessions_file: Option<&PathBuf>,
    req: &AskSessionRequest,
    mcp_daemon_proxy_enabled: bool,
    execute_ask_agent: ExecuteAskAgentFn,
) -> Result<String, String> {
    if mcp_daemon_proxy_enabled {
        return fetch_daemon_session_ask(req)
            .await
            .map_err(|e| format!("ask_session via daemon failed: {e}"));
    }

    let (agent, cwd, had_history) = {
        let mut sessions = sessions.lock().await;
        let state = sessions
            .get_mut(&req.name)
            .ok_or_else(|| format!("session '{}' not found", req.name))?;
        (
            state.agent.clone(),
            state.cwd.clone(),
            !state.history.is_empty(),
        )
    };

    let mut response = execute_ask_agent(
        &AskAgentRequest {
            agent: agent.clone(),
            message: req.message.clone(),
            cwd: cwd.clone(),
            repo: None,
            branch: None,
            ..Default::default()
        },
        None,
    )
    .await
            // A named session is the one caller that genuinely wants to resume: multi-turn
            // memory is the whole point. One-shot ask_agent leaves this None and starts fresh.
            reuse_session: Some(true),
    .map_err(|e| format!("ask_session failed: {e}"))?
    .response;

    // REQ-044: the agy backend is single-turn. When a follow-up to a named Gemini
    // session cannot carry the earlier turns, say so — never silently fake continuity.
    // Only on turn 2+ (turn 1 has no prior context to lose).
    if agent == "gemini"
        && had_history
        && mcp_bridge::gemini_backend() == mcp_bridge::GeminiBackend::Agy
    {
        response = format!(
            "⚠ Multi-turn memory is not available for the Gemini sibling under the agy backend — this answer does not carry the earlier turns of this session.\n\n{response}"
        );
    }

    let mut sessions = sessions.lock().await;
    if let Some(state) = sessions.get_mut(&req.name) {
        state.history.push(format!("user: {}", req.message));
        state.history.push(format!("assistant: {response}"));
    }
    core_persist_json_file_if_enabled(sessions_file, &*sessions)
        .map_err(|e| format!("failed to persist sessions: {e}"))?;

    Ok(response)
}

pub async fn dismiss_session(
    sessions: &Sessions,
    sessions_file: Option<&PathBuf>,
    req: &DismissSessionRequest,
    mcp_daemon_proxy_enabled: bool,
) -> Result<String, String> {
    if mcp_daemon_proxy_enabled {
        return fetch_daemon_session_dismiss(req)
            .await
            .map_err(|e| format!("dismiss_session via daemon failed: {e}"));
    }
    let mut sessions = sessions.lock().await;
    match sessions.remove(&req.name) {
        Some(removed_session) => {
            let should_drop_worker = !sessions
                .values()
                .any(|s| s.agent == removed_session.agent && s.cwd == removed_session.cwd);
            if should_drop_worker {
                let cwd = removed_session.cwd.unwrap_or_else(|| ".".to_string());
                let _ = dismiss_worker(&removed_session.agent, &cwd).await;
            }
            core_persist_json_file_if_enabled(sessions_file, &*sessions)
                .map_err(|e| format!("failed to persist sessions: {e}"))?;
            Ok(format!("session '{}' dismissed", req.name))
        }
        None => Err(format!("session '{}' not found", req.name)),
    }
}

pub async fn list_sessions(
    sessions: &Sessions,
    mcp_daemon_proxy_enabled: bool,
) -> Json<SessionListResponse> {
    if mcp_daemon_proxy_enabled
        && let Ok(response) = fetch_daemon_session_list().await
    {
        return Json(response);
    }
    let sessions = sessions.lock().await;
    let mut out = sessions
        .iter()
        .map(|(name, s)| SessionInfo {
            name: name.clone(),
            agent: s.agent.clone(),
            turns: s.history.len(),
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Json(SessionListResponse { sessions: out })
}

pub async fn get_status(
    sessions: &Sessions,
    mcp_daemon_proxy_enabled: bool,
) -> Json<StatusResponse> {
    let sessions = sessions.lock().await;
    let local_bind_addr =
        core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());
    if mcp_daemon_proxy_enabled
        && let Ok(snapshot) = fetch_daemon_status_snapshot().await
    {
        return Json(StatusResponse {
            daemon_mode: snapshot
                .daemon_mode
                .unwrap_or_else(|| "incremental-dev".to_string()),
            active_sessions: sessions.len(),
            supported_agents: snapshot
                .supported_agents
                .unwrap_or_else(|| vec!["gemini".to_string(), "codex".to_string()]),
            pending_fallbacks: snapshot.pending_fallbacks.unwrap_or(0),
            fallback_tickets: snapshot.fallback_tickets.unwrap_or_default(),
            daemon_bind_addr: snapshot.daemon_bind_addr.unwrap_or(local_bind_addr),
        });
    }
    let pending_fallbacks = count_pending_fallbacks().unwrap_or(0);
    let fallback_tickets = list_pending_fallback_paths(10).unwrap_or_default();
    Json(StatusResponse {
        daemon_mode: "incremental-dev".to_string(),
        active_sessions: sessions.len(),
        supported_agents: vec![
            "gemini".to_string(),
            "codex".to_string(),
            "deepseek".to_string(), // T-001 (REQ-DS-001/013/016)
        ],
        pending_fallbacks,
        fallback_tickets: fallback_tickets
            .into_iter()
            .map(|p| p.display().to_string())
            .collect(),
        daemon_bind_addr: local_bind_addr,
    })
}

pub async fn daemon_health() -> Result<Json<DaemonHealthResponse>, String> {
    fetch_daemon_status()
        .await
        .map(Json)
        .map_err(|e| format!("daemon health query failed: {e}"))
}
