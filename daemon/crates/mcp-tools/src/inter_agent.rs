use crate::{ProgressEmitter, display_agent_name, next_heartbeat_offset};
use agent_worker::{WorkerAcquireMode, acquire_worker, dismiss_worker};
use daemon_core::{
    PublishResult, TurnOutcome, acquire_session_lock,
    daemon_bind_addr as core_daemon_bind_addr,
    persist_json_file_if_enabled as core_persist_json_file_if_enabled,
    reload_session_if_enabled as core_reload_session_if_enabled, session_busy_wait,
    triumvirate_home_dir,
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
    DismissSessionRequest, SessionInfo, SessionListResponse, SessionPhase, SessionState, SpawnSessionRequest,
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

fn is_new_seat(agent: &str) -> bool {
    mcp_bridge::seat_profile::seat_profile(agent).is_some_and(|profile| {
        profile.resume_strategy != mcp_bridge::seat_profile::ResumeStrategy::Legacy
    })
}

/// Describes an `ask_agent` failure without asserting a cause we have not checked.
///
/// The old text appended "start it with: triumvirate daemon" to EVERY failure. On
/// 2026-07-28 that sent the operator hunting for a dead daemon that had been running
/// continuously for four days: the real cause was the 180s client ceiling on a dispatch
/// that needed 424s. An unverified remediation is worse than none, because it reads like a
/// diagnosis and closes the investigation.
/// The seat gate at an ask-family call site (Seven Primes T-204, REQ-090, REQ-091): `ask_agent`, a session spawn or
/// turn, a jury seat, a blind validator. Decided before anything is spawned; a refusal comes back as the Refusal's
/// exact payload, which the caller returns as its tool or HTTP error text. `tty` is false: none of these surfaces runs
/// on the operator's terminal. The surface is the caller's, never inferred here.
pub fn gate_seat(
    seat: &str,
    surface: mcp_bridge::seat_gate::Surface,
    origin: shared_types::Origin,
) -> Result<(), String> {
    gate_trail::note(seat, surface, origin);
    mcp_bridge::seat_gate::seat_gate(seat, surface, origin, false).map_err(|r| r.message)
}

/// Which (seat, surface, origin) triples reached `gate_seat`, so a test can prove a request's `origin` arrived at
/// the gate (under A-2 no answer depends on origin, so the answer alone cannot show it). Recorded only while at
/// least one `Recording` is alive; otherwise `note` is one atomic load.
#[doc(hidden)]
pub mod gate_trail {
    use mcp_bridge::seat_gate::Surface;
    use shared_types::Origin;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    static CALLS: Mutex<Vec<(String, Surface, Origin)>> = Mutex::new(Vec::new());

    /// While this lives, gate calls are recorded. Tests that run in parallel share one trail, so assert on a
    /// triple your own test produced, not on the trail's length.
    pub struct Recording(());

    impl Drop for Recording {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Start recording. The first recorder clears what an earlier one left, so callers that serialise on their own
    /// lock see only their own calls.
    pub fn record() -> Recording {
        if ACTIVE.fetch_add(1, Ordering::SeqCst) == 0 {
            CALLS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
        }
        Recording(())
    }

    /// Every triple recorded since recording started, in call order.
    pub fn calls() -> Vec<(String, Surface, Origin)> {
        CALLS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn note(seat: &str, surface: Surface, origin: Origin) {
        if ACTIVE.load(Ordering::SeqCst) > 0 {
            CALLS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((seat.to_string(), surface, origin));
        }
    }

    /// True if this exact triple has reached the gate while recording.
    pub fn saw(seat: &str, surface: Surface, origin: Origin) -> bool {
        CALLS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|(s, su, o)| s == seat && *su == surface && *o == origin)
    }
}

pub(crate) fn describe_ask_agent_failure(err: &anyhow::Error) -> String {
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
    // Seven Primes T-204 (REQ-090): a seat the gate refuses on Session never gets a session or a worker.
    gate_seat(&agent, mcp_bridge::seat_gate::Surface::Session, req.origin)?;
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
            cli_session_id: None,
            phase: if is_new_seat(&agent) {
                SessionPhase::Pending
            } else {
                SessionPhase::Ready
            },
            broken_reason: None,
            generation: 0,
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

    let session_agent = {
        let mut sessions = sessions.lock().await;
        if !sessions.contains_key(&req.name) {
            core_reload_session_if_enabled(sessions_file, &req.name, &mut sessions)
                .map_err(|e| format!("failed to reload sessions: {e}"))?;
        }
        sessions
            .get(&req.name)
            .map(|state| normalize_agent_name(&state.agent))
            .ok_or_else(|| format!("session '{}' not found", req.name))?
    };
    if is_new_seat(&session_agent) {
        return ask_new_seat_session(sessions, sessions_file, req, execute_ask_agent).await;
    }

    let (agent, cwd, had_history, prior_cli_session) = {
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
            state.cli_session_id.clone(),
        )
    };

    let response = execute_ask_agent(
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
            prior_cli_session_id: prior_cli_session.clone(),
            // Carry the sight fields through. Without these a multi-turn review could not be
            // gated at all, because this construction discarded them via ..Default::default().
            required_sources: req.required_sources.clone(),
            require_sight: req.require_sight,
            // Same story as the sight fields: without this a named grok session could never
            // run Deep, so a session review could be gated on sources it had no turns to read.
            grok_depth: req.grok_depth,
            // Seven Primes T-204 (REQ-090): this turn is gated on the Session surface with the caller's origin.
            origin: req.origin,
            ask_surface: shared_types::AskSurface::Session,
            ..Default::default()
        },
        None,
    )
    .await
    .map_err(|e| format!("ask_session failed: {e}"))?;
    let new_cli_session_id = response.cli_session_id.clone();
    let mut response = response.response;

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
        // The session OWNS its CLI id, on BOTH surfaces. Persisting it only on the HTTP route
        // would recreate the exact drift that let the cross-session leak survive here.
        if new_cli_session_id.is_some() {
            state.cli_session_id = new_cli_session_id;
        }
    }
    core_persist_json_file_if_enabled(sessions_file, &*sessions)
        .map_err(|e| format!("failed to persist sessions: {e}"))?;

    Ok(response)
}

