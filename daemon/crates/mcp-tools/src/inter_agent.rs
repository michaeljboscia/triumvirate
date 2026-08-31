use crate::{ProgressEmitter, display_agent_name, next_heartbeat_offset};
use agent_worker::{WorkerAcquireMode, acquire_worker, dismiss_worker};
use daemon_core::{
    daemon_bind_addr as core_daemon_bind_addr,
    persist_json_file_if_enabled as core_persist_json_file_if_enabled,
};
use daemon_http::{
    DaemonRequestError, DaemonRequestFailure, daemon_ask_timeout_secs,
    fetch_daemon_ask_agent, fetch_daemon_session_ask, fetch_daemon_session_dismiss,
    fetch_daemon_session_list, fetch_daemon_session_spawn, fetch_daemon_status,
    fetch_daemon_status_snapshot,
};
use fallback_outbox::{count_pending_fallbacks, list_pending_fallback_paths};
use mcp_bridge::{caller_driver_identity, is_supported_agent_name, normalize_agent_name};
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

/// Describes an `ask_agent` failure without asserting a cause we have not checked.
///
/// The old text appended "start it with: triumvirate daemon" to EVERY failure. On
/// 2026-07-28 that sent the operator hunting for a dead daemon that had been running
/// continuously for four days: the real cause was the 180s client ceiling on a dispatch
/// that needed 424s. An unverified remediation is worse than none, because it reads like a
/// diagnosis and closes the investigation.
fn describe_ask_agent_failure(err: &anyhow::Error) -> String {
    let detail = format!("ask_agent failed: {err:#}");
    match err.downcast_ref::<DaemonRequestError>().map(|e| e.failure) {
        Some(DaemonRequestFailure::Unreachable) => {
            format!("{detail}\nstart it with: triumvirate daemon")
        }
        Some(DaemonRequestFailure::Timeout) => format!(
            "{detail}\nthe daemon is running and may still be finishing this request; raise \
             TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS (currently {}s) if the model needs longer",
            daemon_ask_timeout_secs()
        ),
        _ => detail,
    }
}