async fn ask_new_seat_session(
    sessions: &Sessions,
    sessions_file: Option<&PathBuf>,
    req: &AskSessionRequest,
    execute_ask_agent: ExecuteAskAgentFn,
) -> Result<String, String> {
    let home = sessions_file
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .map(Ok)
        .unwrap_or_else(triumvirate_home_dir)
        .map_err(|e| format!("session_lock: {e}"))?;
    let _session_guard = acquire_session_lock(&home, &req.name, session_busy_wait())
        .await
        .map_err(|e| {
            if e == "session_busy" {
                format!("session_busy: {}", req.name)
            } else {
                e
            }
        })?;

    let (agent, cwd, generation, prior_cli_session) = {
        let mut all = sessions.lock().await;
        core_reload_session_if_enabled(sessions_file, &req.name, &mut all)
            .map_err(|e| format!("failed to reload sessions: {e}"))?;
        let state = all
            .get(&req.name)
            .ok_or_else(|| format!("session '{}' not found", req.name))?;
        if matches!(state.phase, SessionPhase::Broken) {
            return Err(format!(
                "session_broken: {}; dismiss_session and spawn a new one",
                state.broken_reason.as_deref().unwrap_or("unknown")
            ));
        }
        (
            normalize_agent_name(&state.agent),
            state.cwd.clone(),
            state.generation,
            if matches!(state.phase, SessionPhase::Ready) {
                state.cli_session_id.clone()
            } else {
                None
            },
        )
    };

    let turn = execute_ask_agent(
        &AskAgentRequest {
            agent: agent.clone(),
            message: req.message.clone(),
            cwd,
            repo: None,
            branch: None,
            reuse_session: Some(true),
            session_key: Some(req.name.clone()),
            prior_cli_session_id: prior_cli_session,
            required_sources: req.required_sources.clone(),
            require_sight: req.require_sight,
            grok_depth: req.grok_depth,
            origin: req.origin,
            ask_surface: shared_types::AskSurface::Session,
            ..Default::default()
        },
        None,
    )
    .await;

    let mut all = sessions.lock().await;
    let state = all
        .get_mut(&req.name)
        .ok_or_else(|| format!("session '{}' not found", req.name))?;
    let publish = match &turn {
        Ok(response) => daemon_core::publish_turn(
            state,
            generation,
            &agent,
            TurnOutcome::Success {
                cli_session_id: response.cli_session_id.as_deref(),
            },
        ),
        Err(error) => {
            daemon_core::publish_turn(state, generation, &agent, TurnOutcome::Failure(error))
        }
    };
    if matches!(publish, PublishResult::Conflict) {
        tracing::warn!(
            name = %req.name,
            start_generation = generation,
            stored_generation = state.generation,
            "session_generation_conflict"
        );
    }
    let result = match turn {
        Ok(response) => {
            let mut answer = response.response;
            if let PublishResult::NotResumable(reason) = publish {
                answer.push_str(&format!("\n(session not resumable: {reason})"));
            }
            state.history.push(format!("user: {}", req.message));
            state.history.push(format!("assistant: {answer}"));
            Ok(answer)
        }
        Err(error) => Err(format!("ask_session failed: {error}")),
    };
    core_persist_json_file_if_enabled(sessions_file, &*all)
        .map_err(|e| format!("failed to persist sessions: {e}"))?;
    result
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

/// Seven Primes T-203 (REQ-080, REQ-091): every seat that needs `doctor --live` verification, with its state read NOW
/// (configured from its env flag, verified/suspended from `$TRIUMVIRATE_HOME/seats/<seat>/`), and `reason` set to the
/// exact refusal the gate would give an ask.
pub fn seats_status() -> std::collections::BTreeMap<String, shared_types::SeatStatus> {
    use mcp_bridge::seat_profile::{Configured, VerifyPolicy, all_seats};
    let home = mcp_bridge::seat_state::triumvirate_home().ok();
    all_seats()
        .iter()
        .filter(|row| row.verify_policy == VerifyPolicy::DoctorLive)
        .map(|row| {
            let configured = match row.configured {
                Configured::Always => true,
                Configured::EnvFlag(var) => std::env::var(var).is_ok_and(|v| v == "1"),
            };
            let version = mcp_bridge::seat_gate::installed_cli_version(row.key);
            let state = home
                .as_deref()
                .and_then(|h| {
                    mcp_bridge::seat_state::availability(h, row.key, configured, version.as_deref())
                        .ok()
                })
                .unwrap_or(mcp_bridge::seat_state::SeatAvailability {
                    configured,
                    verified: false,
                    suspended: None,
                });
            let reason = mcp_bridge::seat_gate::seat_gate_with(
                row.key,
                mcp_bridge::seat_gate::Surface::AskAgent,
                shared_types::Origin::Interactive,
                true,
                &state,
            )
            .err()
            .map(|refusal| refusal.message);
            let status = shared_types::SeatStatus {
                configured: state.configured,
                verified: state.verified,
                suspended: state.suspended.is_some(),
                available: state.available(),
                reason,
            };
            (row.key.to_string(), status)
        })
        .collect()
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
            seats: snapshot.seats.unwrap_or_else(seats_status),
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
        seats: seats_status(),
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ACTIVE_NEW_SEAT_TURNS: AtomicUsize = AtomicUsize::new(0);
    static MAX_ACTIVE_NEW_SEAT_TURNS: AtomicUsize = AtomicUsize::new(0);
    static NEW_SEAT_CALLS: AtomicUsize = AtomicUsize::new(0);
    static NEW_SEAT_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    fn overlapping_probe_executor<'a>(
        req: &'a AskAgentRequest,
        _emitter: Option<ProgressEmitter>,
    ) -> Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>> {
        Box::pin(async move {
            NEW_SEAT_CALLS.fetch_add(1, Ordering::SeqCst);
            let active = ACTIVE_NEW_SEAT_TURNS.fetch_add(1, Ordering::SeqCst) + 1;
            MAX_ACTIVE_NEW_SEAT_TURNS.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(100)).await;
            ACTIVE_NEW_SEAT_TURNS.fetch_sub(1, Ordering::SeqCst);
            let mut response = AskAgentResponse::direct(
                "request".to_string(),
                req.agent.clone(),
                "answer".to_string(),
                Vec::new(),
            );
            response.cli_session_id = Some("vendor-id".to_string());
            Ok(response)
        })
    }

    fn no_id_executor<'a>(
        req: &'a AskAgentRequest,
        _emitter: Option<ProgressEmitter>,
    ) -> Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>> {
        Box::pin(async move {
            NEW_SEAT_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(AskAgentResponse::direct(
                "request".to_string(),
                req.agent.clone(),
                "answer".to_string(),
                Vec::new(),
            ))
        })
    }

    fn slow_executor<'a>(
        req: &'a AskAgentRequest,
        _emitter: Option<ProgressEmitter>,
    ) -> Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>> {
        Box::pin(async move {
            NEW_SEAT_CALLS.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(3)).await;
            let mut response = AskAgentResponse::direct(
                "request".to_string(),
                req.agent.clone(),
                "answer".to_string(),
                Vec::new(),
            );
            response.cli_session_id = Some("vendor-id".to_string());
            Ok(response)
        })
    }

    fn pending_session(agent: &str) -> SessionState {
        SessionState {
            agent: agent.to_string(),
            cwd: None,
            history: Vec::new(),
            cli_session_id: None,
            phase: SessionPhase::Pending,
            broken_reason: None,
            generation: 0,
            parent_session_id: None,
            root_session_id: None,
            pantheon_session_id: None,
        }
    }

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
        assert!(
            message.contains("TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS"),
            "{message}"
        );
    }

    /// RED IF: two in-process asks for one new-seat session overlap child lifetimes.
    #[tokio::test]
    async fn new_seat_session_asks_are_serialized() {
        let _test_guard = NEW_SEAT_TEST_LOCK.lock().await;
        ACTIVE_NEW_SEAT_TURNS.store(0, Ordering::SeqCst);
        MAX_ACTIVE_NEW_SEAT_TURNS.store(0, Ordering::SeqCst);
        NEW_SEAT_CALLS.store(0, Ordering::SeqCst);
        let dir = tempfile::tempdir().unwrap();
        let file = daemon_core::sessions_file_path(dir.path());
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::from([(
            "same".to_string(),
            pending_session("kimi"),
        )])));
        core_persist_json_file_if_enabled(Some(&file), &*sessions.lock().await).unwrap();
        let req = AskSessionRequest {
            name: "same".to_string(),
            message: "turn".to_string(),
            ..Default::default()
        };
        let (first, second) = tokio::join!(
            ask_session(
                &sessions,
                Some(&file),
                &req,
                false,
                overlapping_probe_executor
            ),
            ask_session(
                &sessions,
                Some(&file),
                &req,
                false,
                overlapping_probe_executor
            )
        );
        assert!(first.is_ok() && second.is_ok(), "{first:?} {second:?}");
        assert_eq!(MAX_ACTIVE_NEW_SEAT_TURNS.load(Ordering::SeqCst), 1);
        assert_eq!(NEW_SEAT_CALLS.load(Ordering::SeqCst), 2);
    }

    /// RED IF: an in-process contender waits past its configured lock limit or invokes its executor.
    #[tokio::test]
    async fn in_process_lock_timeout_returns_session_busy_without_second_spawn() {
        let _test_guard = NEW_SEAT_TEST_LOCK.lock().await;
        NEW_SEAT_CALLS.store(0, Ordering::SeqCst);
        let prior = std::env::var_os("TRIUMVIRATE_SESSION_BUSY_SECS");
        unsafe { std::env::set_var("TRIUMVIRATE_SESSION_BUSY_SECS", "1") };
        let dir = tempfile::tempdir().unwrap();
        let file = daemon_core::sessions_file_path(dir.path());
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::from([(
            "busy".to_string(),
            pending_session("kimi"),
        )])));
        core_persist_json_file_if_enabled(Some(&file), &*sessions.lock().await).unwrap();
        let req = AskSessionRequest {
            name: "busy".to_string(),
            message: "turn".to_string(),
            ..Default::default()
        };
        let first = ask_session(&sessions, Some(&file), &req, false, slow_executor);
        let second = async {
            while NEW_SEAT_CALLS.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            ask_session(&sessions, Some(&file), &req, false, slow_executor).await
        };
        let (first, second) = tokio::join!(first, second);
        match prior {
            Some(value) => unsafe { std::env::set_var("TRIUMVIRATE_SESSION_BUSY_SECS", value) },
            None => unsafe { std::env::remove_var("TRIUMVIRATE_SESSION_BUSY_SECS") },
        }
        assert!(first.is_ok(), "{first:?}");
        assert_eq!(second.unwrap_err(), "session_busy: busy");
        assert_eq!(NEW_SEAT_CALLS.load(Ordering::SeqCst), 1);
    }

    /// RED IF: a first successful turn without an id stays usable or spawns again.
    #[tokio::test]
    async fn each_new_seat_missing_first_id_breaks_session_and_blocks_next_spawn() {
        let _test_guard = NEW_SEAT_TEST_LOCK.lock().await;
        for seat in ["kimi", "mistral", "qwen"] {
            NEW_SEAT_CALLS.store(0, Ordering::SeqCst);
            let dir = tempfile::tempdir().unwrap();
            let file = daemon_core::sessions_file_path(dir.path());
            let sessions: Sessions = Arc::new(Mutex::new(HashMap::from([(
                "missing".to_string(),
                pending_session(seat),
            )])));
            core_persist_json_file_if_enabled(Some(&file), &*sessions.lock().await).unwrap();
            let req = AskSessionRequest {
                name: "missing".to_string(),
                message: "turn".to_string(),
                ..Default::default()
            };
            let answer = ask_session(&sessions, Some(&file), &req, false, no_id_executor)
                .await
                .unwrap();
            assert!(answer.contains(&format!("session not resumable: missing_session_id:{seat}")));
            let error = ask_session(&sessions, Some(&file), &req, false, no_id_executor)
                .await
                .unwrap_err();
            assert_eq!(
                error,
                format!(
                    "session_broken: missing_session_id:{seat}; dismiss_session and spawn a new one"
                )
            );
            assert_eq!(NEW_SEAT_CALLS.load(Ordering::SeqCst), 1, "{seat}");
        }
    }

    /// RED IF: in-process spawn does not create and persist a Pending new-seat record before any ask.
    #[tokio::test]
    async fn in_process_spawn_persists_pending_before_first_ask() {
        let _test_guard = NEW_SEAT_TEST_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("empty-cwd");
        std::fs::create_dir(&cwd).unwrap();
        let file = daemon_core::sessions_file_path(dir.path());
        let prior_home = std::env::var_os("TRIUMVIRATE_HOME");
        let prior_enable = std::env::var_os("TRIUMVIRATE_ENABLE_KIMI");
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", dir.path());
            std::env::set_var("TRIUMVIRATE_ENABLE_KIMI", "1");
        }
        mcp_bridge::seat_state::write_verified(
            dir.path(),
            "kimi",
            &mcp_bridge::seat_state::VerifiedRecord {
                cli_version: "2.2.0".to_string(),
                seat_home: dir.path().join("homes/kimi").display().to_string(),
                probed_at: "2026-10-19T00:00:00Z".to_string(),
                served_model: None,
            },
        )
        .unwrap();
        mcp_bridge::seat_gate::register_installed_cli_version("kimi", "2.2.0");
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let result = spawn_session(
            &sessions,
            Some(&file),
            &SpawnSessionRequest {
                agent: "kimi".to_string(),
                name: "pending".to_string(),
                cwd: Some(cwd.display().to_string()),
                ..Default::default()
            },
            false,
        )
        .await;
        match prior_home {
            Some(value) => unsafe { std::env::set_var("TRIUMVIRATE_HOME", value) },
            None => unsafe { std::env::remove_var("TRIUMVIRATE_HOME") },
        }
        match prior_enable {
            Some(value) => unsafe { std::env::set_var("TRIUMVIRATE_ENABLE_KIMI", value) },
            None => unsafe { std::env::remove_var("TRIUMVIRATE_ENABLE_KIMI") },
        }
        result.unwrap();
        let state = sessions.lock().await.get("pending").cloned().unwrap();
        assert_eq!(state.phase, SessionPhase::Pending);
        assert_eq!(state.generation, 0);
        let disk: HashMap<String, SessionState> = daemon_core::load_json_file(&file).unwrap();
        assert_eq!(disk["pending"].phase, SessionPhase::Pending);
    }

    /// RED IF: a Codex session enters the new-seat state machine or creates a session lock file.
    #[tokio::test]
    async fn codex_session_keeps_legacy_path_without_phase_or_lock_change() {
        let _test_guard = NEW_SEAT_TEST_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let file = daemon_core::sessions_file_path(dir.path());
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::from([(
            "legacy".to_string(),
            SessionState {
                agent: "codex".to_string(),
                cwd: Some(dir.path().join("empty-cwd").display().to_string()),
                history: Vec::new(),
                cli_session_id: None,
                phase: SessionPhase::Ready,
                broken_reason: None,
                generation: 0,
                parent_session_id: None,
                root_session_id: None,
                pantheon_session_id: None,
            },
        )])));
        std::fs::create_dir(dir.path().join("empty-cwd")).unwrap();
        core_persist_json_file_if_enabled(Some(&file), &*sessions.lock().await).unwrap();
        let answer = ask_session(
            &sessions,
            Some(&file),
            &AskSessionRequest {
                name: "legacy".to_string(),
                message: "turn".to_string(),
                ..Default::default()
            },
            false,
            no_id_executor,
        )
        .await
        .unwrap();
        assert_eq!(answer, "answer");
        let state = sessions.lock().await.get("legacy").cloned().unwrap();
        assert_eq!(state.phase, SessionPhase::Ready);
        assert_eq!(state.generation, 0);
        assert!(!dir.path().join("locks").exists());
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

    /// The CLI session id must be persisted on BOTH surfaces. Persisting it only on the HTTP
    /// route would recreate the exact drift that let the cross-session leak survive on this one.
    #[test]
    fn u_ia_both_surfaces_persist_the_session_owned_cli_id() {
        let needle = format!("state.{} = ", "cli_session_id");
        let mcp = include_str!("inter_agent.rs");
        assert!(
            mcp.contains(needle.as_str()),
            "the in-process MCP route must persist the session-owned CLI id"
        );
        // And it must READ it back, or the next turn falls through to the legacy worker registry.
        assert!(
            mcp.contains("prior_cli_session_id"),
            "the MCP route must pass the session-owned id back in as authoritative"
        );
    }

}

#[cfg(test)]
mod session_sight_tests {
    use super::*;
    use std::sync::OnceLock;

    /// Captures the `AskAgentRequest` that `ask_session` actually built.
    ///
    /// A function pointer cannot close over state, so the capture is a static. That is fine
    /// here: this is the only test using it, and it asserts on the whole request.
    static CAPTURED: OnceLock<std::sync::Mutex<Option<AskAgentRequest>>> = OnceLock::new();

    fn captured() -> &'static std::sync::Mutex<Option<AskAgentRequest>> {
        CAPTURED.get_or_init(|| std::sync::Mutex::new(None))
    }

    fn capturing_executor<'a>(
        req: &'a AskAgentRequest,
        _p: Option<ProgressEmitter>,
    ) -> Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>> {
        *captured().lock().unwrap() = Some(req.clone());
        Box::pin(async move {
            Ok(AskAgentResponse::direct(
                "req-1".to_string(),
                req.agent.clone(),
                "ok".to_string(),
                Vec::new(),
            ))
        })
    }

    /// A multi-turn review must be gateable.
    ///
    /// Before this, `ask_session` built `AskAgentRequest { ..Default::default() }`, so neither
    /// sight field could ever reach the dispatcher and a session review could not be gated at
    /// all. Codex found that in its route survey. This asserts the fields actually arrive,
    /// rather than that the struct has them.
    ///
    /// RED IF: `ask_session` stops forwarding `required_sources`, `require_sight` or
    /// `grok_depth`, which would silently return session reviews to being ungated (or to
    /// Fast, which cannot read the gated sources) while every other test passed.
    #[tokio::test]
    async fn ask_session_forwards_the_sight_fields_to_dispatch() {
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        sessions.lock().await.insert(
            "reviewer".to_string(),
            SessionState {
                agent: "grok".to_string(),
                cwd: Some("/repo".to_string()),
                history: Vec::new(),
                cli_session_id: None,
                phase: SessionPhase::Ready,
                broken_reason: None,
                generation: 0,
                parent_session_id: None,
                root_session_id: None,
                pantheon_session_id: None,
            },
        );

        let req = AskSessionRequest {
            name: "reviewer".to_string(),
            message: "review it".to_string(),
            required_sources: vec!["/repo/a.rs".to_string()],
            require_sight: Some(true),
            grok_depth: Some(shared_types::GrokDepthOverride::Deep),
            ..Default::default()
        };
        ask_session(&sessions, None, &req, false, capturing_executor)
            .await
            .expect("ask_session should dispatch");

        let got = captured().lock().unwrap().clone().expect("executor ran");
        assert_eq!(
            got.required_sources,
            vec!["/repo/a.rs".to_string()],
            "named sources must reach the dispatcher, or a session review is silently ungated"
        );
        assert_eq!(got.require_sight, Some(true));
        assert_eq!(
            got.grok_depth,
            Some(shared_types::GrokDepthOverride::Deep),
            "depth must reach the dispatcher, or a named grok session is stuck at Fast"
        );
        // The session identity must still be carried, or this fix would trade one leak for
        // another: session_key is what stops two named sessions resuming each other.
        assert_eq!(got.session_key.as_deref(), Some("reviewer"));
    }

    /// Seven Primes T-204: a session turn reaches dispatch on the Session surface with the turn's own origin, for
    /// both origins. RED IF: ask_session drops `origin` or stops naming the Session surface.
    fn origin_capturing_executor<'a>(
        req: &'a AskAgentRequest,
        _p: Option<ProgressEmitter>,
    ) -> Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>> {
        ORIGIN_SEEN
            .lock()
            .unwrap()
            .push((req.origin, req.ask_surface));
        Box::pin(async move {
            Ok(AskAgentResponse::direct(
                "req-1".to_string(),
                req.agent.clone(),
                "ok".to_string(),
                Vec::new(),
            ))
        })
    }
    static ORIGIN_SEEN: std::sync::Mutex<Vec<(shared_types::Origin, shared_types::AskSurface)>> =
        std::sync::Mutex::new(Vec::new());

    #[tokio::test]
    async fn ask_session_forwards_origin_and_the_session_surface() {
        for origin in [
            shared_types::Origin::Interactive,
            shared_types::Origin::Unattended,
        ] {
            let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
            sessions.lock().await.insert(
                "s".to_string(),
                SessionState {
                    agent: "codex".to_string(),
                    cwd: Some("/repo".to_string()),
                    history: Vec::new(),
                    cli_session_id: None,
                    phase: SessionPhase::Ready,
                    broken_reason: None,
                    generation: 0,
                    parent_session_id: None,
                    root_session_id: None,
                    pantheon_session_id: None,
                },
            );
            let req = AskSessionRequest {
                name: "s".to_string(),
                message: "m".to_string(),
                origin,
                ..Default::default()
            };
            ask_session(&sessions, None, &req, false, origin_capturing_executor)
                .await
                .expect("dispatch");
        }
        assert_eq!(
            *ORIGIN_SEEN.lock().unwrap(),
            [
                (
                    shared_types::Origin::Interactive,
                    shared_types::AskSurface::Session
                ),
                (
                    shared_types::Origin::Unattended,
                    shared_types::AskSurface::Session
                ),
            ]
        );
    }

    /// Seven Primes T-204 (REQ-090): spawn_session gates the seat on Session with the request's origin BEFORE any
    /// session or worker exists. An unconfigured kimi is refused with the gate's exact payload and leaves no session;
    /// an existing seat spawns for either origin; and the trail shows the unattended origin reached the gate.
    /// RED IF: spawn_session stops calling the gate, gates after creating the session, or drops `origin`.
    #[tokio::test]
    async fn spawn_session_gates_before_creating_anything() {
        let _rec = gate_trail::record();
        let prior = {
            let _env = crate::PROCESS_ENV_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let prior = std::env::var_os("TRIUMVIRATE_ENABLE_KIMI");
            // SAFETY: under PROCESS_ENV_LOCK; restored at the end of this test.
            unsafe { std::env::remove_var("TRIUMVIRATE_ENABLE_KIMI") };
            prior
        };
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let spawn = |agent: &str, name: &str, origin| SpawnSessionRequest {
            agent: agent.to_string(),
            name: name.to_string(),
            cwd: Some("/tmp/t204-spawn".to_string()),
            origin,
        };
        let err = spawn_session(
            &sessions,
            None,
            &spawn("kimi", "k", shared_types::Origin::Unattended),
            false,
        )
        .await
        .expect_err("an unconfigured seat is refused");
        assert_eq!(
            err,
            "seat_not_configured: kimi: set TRIUMVIRATE_ENABLE_KIMI=1 and restart the daemon"
        );
        assert!(
            sessions.lock().await.is_empty(),
            "a refused spawn leaves no session"
        );
        assert!(gate_trail::saw(
            "kimi",
            mcp_bridge::seat_gate::Surface::Session,
            shared_types::Origin::Unattended
        ));
        for (name, origin) in [
            ("c1", shared_types::Origin::Interactive),
            ("c2", shared_types::Origin::Unattended),
        ] {
            spawn_session(&sessions, None, &spawn("codex", name, origin), false)
                .await
                .expect("an existing seat spawns");
            assert!(gate_trail::saw(
                "codex",
                mcp_bridge::seat_gate::Surface::Session,
                origin
            ));
        }
        assert_eq!(sessions.lock().await.len(), 2);
        if let Some(v) = prior {
            let _env = crate::PROCESS_ENV_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // SAFETY: under PROCESS_ENV_LOCK.
            unsafe { std::env::set_var("TRIUMVIRATE_ENABLE_KIMI", v) };
        }
    }
}