pub async fn ask_agent(
    req: &AskAgentRequest,
    context: &RequestContext<RoleServer>,
    local_test_execution_allowed: bool,
    execute_ask_agent: ExecuteAskAgentFn,
) -> Result<Json<AskAgentResponse>, String> {
    if let Some(driver) = caller_driver_identity() {
        // Compare canonical identities so an agy/antigravity caller can't slip past
        // the self-ask guard by using an alias of its own name.
        if normalize_agent_name(&req.agent) == normalize_agent_name(&driver) {
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
                            emitter.emit(format!("→ {display}: FAILED ✗ ({err:#})")).await;
                            return Err(describe_ask_agent_failure(&err));
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
    let agent = normalize_agent_name(&req.agent);
    if !is_supported_agent_name(&agent) {
        // Derived, not hand-written. A literal here had already drifted: it advertised
        // deepseek and claude, which `daemon_target_agents()` excludes, and omitted grok.
        return Err(format!(
            "spawn_session supports only: {}",
            crate::aliases::daemon_target_agents().join(", ")
        ));
    }
    let cwd = req.cwd.clone().unwrap_or_else(|| ".".to_string());
    // Per NAMED session, so two sessions for one agent in one directory do not resume each
    // other. Keyed on (agent, cwd) alone this leaked a passphrase between sessions.
    // Respawning an existing name must start a NEW conversation. Without this the visible
    // history resets while the worker keeps its old CLI session id, so the next ask silently
    // resumes the previous transcript under a session that looks fresh.
    agent_worker::reset_worker_session(&agent, &cwd, Some(req.name.as_str())).await;
    let worker = acquire_worker(&agent, &cwd, Some(req.name.as_str())).await;

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
            // Normalize on read: any session persisted under a raw alias (e.g. an
            // older `agent:"agy"` record) must resolve to the canonical key so it
            // does not hit the unsupported-agent dispatch path.
            normalize_agent_name(&state.agent),
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
            // A named session is the one caller that genuinely wants to resume: multi-turn
            // memory is the whole point. One-shot ask_agent leaves this None and starts fresh.
            reuse_session: Some(true),
            // WITHOUT this the worker key falls back to (agent, cwd) and two named sessions in
            // one directory resume each other. The HTTP route was fixed first and verified
            // through the HTTP route, so this in-process MCP path kept the leak. Found by Codex.
            session_key: Some(req.name.clone()),
            ..Default::default()
        },
        None,
    )
    .await
    .map_err(|e| format!("ask_session failed: {e}"))?
    .response;

    // REQ-044: the agy backend is single-turn. When a follow-up to a named
    // Antigravity session cannot carry the earlier turns, say so — never silently
    // fake continuity. Only on turn 2+ (turn 1 has no prior context to lose).
    if agent == "gemini"
        && had_history
        && mcp_bridge::gemini_backend() == mcp_bridge::GeminiBackend::Agy
    {
        response = format!(
            "⚠ Multi-turn memory is not available for the Antigravity sibling — this answer does not carry the earlier turns of this session.\n\n{response}"
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
            // Each named session now owns its worker record, so dismissing one can never strand
            // another. The old "is any other session sharing (agent, cwd)?" guard existed only
            // because they DID share, which is the bug this key change removes.
            let cwd = removed_session.cwd.clone().unwrap_or_else(|| ".".to_string());
            let _ = dismiss_worker(&removed_session.agent, &cwd, Some(req.name.as_str())).await;
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
                .unwrap_or_else(|| {
                    mcp_bridge::supported_agent_names()
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                }),
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
        // REQ-GROK-003: single source of truth. This list previously omitted `claude`,
        // which was dispatchable but advertised nowhere.
        supported_agents: mcp_bridge::supported_agent_names()
            .iter()
            .map(|s| s.to_string())
            .collect(),
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

#[cfg(test)]
mod ask_agent_failure_message_tests {
    use super::*;

    fn daemon_error(failure: DaemonRequestFailure) -> anyhow::Error {
        anyhow::Error::new(DaemonRequestError {
            failure,
            url: "http://127.0.0.1:8080/ask-agent".to_string(),
            detail: "error sending request: operation timed out".to_string(),
            waited: (failure == DaemonRequestFailure::Timeout)
                .then_some(std::time::Duration::from_secs(180)),
        })
    }

    #[test]
    fn timeout_does_not_tell_the_user_to_start_a_running_daemon() {
        let message = describe_ask_agent_failure(&daemon_error(DaemonRequestFailure::Timeout));
        assert!(
            !message.contains("start it with"),
            "the daemon was never down; this line is what sent the operator hunting a corpse: {message}"
        );
        assert!(message.contains("may still be finishing"), "{message}");
        assert!(message.contains("TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS"), "{message}");
    }

    #[test]
    fn unreachable_still_gets_the_start_instruction() {
        let message = describe_ask_agent_failure(&daemon_error(DaemonRequestFailure::Unreachable));
        assert!(message.contains("start it with: triumvirate daemon"), "{message}");
        assert!(!message.contains("may still be finishing"), "{message}");
    }

    #[test]
    fn an_unclassified_failure_prescribes_nothing() {
        let message = describe_ask_agent_failure(&anyhow::anyhow!("something else entirely"));
        assert_eq!(message, "ask_agent failed: something else entirely");
    }

    #[test]
    fn the_message_carries_the_source_chain() {
        let message = describe_ask_agent_failure(&daemon_error(DaemonRequestFailure::Timeout));
        // `{err}` alone would print only the outermost frame, which is identical for a
        // refused connection and a timeout. `{err:#}` plus the preserved detail is the fix.
        assert!(message.contains("operation timed out"), "{message}");
        assert!(!message.contains(r"\"), "stray backslash from the old literal: {message}");
    }
    /// Every production caller that RESUMES must carry a session key.
    ///
    /// The HTTP route was fixed first and verified through the HTTP route, so this in-process MCP
    /// path kept the cross-session leak: resuming with no session key falls back to the anonymous
    /// `(agent, cwd)` worker, which is exactly the bug. Codex found it in review. This asserts the
    /// invariant on the SOURCE so a future caller cannot repeat it.
    #[test]
    fn u_ia_resume_callers_must_carry_a_session_key() {
        // Built at runtime so this test's own source cannot match the pattern it searches for.
        let needle = format!("reuse_session: {}(true)", "Some");
        let src = include_str!("inter_agent.rs");
        let mut checked = 0;
        for (i, _) in src.match_indices(needle.as_str()) {
            let window = &src[i..src.len().min(i + 400)];
            assert!(
                window.contains("session_key:"),
                "a resume call at byte {i} has no adjacent session key; it would resume the \
                 anonymous (agent, cwd) worker and cross sessions"
            );
            checked += 1;
        }
        assert!(checked > 0, "the invariant found nothing to check, so it guards nothing");
    }

}
