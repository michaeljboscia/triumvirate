use clap::{Parser, Subcommand};
use agent_worker::{
    WorkerAcquireMode, acquire_worker, dismiss_worker, should_invalidate_cached_session,
    update_worker_session,
};
#[cfg(test)]
use agent_worker::reset_worker_registry_for_tests;
use daemon_core::{
    QueueRegistry,
    acquire_project_queue as core_acquire_project_queue,
    append_memory_entry as core_append_memory_entry,
    daemon_bind_addr as core_daemon_bind_addr,
    launchd_plist_path as core_launchd_plist_path,
    render_launch_agent_plist as core_render_launch_agent_plist,
    list_scratchpad as core_list_scratchpad, project_queue_key as core_project_queue_key,
    read_memory_entries as core_read_memory_entries,
    resolve_context as core_resolve_context, triumvirate_home_dir as core_triumvirate_home_dir,
    unix_time_ms as core_unix_time_ms, write_scratchpad as core_write_scratchpad,
    ensure_daemon_token as core_ensure_daemon_token,
    sessions_file_path as core_sessions_file_path,
    load_json_file_if_exists as core_load_json_file_if_exists,
    persist_json_file_if_enabled as core_persist_json_file_if_enabled,
};
use daemon_http::{
    attempt_daemon_autostart_once, fetch_daemon_ask_agent, fetch_daemon_fallback_ack,
    fetch_daemon_fallback_gc, fetch_daemon_fallback_list, fetch_daemon_memory_read,
    fetch_daemon_memory_write, fetch_daemon_outbox_recent, fetch_daemon_scratchpad_list,
    fetch_daemon_scratchpad_write, fetch_daemon_session_ask, fetch_daemon_session_dismiss,
    fetch_daemon_session_list, fetch_daemon_session_spawn, fetch_daemon_status,
    fetch_daemon_status_snapshot, reset_daemon_autostart_flag_for_tests,
};
use fallback_outbox::{
    acknowledge_fallback_path, append_outbox_event, count_pending_fallbacks, gc_fallbacks,
    list_pending_fallback_paths, read_outbox_events, spawn_dead_drop as create_dead_drop_fallback,
};
use mcp_bridge::{
    daemon_base_url,
    codex_command,
    daemon_status_url, gemini_command,
    is_bearer_authorized, is_supported_agent, is_supported_agent_name, should_use_daemon_proxy,
};
use mcp_tools::{ProgressEmitter, display_agent_name, next_heartbeat_offset};
use axum::{
    Json as AxumJson, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use prometheus::{Encoder, HistogramVec, IntCounterVec, IntGauge, Registry, TextEncoder};
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use shared_types::{
    AskAgentRequest, AskAgentResponse, AskSessionRequest,
    DismissSessionRequest,
    FallbackAckRequest, FallbackGcRequest, FallbackGcResponse, FallbackListRequest,
    FallbackListResponse, LifecycleEvent, MemoryEntry, MemoryReadRequest, MemoryReadResponse,
    MemoryWriteRequest, MemoryWriteResponse, OutboxEvent, OutboxRecentRequest,
    OutboxRecentResponse, SessionInfo, SessionListResponse, SpawnSessionRequest,
    ScratchpadListRequest, ScratchpadListResponse, ScratchpadWriteRequest,
    ScratchpadWriteResponse, StatusResponse, DaemonHealthResponse, DaemonStatusSnapshot,
    SessionState,
};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
    },
};
use tokio::{
    process::Command,
    sync::Mutex,
    time::{Duration, Instant, sleep, timeout},
};
#[cfg(test)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "triumvirate")]
#[command(about = "Triumvirate v2 daemon + MCP bridge binary")]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Run the MCP stdio bridge.
    Mcp,
    /// Run the long-lived daemon (stub in Increment 1a).
    Daemon,
    /// Install launchd configuration for zero-ceremony daemon startup.
    Install,
    /// Remove launchd configuration for daemon startup.
    Uninstall,
    /// Print daemon health and status snapshot.
    Status,
    /// Run local diagnostics for daemon readiness.
    Doctor,
}

#[derive(Debug, Clone)]
struct McpBridge {
    tool_router: ToolRouter<Self>,
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    sessions_file: Option<PathBuf>,
}

fn mcp_daemon_proxy_enabled() -> bool {
    #[cfg(test)]
    {
        return should_use_daemon_proxy(std::env::var("TRIUMVIRATE_MCP_USE_DAEMON").ok().as_deref());
    }
    #[cfg(not(test))]
    {
        return use_daemon_for_mcp_from_env();
    }
}

impl McpBridge {
    fn new() -> Self {
        Self::with_persistence(true)
    }

    #[cfg(test)]
    fn new_ephemeral() -> Self {
        Self::with_persistence(false)
    }

    fn with_persistence(enable_persistence: bool) -> Self {
        let sessions_file = if enable_persistence {
            core_triumvirate_home_dir()
                .ok()
                .map(|home| core_sessions_file_path(&home))
        } else {
            None
        };
        // Load persisted sessions on startup so sessions survive MCP bridge restarts.
        let sessions = sessions_file
            .as_ref()
            .and_then(|path| core_load_json_file_if_exists::<HashMap<String, SessionState>>(path).ok())
            .unwrap_or_default();
        Self {
            tool_router: Self::tool_router(),
            sessions: Arc::new(Mutex::new(sessions)),
            sessions_file,
        }
    }
}

#[tool_router]
impl McpBridge {
    #[tool(description = "Health check tool for MCP connectivity")]
    async fn ping(&self) -> String {
        "pong".to_string()
    }

    #[tool(description = "Send a task to a specific agent (Increment 1b supports gemini mock path).")]
    async fn ask_agent(
        &self,
        Parameters(req): Parameters<AskAgentRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<AskAgentResponse>, String> {
        let emitter = ProgressEmitter::from_context(&context);
        if mcp_daemon_proxy_enabled() {
            let display = display_agent_name(&req.agent);
            emitter.emit(format!("→ {display}: sent ✓")).await;
            let mut pending = Box::pin(fetch_daemon_ask_agent(&req));
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
                                return Err(format!("ask_agent via daemon failed: {err}"));
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

        execute_ask_agent(&req, Some(emitter))
            .await
            .map(Json)
    }

    #[tool(description = "Create a persistent named session for an agent.")]
    async fn spawn_session(
        &self,
        Parameters(req): Parameters<SpawnSessionRequest>,
    ) -> Result<String, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_session_spawn(&req)
                .await
                .map_err(|e| format!("spawn_session via daemon failed: {e}"));
        }
        let agent = req.agent.to_lowercase();
        if !is_supported_agent_name(&agent) {
            return Err("spawn_session supports only 'gemini' or 'codex'".to_string());
        }
        let cwd = req.cwd.clone().unwrap_or_else(|| ".".to_string());
        let worker = acquire_worker(&agent, &cwd).await;

        let mut sessions = self.sessions.lock().await;
        sessions.insert(
            req.name.clone(),
            SessionState {
                agent: agent.clone(),
                cwd: Some(cwd),
                history: Vec::new(),
            },
        );
        core_persist_json_file_if_enabled(self.sessions_file.as_ref(), &*sessions)
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

    #[tool(description = "Ask within a named persistent session.")]
    async fn ask_session(
        &self,
        Parameters(req): Parameters<AskSessionRequest>,
    ) -> Result<String, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_session_ask(&req)
                .await
                .map_err(|e| format!("ask_session via daemon failed: {e}"));
        }

        let (agent, cwd) = {
            let mut sessions = self.sessions.lock().await;
            let state = sessions
                .get_mut(&req.name)
                .ok_or_else(|| format!("session '{}' not found", req.name))?;
            (state.agent.clone(), state.cwd.clone())
        };

        let response = execute_ask_agent(
            &AskAgentRequest {
                agent: agent.clone(),
                message: req.message.clone(),
                cwd: cwd.clone(),
                repo: None,
                branch: None,
            },
            None,
        )
        .await
        .map_err(|e| format!("ask_session failed: {e}"))?
        .response;

        let mut sessions = self.sessions.lock().await;
        if let Some(state) = sessions.get_mut(&req.name) {
            state.history.push(format!("user: {}", req.message));
            state.history.push(format!("assistant: {response}"));
        }
        core_persist_json_file_if_enabled(self.sessions_file.as_ref(), &*sessions)
            .map_err(|e| format!("failed to persist sessions: {e}"))?;

        Ok(response)
    }

    #[tool(description = "Dismiss a named session.")]
    async fn dismiss_session(
        &self,
        Parameters(req): Parameters<DismissSessionRequest>,
    ) -> Result<String, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_session_dismiss(&req)
                .await
                .map_err(|e| format!("dismiss_session via daemon failed: {e}"));
        }
        let mut sessions = self.sessions.lock().await;
        match sessions.remove(&req.name) {
            Some(removed_session) => {
                let should_drop_worker = !sessions.values().any(|s| {
                    s.agent == removed_session.agent && s.cwd == removed_session.cwd
                });
                if should_drop_worker {
                    let cwd = removed_session.cwd.unwrap_or_else(|| ".".to_string());
                    let _ = dismiss_worker(&removed_session.agent, &cwd).await;
                }
                core_persist_json_file_if_enabled(self.sessions_file.as_ref(), &*sessions)
                    .map_err(|e| format!("failed to persist sessions: {e}"))?;
                Ok(format!("session '{}' dismissed", req.name))
            }
            None => Err(format!("session '{}' not found", req.name)),
        }
    }

    #[tool(description = "List active sessions.")]
    async fn list_sessions(&self) -> Json<SessionListResponse> {
        if mcp_daemon_proxy_enabled()
            && let Ok(response) = fetch_daemon_session_list().await
        {
            return Json(response);
        }
        let sessions = self.sessions.lock().await;
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

    #[tool(description = "Get current system status snapshot.")]
    async fn get_status(&self) -> Json<StatusResponse> {
        let sessions = self.sessions.lock().await;
        let local_bind_addr =
            core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());
        if mcp_daemon_proxy_enabled()
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
            supported_agents: vec!["gemini".to_string(), "codex".to_string()],
            pending_fallbacks,
            fallback_tickets: fallback_tickets
                .into_iter()
                .map(|p| p.display().to_string())
                .collect(),
            daemon_bind_addr: local_bind_addr,
        })
    }

    #[tool(description = "Query daemon HTTP status using local bearer token.")]
    async fn daemon_health(&self) -> Result<Json<DaemonHealthResponse>, String> {
        fetch_daemon_status()
            .await
            .map(Json)
            .map_err(|e| format!("daemon health query failed: {e}"))
    }

    #[tool(description = "Write a shared memory entry.")]
    async fn memory_write(
        &self,
        Parameters(req): Parameters<MemoryWriteRequest>,
    ) -> Result<Json<MemoryWriteResponse>, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_memory_write(&req)
                .await
                .map(Json)
                .map_err(|e| format!("memory_write via daemon failed: {e}"));
        }
        let id = Uuid::new_v4().to_string();
        let entry = MemoryEntry {
            id: id.clone(),
            namespace: req.namespace,
            key: req.key,
            value: req.value,
            ts_ms: core_unix_time_ms(),
        };
        append_memory_entry(&entry).map_err(|e| format!("memory_write failed: {e}"))?;
        Ok(Json(MemoryWriteResponse {
            id,
            status: "ok".to_string(),
        }))
    }

    #[tool(description = "Read shared memory entries.")]
    async fn memory_read(
        &self,
        Parameters(req): Parameters<MemoryReadRequest>,
    ) -> Result<Json<MemoryReadResponse>, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_memory_read(&req)
                .await
                .map(Json)
                .map_err(|e| format!("memory_read via daemon failed: {e}"));
        }
        let mut entries =
            read_memory_entries().map_err(|e| format!("memory_read failed: {e}"))?;
        entries.retain(|e| e.namespace == req.namespace);
        if let Some(key) = req.key {
            entries.retain(|e| e.key == key);
        }
        entries.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
        if let Some(limit) = req.limit {
            entries.truncate(limit);
        }
        Ok(Json(MemoryReadResponse { entries }))
    }

    #[tool(description = "Write a scratchpad file in the shared workspace.")]
    async fn scratchpad_write(
        &self,
        Parameters(req): Parameters<ScratchpadWriteRequest>,
    ) -> Result<Json<ScratchpadWriteResponse>, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_scratchpad_write(&req)
                .await
                .map(Json)
                .map_err(|e| format!("scratchpad_write via daemon failed: {e}"));
        }
        let path = write_scratchpad(&req.project, &req.topic, &req.content)
            .map_err(|e| format!("scratchpad_write failed: {e}"))?;
        Ok(Json(ScratchpadWriteResponse {
            path: path.display().to_string(),
        }))
    }

    #[tool(description = "List scratchpad files for a project.")]
    async fn scratchpad_list(
        &self,
        Parameters(req): Parameters<ScratchpadListRequest>,
    ) -> Result<Json<ScratchpadListResponse>, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_scratchpad_list(&req)
                .await
                .map(Json)
                .map_err(|e| format!("scratchpad_list via daemon failed: {e}"));
        }
        let files = list_scratchpad(&req.project)
            .map_err(|e| format!("scratchpad_list failed: {e}"))?
            .into_iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        Ok(Json(ScratchpadListResponse { files }))
    }

    #[tool(description = "Read recent outbox lifecycle events.")]
    async fn outbox_recent(
        &self,
        Parameters(req): Parameters<OutboxRecentRequest>,
    ) -> Result<Json<OutboxRecentResponse>, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_outbox_recent(&req)
                .await
                .map(Json)
                .map_err(|e| format!("outbox_recent via daemon failed: {e}"));
        }
        let mut events = read_outbox_events().map_err(|e| format!("outbox_recent failed: {e}"))?;
        events.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
        events.truncate(req.limit.unwrap_or(50));
        Ok(Json(OutboxRecentResponse { events }))
    }

    #[tool(description = "List pending dead-drop fallback tickets.")]
    async fn fallback_list(
        &self,
        Parameters(req): Parameters<FallbackListRequest>,
    ) -> Result<Json<FallbackListResponse>, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_fallback_list(&req)
                .await
                .map(Json)
                .map_err(|e| format!("fallback_list via daemon failed: {e}"));
        }
        let tickets = list_pending_fallback_paths(req.limit.unwrap_or(20))
            .map_err(|e| format!("fallback_list failed: {e}"))?
            .into_iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        Ok(Json(FallbackListResponse { tickets }))
    }

    #[tool(description = "Acknowledge a dead-drop fallback ticket by deleting it.")]
    async fn fallback_ack(&self, Parameters(req): Parameters<FallbackAckRequest>) -> Result<String, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_fallback_ack(&req)
                .await
                .map_err(|e| format!("fallback_ack via daemon failed: {e}"));
        }
        acknowledge_fallback_path(&req.path).map_err(|e| format!("fallback_ack failed: {e}"))?;
        Ok(format!("acknowledged {}", req.path))
    }

    #[tool(description = "Garbage collect stale dead-drop fallback tickets.")]
    async fn fallback_gc(
        &self,
        Parameters(req): Parameters<FallbackGcRequest>,
    ) -> Result<Json<FallbackGcResponse>, String> {
        if mcp_daemon_proxy_enabled() {
            return fetch_daemon_fallback_gc(&req)
                .await
                .map(Json)
                .map_err(|e| format!("fallback_gc via daemon failed: {e}"));
        }
        let removed = gc_fallbacks(req.max_age_days.unwrap_or(7))
            .map_err(|e| format!("fallback_gc failed: {e}"))?;
        Ok(Json(FallbackGcResponse { removed }))
    }
}

async fn execute_ask_agent(
    req: &AskAgentRequest,
    progress: Option<ProgressEmitter>,
) -> Result<AskAgentResponse, String> {
    if !is_supported_agent(req) {
        return Err("ask_agent supports only agent='gemini' or agent='codex'".to_string());
    }
    let agent = req.agent.to_lowercase();
    let request_id = Uuid::new_v4().to_string();
    let (resolved_cwd, resolved_repo, resolved_branch) =
        core_resolve_context(req.cwd.as_ref(), req.repo.as_ref(), req.branch.as_ref());
    let exec_cwd = resolved_cwd
        .clone()
        .unwrap_or_else(|| ".".to_string());
    let execution_prompt = req.message.clone();
    let worker = acquire_worker(&agent, &exec_cwd).await;
    let mut worker_session_id = worker.session_id.clone();
    let worker_mode = worker.mode.clone();
    let worker_mode_state = "SPAWNED";

    let agent_display = display_agent_name(&agent);
    let mut lifecycle = vec![LifecycleEvent {
        state: worker_mode_state.to_string(),
        detail: format!(
            "{} {} worker{}{}{} (spawn_count={})",
            if worker_mode == WorkerAcquireMode::Spawned {
                "Started"
            } else {
                "Reused"
            },
            agent,
            req.cwd
                .as_ref()
                .map(|v| format!(" cwd={v}"))
                .unwrap_or_default(),
            req.repo
                .as_ref()
                .map(|v| format!(" repo={v}"))
                .unwrap_or_default(),
            req.branch
                .as_ref()
                .map(|v| format!(" branch={v}"))
                .unwrap_or_default(),
            worker.spawn_count
        ),
    }];
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: core_unix_time_ms(),
        request_id: request_id.clone(),
        tool: "ask_agent".to_string(),
        status: "SPAWNED".to_string(),
        agent: Some(agent.clone()),
        detail: lifecycle
            .last()
            .map(|e| e.detail.clone())
            .unwrap_or_default(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress.as_ref() {
        emitter.emit(format!("→ {agent_display}: sent ✓")).await;
    }

    lifecycle.push(LifecycleEvent {
        state: "WORKING".to_string(),
        detail: format!("{agent} is processing request"),
    });
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: core_unix_time_ms(),
        request_id: request_id.clone(),
        tool: "ask_agent".to_string(),
        status: "WORKING".to_string(),
        agent: Some(agent.clone()),
        detail: lifecycle
            .last()
            .map(|e| e.detail.clone())
            .unwrap_or_default(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress.as_ref() {
        emitter.emit(format!("→ {agent_display}: working...")).await;
    }

    let backoffs = [Duration::from_millis(250), Duration::from_secs(1), Duration::from_secs(2)];
    let mut last_err: Option<String> = None;

    for (idx, backoff) in backoffs.iter().enumerate() {
        let session_for_attempt = worker_session_id.clone();
        let mut attempt = Box::pin(run_named_agent_with_session(
            &agent,
            &execution_prompt,
            &exec_cwd,
            session_for_attempt.as_deref(),
        ));
        let started = Instant::now();
        let mut next_heartbeat = Duration::from_secs(10);

        let attempt_result = loop {
            let sleep_duration = next_heartbeat.saturating_sub(started.elapsed());
            tokio::select! {
                result = &mut attempt => break result,
                _ = sleep(sleep_duration) => {
                    if started.elapsed() >= next_heartbeat {
                        let elapsed = started.elapsed().as_secs();
                        let detail = format!("{agent} still working ({elapsed}s elapsed)");
                        lifecycle.push(LifecycleEvent {
                            state: "WORKING".to_string(),
                            detail,
                        });
                        if let Some(emitter) = progress.as_ref() {
                            emitter
                                .emit(format!("→ {agent_display}: working... ({elapsed}s elapsed)"))
                                .await;
                        }
                        next_heartbeat = next_heartbeat_offset(next_heartbeat);
                    }
                }
            }
        };

        match attempt_result {
            Ok((response, next_session_id)) => {
                worker_session_id = next_session_id.clone();
                update_worker_session(&agent, &exec_cwd, next_session_id).await;
                lifecycle.push(LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: format!("{agent} responded on attempt {}", idx + 1),
                });
                if let Err(e) = append_outbox_event(&OutboxEvent {
                    ts_ms: core_unix_time_ms(),
                    request_id: request_id.clone(),
                    tool: "ask_agent".to_string(),
                    status: "DONE".to_string(),
                    agent: Some(agent.clone()),
                    detail: lifecycle
                        .last()
                        .map(|e| e.detail.clone())
                        .unwrap_or_default(),
                    cwd: resolved_cwd.clone(),
                    repo: resolved_repo.clone(),
                    branch: resolved_branch.clone(),
                }) {
                    tracing::warn!("failed to append outbox event: {e}");
                }
                if let Some(emitter) = progress.as_ref() {
                    emitter.emit(format!("→ {agent_display}: responded ✓")).await;
                }
                return Ok(AskAgentResponse {
                    request_id,
                    agent: agent.clone(),
                    response,
                    lifecycle,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                if session_for_attempt.is_some() && should_invalidate_cached_session(&msg) {
                    worker_session_id = None;
                    update_worker_session(&agent, &exec_cwd, None).await;
                    let invalidated_detail = format!(
                        "{agent} session invalidated after stale resume ID; retrying with a fresh session"
                    );
                    lifecycle.push(LifecycleEvent {
                        state: "SESSION_INVALIDATED".to_string(),
                        detail: invalidated_detail.clone(),
                    });
                    if let Err(e) = append_outbox_event(&OutboxEvent {
                        ts_ms: core_unix_time_ms(),
                        request_id: request_id.clone(),
                        tool: "ask_agent".to_string(),
                        status: "SESSION_INVALIDATED".to_string(),
                        agent: Some(agent.clone()),
                        detail: invalidated_detail,
                        cwd: resolved_cwd.clone(),
                        repo: resolved_repo.clone(),
                        branch: resolved_branch.clone(),
                    }) {
                        tracing::warn!("failed to append outbox event: {e}");
                    }
                    if let Some(emitter) = progress.as_ref() {
                        emitter
                            .emit(format!("→ {agent_display}: stale session detected, respawning..."))
                            .await;
                    }
                }
                if msg.contains("timed out") {
                    lifecycle.push(LifecycleEvent {
                        state: "TIMEOUT".to_string(),
                        detail: format!("{agent} timed out on attempt {}", idx + 1),
                    });
                    if let Some(emitter) = progress.as_ref() {
                        emitter
                            .emit(format!("→ {agent_display}: TIMEOUT after 60s ✗"))
                            .await;
                    }
                }
                lifecycle.push(LifecycleEvent {
                    state: "RETRY".to_string(),
                    detail: format!(
                        "Retrying {} ({}/{}) after {}",
                        agent,
                        idx + 1,
                        backoffs.len(),
                        msg
                    ),
                });
                if let Err(e) = append_outbox_event(&OutboxEvent {
                    ts_ms: core_unix_time_ms(),
                    request_id: request_id.clone(),
                    tool: "ask_agent".to_string(),
                    status: "RETRY".to_string(),
                    agent: Some(agent.clone()),
                    detail: lifecycle
                        .last()
                        .map(|e| e.detail.clone())
                        .unwrap_or_default(),
                    cwd: resolved_cwd.clone(),
                    repo: resolved_repo.clone(),
                    branch: resolved_branch.clone(),
                }) {
                    tracing::warn!("failed to append outbox event: {e}");
                }
                if let Some(emitter) = progress.as_ref() {
                    emitter
                        .emit(format!("→ {agent_display}: retrying ({}/{})...", idx + 1, backoffs.len()))
                        .await;
                }
                last_err = Some(msg);
                sleep(*backoff).await;
            }
        }
    }

    lifecycle.push(LifecycleEvent {
        state: "FAILED".to_string(),
        detail: format!("{} failed after {} attempts", agent, backoffs.len()),
    });
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: core_unix_time_ms(),
        request_id: request_id.clone(),
        tool: "ask_agent".to_string(),
        status: "FAILED".to_string(),
        agent: Some(agent.clone()),
        detail: lifecycle
            .last()
            .map(|e| e.detail.clone())
            .unwrap_or_default(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress.as_ref() {
        emitter
            .emit(format!("→ {agent_display}: FAILED after {} attempts", backoffs.len()))
            .await;
    }

    let fallback_path = spawn_dead_drop(
        &agent,
        &req.message,
        &last_err.clone().unwrap_or_else(|| "unknown error".to_string()),
        &resolved_cwd,
        &resolved_repo,
        &resolved_branch,
    )
    .ok();
    if let Some(path) = fallback_path.as_ref() {
        lifecycle.push(LifecycleEvent {
            state: "FALLBACK".to_string(),
            detail: format!("dead drop launched: {}", path.display()),
        });
        let _ = append_outbox_event(&OutboxEvent {
            ts_ms: core_unix_time_ms(),
            request_id: request_id.clone(),
            tool: "ask_agent".to_string(),
            status: "FALLBACK".to_string(),
            agent: Some(agent.clone()),
            detail: format!("dead drop launched: {}", path.display()),
            cwd: resolved_cwd.clone(),
            repo: resolved_repo.clone(),
            branch: resolved_branch.clone(),
        });
        if let Some(emitter) = progress.as_ref() {
            emitter
                .emit(format!("→ {agent_display}: dead drop launched, {}", path.display()))
                .await;
        }
    }
    Err(format!(
        "ask_agent failed after lifecycle {:?}: {}{}",
        lifecycle
            .iter()
            .map(|e| e.state.as_str())
            .collect::<Vec<_>>(),
        last_err.unwrap_or_else(|| "unknown error".to_string()),
        fallback_path
            .map(|p| format!("; dead drop launched at {}", p.display()))
            .unwrap_or_default()
    ))
}

async fn run_named_agent(agent: &str, message: &str, cwd: &str) -> anyhow::Result<String> {
    let (response, _) = run_named_agent_with_session(agent, message, cwd, None).await?;
    Ok(response)
}

async fn run_named_agent_with_session(
    agent: &str,
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
) -> anyhow::Result<(String, Option<String>)> {
    match agent {
        "gemini" => {
            let (bin, args) = gemini_command();
            run_agent_process_with_session("gemini", &bin, &args, message, cwd, session_id).await
        }
        "codex" => {
            let (bin, args) = codex_command();
            run_agent_process_with_session("codex", &bin, &args, message, cwd, session_id).await
        }
        _ => anyhow::bail!("unsupported agent: {agent}"),
    }
}

async fn prewarm_worker(agent: &str, cwd: &str) {
    let worker = acquire_worker(agent, cwd).await;
    let warm_prompt = "Prewarm this session. Reply with only: ready";
    let warm_result = timeout(
        daemon_prewarm_timeout(),
        run_named_agent_with_session(agent, warm_prompt, cwd, worker.session_id.as_deref()),
    )
    .await;

    match warm_result {
        Ok(Ok((_, session_id))) => {
            update_worker_session(agent, cwd, session_id).await;
            tracing::info!("prewarm complete for {agent} cwd={cwd}");
        }
        Ok(Err(err)) => {
            tracing::warn!("prewarm failed for {agent} cwd={cwd}: {err}");
            let _ = dismiss_worker(agent, cwd).await;
        }
        Err(_) => {
            tracing::warn!("prewarm timeout for {agent} cwd={cwd}");
            let _ = dismiss_worker(agent, cwd).await;
        }
    }
}

async fn prewarm_daemon_workers() {
    if !daemon_prewarm_enabled() {
        tracing::info!("daemon prewarm disabled");
        return;
    }

    let cwds = daemon_prewarm_cwds();
    for cwd in cwds {
        prewarm_worker("gemini", &cwd).await;
        prewarm_worker("codex", &cwd).await;
    }
}

fn is_mock_connector(bin: &str) -> bool {
    std::path::Path::new(bin)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|name| name.starts_with("mock-"))
        .unwrap_or(false)
}

fn connector_timeout() -> Duration {
    std::env::var("TRIUMVIRATE_CONNECTOR_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(180))
}

fn daemon_prewarm_enabled() -> bool {
    std::env::var("TRIUMVIRATE_DAEMON_PREWARM")
        .ok()
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

fn daemon_prewarm_timeout() -> Duration {
    std::env::var("TRIUMVIRATE_DAEMON_PREWARM_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(60))
}

fn daemon_prewarm_cwds() -> Vec<String> {
    if let Ok(raw) = std::env::var("TRIUMVIRATE_DAEMON_PREWARM_CWDS") {
        let cwds = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !cwds.is_empty() {
            return cwds;
        }
    }

    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.display().to_string());
    }
    if let Ok(cwd) = std::env::current_dir() {
        let cwd = cwd.display().to_string();
        if !out.iter().any(|v| v == &cwd) {
            out.push(cwd);
        }
    }
    if !out.is_empty() {
        return out;
    }

    vec![".".to_string()]
}

#[cfg(test)]
async fn run_mock_connector_process(
    bin: &str,
    args: &[String],
    message: &str,
    session_id: Option<&str>,
) -> anyhow::Result<(String, Option<String>)> {
    use tokio::io::AsyncReadExt;

    let mut child = Command::new(&bin)
        .args(args)
        .env("TRIUMVIRATE_WORKER_SESSION_ID", session_id.unwrap_or(""))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await?;
        stdin.flush().await?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("gemini stdout missing"))?;
    let mut stderr_reader = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("gemini stderr missing"))?;
    let mut lines = BufReader::new(stdout).lines();
    let mut non_json_line: Option<String> = None;

    // The mock connector may emit readiness notifications before the final result; scan until we
    // find a JSON-RPC payload with result.text.
    let read_result = timeout(Duration::from_secs(5), async {
        while let Some(line) = lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(text) = json
                    .get("result")
                    .and_then(|r| r.get("text"))
                    .and_then(|t| t.as_str())
                {
                    return Ok(text.to_string());
                }
            } else {
                non_json_line = Some(trimmed.to_string());
            }
        }
        if let Some(line) = non_json_line {
            Err(anyhow::anyhow!("mock connector output: {line}"))
        } else {
            Err(anyhow::anyhow!("no result.text message from gemini connector"))
        }
    })
    .await;

    let response = match read_result {
        Ok(result) => match result {
            Ok(response) => response,
            Err(err) => {
                let _ = child.kill().await;
                let mut stderr = String::new();
                let _ = stderr_reader.read_to_string(&mut stderr).await;
                let _ = child.wait().await;
                let stderr = stderr.trim();
                if !stderr.is_empty() {
                    anyhow::bail!("mock connector failed: {stderr}");
                }
                return Err(err);
            }
        },
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!("mock connector timed out")
        }
    };

    let _ = child.kill().await;
    let _ = child.wait().await;

    let out_session = session_id
        .map(ToString::to_string)
        .or_else(|| Some(format!("mock-session-{}", Uuid::new_v4())));
    Ok((response, out_session))
}

fn has_any_arg(args: &[String], candidates: &[&str]) -> bool {
    args.iter().any(|arg| candidates.iter().any(|c| arg == c))
}

async fn run_gemini_cli_process(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
) -> anyhow::Result<(String, Option<String>)> {
    run_gemini_cli_process_with_session(bin, args, message, cwd, None).await
}

async fn run_gemini_cli_process_with_session(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
) -> anyhow::Result<(String, Option<String>)> {
    let mut final_args = args.to_vec();
    if !has_any_arg(&final_args, &["-o", "--output-format"]) {
        final_args.push("-o".to_string());
        final_args.push("stream-json".to_string());
    }
    if let Some(session_id) = session_id
        && !has_any_arg(&final_args, &["-r", "--resume"])
    {
        final_args.push("-r".to_string());
        final_args.push(session_id.to_string());
    }
    if !has_any_arg(&final_args, &["-p", "--prompt"]) {
        final_args.push("-p".to_string());
        final_args.push(message.to_string());
    }

    let output = timeout(
        connector_timeout(),
        Command::new(bin)
            .args(&final_args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("gemini connector timed out"))??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("gemini exited with status {}", output.status)
        } else {
            stderr
        };
        anyhow::bail!("gemini connector failed: {detail}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let mut discovered_session: Option<String> = None;
    let mut assistant_chunks: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if discovered_session.is_none() {
            discovered_session = json
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
        }
        let role = json.get("role").and_then(|v| v.as_str()).unwrap_or_default();
        let ty = json.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        if ty == "message" && role == "assistant"
            && let Some(content) = json.get("content").and_then(|v| v.as_str())
        {
            assistant_chunks.push(content.to_string());
        }
    }
    if !assistant_chunks.is_empty() {
        return Ok((assistant_chunks.join(""), discovered_session.or_else(|| session_id.map(ToString::to_string))));
    }

    let stdout_trimmed = stdout.trim().to_string();
    if !stdout_trimmed.is_empty() {
        return Ok((stdout_trimmed, discovered_session.or_else(|| session_id.map(ToString::to_string))));
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Ok((stderr, discovered_session.or_else(|| session_id.map(ToString::to_string))));
    }

    anyhow::bail!("gemini connector returned empty output")
}

fn extract_text_from_jsonl(stdout: &str) -> Option<String> {
    let mut candidate = None;
    for line in stdout.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
            candidate = Some(text.to_string());
            continue;
        }

        if let Some(text) = json
            .get("result")
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
        {
            candidate = Some(text.to_string());
            continue;
        }

        if let Some(text) = json
            .get("response")
            .and_then(|r| r.get("output_text"))
            .and_then(|t| t.as_str())
        {
            candidate = Some(text.to_string());
        }
    }
    candidate
}

fn is_git_worktree(path: &str) -> bool {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim() == "true",
        _ => false,
    }
}

async fn run_codex_cli_process(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
) -> anyhow::Result<(String, Option<String>)> {
    run_codex_cli_process_with_session(bin, args, message, cwd, None).await
}

async fn run_codex_cli_process_with_session(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
) -> anyhow::Result<(String, Option<String>)> {
    let mut final_args = args.to_vec();
    if session_id.is_some() {
        final_args.insert(0, "resume".to_string());
        final_args.insert(0, "exec".to_string());
        if let Some(session_id) = session_id {
            final_args.push(session_id.to_string());
        }
    } else if final_args.first().map(|s| s.as_str()) != Some("exec") {
        final_args.insert(0, "exec".to_string());
    }
    if !has_any_arg(&final_args, &["--json"]) {
        final_args.push("--json".to_string());
    }
    if !is_git_worktree(cwd) && !has_any_arg(&final_args, &["--skip-git-repo-check"]) {
        final_args.push("--skip-git-repo-check".to_string());
    }

    let output_file = std::env::temp_dir().join(format!(
        "triumvirate-codex-last-message-{}.txt",
        Uuid::new_v4()
    ));
    final_args.push("--output-last-message".to_string());
    final_args.push(output_file.display().to_string());
    final_args.push(message.to_string());

    let output = timeout(
        connector_timeout(),
        Command::new(bin)
            .args(&final_args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("codex connector timed out"))??;

    let last_message = fs::read_to_string(&output_file).unwrap_or_default();
    let _ = fs::remove_file(&output_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("codex exited with status {}", output.status)
        } else {
            stderr
        };
        anyhow::bail!("codex connector failed: {detail}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let mut discovered_thread: Option<String> = None;
    for line in stdout.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if discovered_thread.is_none() {
            discovered_thread = json
                .get("thread_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
        }
    }

    let session_out = discovered_thread.or_else(|| session_id.map(ToString::to_string));

    if !last_message.trim().is_empty() {
        return Ok((last_message.trim().to_string(), session_out));
    }

    if let Some(text) = extract_text_from_jsonl(&stdout) {
        return Ok((text, session_out));
    }
    let stdout_trimmed = stdout.trim().to_string();
    if !stdout_trimmed.is_empty() {
        return Ok((stdout_trimmed, session_out));
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Ok((stderr, session_out));
    }

    anyhow::bail!("codex connector returned empty output")
}

async fn run_agent_process(
    agent: &str,
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
) -> anyhow::Result<(String, Option<String>)> {
    run_agent_process_with_session(agent, bin, args, message, cwd, None).await
}

async fn run_agent_process_with_session(
    agent: &str,
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
) -> anyhow::Result<(String, Option<String>)> {
    if is_mock_connector(bin) {
        #[cfg(test)]
        {
            let _ = cwd;
            return run_mock_connector_process(bin, args, message, session_id).await;
        }
        #[cfg(not(test))]
        {
            anyhow::bail!(
                "mock connectors are test-only; set TRIUMVIRATE_{}_BIN to a real CLI binary",
                agent.to_uppercase()
            );
        }
    }

    match agent {
        "gemini" => run_gemini_cli_process_with_session(bin, args, message, cwd, session_id).await,
        "codex" => run_codex_cli_process_with_session(bin, args, message, cwd, session_id).await,
        _ => anyhow::bail!("unsupported agent: {agent}"),
    }
}

#[tool_handler]
impl ServerHandler for McpBridge {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Triumvirate MCP bridge. Use `ping` to verify connectivity.")
    }
}

fn init_tracing() -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "triumvirate=info".into());
    let json_logs = should_use_daemon_proxy(std::env::var("TRIUMVIRATE_JSON_LOGS").ok().as_deref());
    let otel_endpoint = std::env::var("TRIUMVIRATE_OTEL_EXPORTER_OTLP_ENDPOINT").ok();

    match (json_logs, otel_endpoint) {
        (true, Some(endpoint)) => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .build()?;
            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(Resource::builder().with_service_name("triumvirate-daemon-v2").build())
                .build();
            let tracer = provider.tracer("triumvirate-daemon-v2");
            opentelemetry::global::set_tracer_provider(provider);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().json().with_target(false))
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
        }
        (false, Some(endpoint)) => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .build()?;
            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(Resource::builder().with_service_name("triumvirate-daemon-v2").build())
                .build();
            let tracer = provider.tracer("triumvirate-daemon-v2");
            opentelemetry::global::set_tracer_provider(provider);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().with_target(false))
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
        }
        (true, None) => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().json().with_target(false))
                .init();
        }
        (false, None) => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().with_target(false))
                .init();
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing()?;

    match Cli::parse().command {
        CliCommand::Mcp => {
            McpBridge::new().serve(stdio()).await?.waiting().await?;
        }
        CliCommand::Daemon => {
            run_daemon().await?;
        }
        CliCommand::Install => {
            run_install()?;
        }
        CliCommand::Uninstall => {
            run_uninstall()?;
        }
        CliCommand::Status => {
            run_status().await?;
        }
        CliCommand::Doctor => {
            run_doctor().await?;
        }
    }

    Ok(())
}

fn run_install() -> anyhow::Result<()> {
    let home = core_triumvirate_home_dir()?;
    fs::create_dir_all(&home)?;
    let launch_agents = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?
        .join("Library/LaunchAgents");
    fs::create_dir_all(&launch_agents)?;

    let plist_path = core_launchd_plist_path()?;
    let exe_path = std::env::current_exe()?;
    let plist = core_render_launch_agent_plist(&exe_path.display().to_string(), &home.display().to_string());
    fs::write(&plist_path, plist)?;

    println!("Installed launchd plist at {}", plist_path.display());
    println!("Load with: launchctl load {}", plist_path.display());
    println!("Start now with: launchctl start com.triumvirate.daemon-v2");
    Ok(())
}

fn run_uninstall() -> anyhow::Result<()> {
    let plist_path = core_launchd_plist_path()?;
    if plist_path.exists() {
        fs::remove_file(&plist_path)?;
        println!("Removed launchd plist at {}", plist_path.display());
    } else {
        println!("No launchd plist found at {}", plist_path.display());
    }
    println!("Unload with: launchctl unload {}", plist_path.display());
    Ok(())
}

async fn run_doctor() -> anyhow::Result<()> {
    let token_path = core_triumvirate_home_dir()?.join("daemon.token");
    let plist_path = core_launchd_plist_path()?;
    let daemon_health = fetch_daemon_status().await.ok();
    let daemon_bind_addr =
        core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());
    let daemon_base_url = daemon_base_url();
    let daemon_status_url = daemon_status_url();
    let report = serde_json::json!({
        "token_file_exists": token_path.exists(),
        "token_file_path": token_path,
        "launchd_plist_exists": plist_path.exists(),
        "launchd_plist_path": plist_path,
        "daemon_bind_addr": daemon_bind_addr,
        "daemon_base_url": daemon_base_url,
        "daemon_status_url": daemon_status_url,
        "daemon_reachable": daemon_health.is_some(),
        "daemon_health": daemon_health
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn run_status() -> anyhow::Result<()> {
    let daemon_bind_addr =
        core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());

    let health = fetch_daemon_status().await.ok();
    let snapshot = fetch_daemon_status_snapshot().await.ok();
    let pending_fallbacks = count_pending_fallbacks().unwrap_or(0);
    let fallback_tickets = list_pending_fallback_paths(10)
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>();
    let report = build_status_report(
        daemon_bind_addr,
        health,
        snapshot,
        pending_fallbacks,
        fallback_tickets,
    );

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn build_status_report(
    daemon_bind_addr: String,
    health: Option<DaemonHealthResponse>,
    snapshot: Option<DaemonStatusSnapshot>,
    pending_fallbacks: usize,
    fallback_tickets: Vec<String>,
) -> serde_json::Value {
    if let (Some(health), Some(snapshot)) = (health, snapshot) {
        // Normalize daemon snapshot payload so CLI output is stable even if the daemon
        // omitted optional fields in older versions.
        let snapshot_value = serde_json::json!({
            "daemon_mode": snapshot.daemon_mode.unwrap_or_else(|| "incremental-dev".to_string()),
            "supported_agents": snapshot
                .supported_agents
                .unwrap_or_else(|| vec!["gemini".to_string(), "codex".to_string()]),
            "pending_fallbacks": snapshot.pending_fallbacks.unwrap_or(0),
            "fallback_tickets": snapshot.fallback_tickets.unwrap_or_default(),
            "daemon_bind_addr": snapshot.daemon_bind_addr.unwrap_or_else(|| daemon_bind_addr.clone()),
        });

        return serde_json::json!({
            "daemon_reachable": true,
            "daemon_bind_addr": daemon_bind_addr,
            "health": health,
            "snapshot": snapshot_value
        });
    }

    // Fallback path keeps `status` useful even when daemon HTTP is unavailable.
    serde_json::json!({
        "daemon_reachable": false,
        "daemon_bind_addr": daemon_bind_addr.clone(),
        "health": null,
        "snapshot": {
            "daemon_mode": "incremental-dev",
            "supported_agents": ["gemini", "codex"],
            "pending_fallbacks": pending_fallbacks,
            "fallback_tickets": fallback_tickets,
            "daemon_bind_addr": daemon_bind_addr
        }
    })
}

async fn run_daemon() -> anyhow::Result<()> {
    #[derive(Debug, Clone)]
    struct DaemonMetrics {
        registry: Registry,
        requests_total: IntCounterVec,
        request_duration_seconds: HistogramVec,
        in_flight_requests: IntGauge,
    }

    impl DaemonMetrics {
        fn new() -> anyhow::Result<Self> {
            let registry = Registry::new();
            let requests_total = IntCounterVec::new(
                prometheus::Opts::new("triumvirate_http_requests_total", "HTTP requests by route and status"),
                &["route", "status"],
            )?;
            let request_duration_seconds = HistogramVec::new(
                prometheus::HistogramOpts::new(
                    "triumvirate_http_request_duration_seconds",
                    "HTTP request durations by route",
                ),
                &["route"],
            )?;
            let in_flight_requests = IntGauge::new(
                "triumvirate_http_requests_in_flight",
                "In-flight HTTP requests",
            )?;
            registry.register(Box::new(requests_total.clone()))?;
            registry.register(Box::new(request_duration_seconds.clone()))?;
            registry.register(Box::new(in_flight_requests.clone()))?;
            Ok(Self {
                registry,
                requests_total,
                request_duration_seconds,
                in_flight_requests,
            })
        }
    }

    #[derive(Debug, Clone)]
    struct DaemonState {
        token: String,
        queues: QueueRegistry,
        bind_addr: String,
        sessions: Arc<Mutex<HashMap<String, SessionState>>>,
        sessions_file: Option<PathBuf>,
        metrics: Arc<DaemonMetrics>,
    }

    async fn metrics_route(
        State(state): State<DaemonState>,
    ) -> Result<String, (StatusCode, AxumJson<serde_json::Value>)> {
        let metric_families = state.metrics.registry.gather();
        let mut body = Vec::<u8>::new();
        TextEncoder::new().encode(&metric_families, &mut body).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
        String::from_utf8(body).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })
    }

    async fn metrics_middleware(
        State(state): State<DaemonState>,
        request: Request<Body>,
        next: Next,
    ) -> Response {
        let route = request.uri().path().to_string();
        state.metrics.in_flight_requests.inc();
        let started = Instant::now();
        let response = next.run(request).await;
        let elapsed = started.elapsed().as_secs_f64();
        let status = response.status().as_u16().to_string();
        state
            .metrics
            .requests_total
            .with_label_values(&[route.as_str(), status.as_str()])
            .inc();
        state
            .metrics
            .request_duration_seconds
            .with_label_values(&[route.as_str()])
            .observe(elapsed);
        state.metrics.in_flight_requests.dec();
        response
    }

    async fn health(
        State(state): State<DaemonState>,
        headers: HeaderMap,
    ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(AxumJson(serde_json::json!({
            "status": "ok",
            "service": "triumvirate-daemon-v2",
            "mode": "incremental-dev",
            "daemon_bind_addr": state.bind_addr
        })))
    }

    async fn status(
        State(state): State<DaemonState>,
        headers: HeaderMap,
    ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let pending = count_pending_fallbacks().unwrap_or(0);
        let tickets = list_pending_fallback_paths(10)
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        Ok(AxumJson(serde_json::json!({
            "daemon": "running",
            "auth": "bearer-required",
            "daemon_mode": "incremental-dev",
            "supported_agents": ["gemini", "codex"],
            "pending_fallbacks": pending,
            "fallback_tickets": tickets,
            "daemon_bind_addr": state.bind_addr
        })))
    }

    async fn ask_agent_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<AskAgentRequest>,
    ) -> Result<AxumJson<AskAgentResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        // Serialize agent execution per project to keep ordering predictable for concurrent bridges.
        let queue = core_acquire_project_queue(
            &state.queues,
            core_project_queue_key(req.cwd.as_ref(), req.repo.as_ref()),
        )
        .await;
        let _guard = queue.lock().await;
        execute_ask_agent(&req, None).await.map(AxumJson).map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                AxumJson(serde_json::json!({ "error": e })),
            )
        })
    }

    async fn memory_write_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<MemoryWriteRequest>,
    ) -> Result<AxumJson<MemoryWriteResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        let id = Uuid::new_v4().to_string();
        let entry = MemoryEntry {
            id: id.clone(),
            namespace: req.namespace,
            key: req.key,
            value: req.value,
            ts_ms: core_unix_time_ms(),
        };
        append_memory_entry(&entry).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
        Ok(AxumJson(MemoryWriteResponse {
            id,
            status: "ok".to_string(),
        }))
    }

    async fn memory_read_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<MemoryReadRequest>,
    ) -> Result<AxumJson<MemoryReadResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        let mut entries = read_memory_entries().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
        entries.retain(|e| e.namespace == req.namespace);
        if let Some(key) = req.key {
            entries.retain(|e| e.key == key);
        }
        entries.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
        if let Some(limit) = req.limit {
            entries.truncate(limit);
        }
        Ok(AxumJson(MemoryReadResponse { entries }))
    }

    async fn scratchpad_write_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<ScratchpadWriteRequest>,
    ) -> Result<AxumJson<ScratchpadWriteResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        let path = write_scratchpad(&req.project, &req.topic, &req.content).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
        Ok(AxumJson(ScratchpadWriteResponse {
            path: path.display().to_string(),
        }))
    }

    async fn scratchpad_list_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<ScratchpadListRequest>,
    ) -> Result<AxumJson<ScratchpadListResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        let files = list_scratchpad(&req.project)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AxumJson(serde_json::json!({ "error": e.to_string() })),
                )
            })?
            .into_iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        Ok(AxumJson(ScratchpadListResponse { files }))
    }

    async fn outbox_recent_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<OutboxRecentRequest>,
    ) -> Result<AxumJson<OutboxRecentResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        let mut events = read_outbox_events().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
        events.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
        events.truncate(req.limit.unwrap_or(50));
        Ok(AxumJson(OutboxRecentResponse { events }))
    }

    async fn fallback_list_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<FallbackListRequest>,
    ) -> Result<AxumJson<FallbackListResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        let tickets = list_pending_fallback_paths(req.limit.unwrap_or(20))
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AxumJson(serde_json::json!({ "error": e.to_string() })),
                )
            })?
            .into_iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        Ok(AxumJson(FallbackListResponse { tickets }))
    }

    async fn fallback_ack_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<FallbackAckRequest>,
    ) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        acknowledge_fallback_path(&req.path).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
        Ok(AxumJson(serde_json::json!({
            "status": "ok",
            "message": format!("acknowledged {}", req.path)
        })))
    }

    async fn fallback_gc_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<FallbackGcRequest>,
    ) -> Result<AxumJson<FallbackGcResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        let removed = gc_fallbacks(req.max_age_days.unwrap_or(7)).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
        Ok(AxumJson(FallbackGcResponse { removed }))
    }

    async fn session_spawn_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<SpawnSessionRequest>,
    ) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((StatusCode::UNAUTHORIZED, AxumJson(serde_json::json!({ "error": "unauthorized" }))));
        }
        let agent = req.agent.to_lowercase();
        if !is_supported_agent_name(&agent) {
            return Err((StatusCode::BAD_REQUEST, AxumJson(serde_json::json!({ "error": "spawn_session supports only 'gemini' or 'codex'" }))));
        }
        let cwd = req.cwd.clone().unwrap_or_else(|| ".".to_string());
        let worker = acquire_worker(&agent, &cwd).await;
        let mut sessions = state.sessions.lock().await;
        sessions.insert(
            req.name.clone(),
            SessionState {
                agent: agent.clone(),
                cwd: Some(cwd),
                history: Vec::new(),
            },
        );
        core_persist_json_file_if_enabled(state.sessions_file.as_ref(), &*sessions).map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(serde_json::json!({ "error": e.to_string() })))
        })?;
        Ok(AxumJson(serde_json::json!({
            "status": "ok",
            "message": format!("session '{}' {} for {}", req.name, if worker.mode == WorkerAcquireMode::Spawned { "spawned" } else { "reused" }, agent)
        })))
    }

    async fn session_ask_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<AskSessionRequest>,
    ) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((StatusCode::UNAUTHORIZED, AxumJson(serde_json::json!({ "error": "unauthorized" }))));
        }
        let (agent, cwd) = {
            let sessions = state.sessions.lock().await;
            let session = sessions.get(&req.name).ok_or_else(|| {
                (StatusCode::NOT_FOUND, AxumJson(serde_json::json!({ "error": format!("session '{}' not found", req.name) })))
            })?;
            (session.agent.clone(), session.cwd.clone())
        };
        let response = execute_ask_agent(
            &AskAgentRequest {
                agent,
                message: req.message.clone(),
                cwd,
                repo: None,
                branch: None,
            },
            None,
        )
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, AxumJson(serde_json::json!({ "error": e }))))?
        .response;

        let mut sessions = state.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&req.name) {
            session.history.push(format!("user: {}", req.message));
            session.history.push(format!("assistant: {response}"));
        }
        core_persist_json_file_if_enabled(state.sessions_file.as_ref(), &*sessions).map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(serde_json::json!({ "error": e.to_string() })))
        })?;

        Ok(AxumJson(serde_json::json!({ "status": "ok", "response": response })))
    }

    async fn session_dismiss_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<DismissSessionRequest>,
    ) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((StatusCode::UNAUTHORIZED, AxumJson(serde_json::json!({ "error": "unauthorized" }))));
        }
        let mut sessions = state.sessions.lock().await;
        let Some(removed) = sessions.remove(&req.name) else {
            return Err((StatusCode::NOT_FOUND, AxumJson(serde_json::json!({ "error": format!("session '{}' not found", req.name) }))));
        };
        let should_drop_worker = !sessions.values().any(|s| s.agent == removed.agent && s.cwd == removed.cwd);
        if should_drop_worker {
            let cwd = removed.cwd.unwrap_or_else(|| ".".to_string());
            let _ = dismiss_worker(&removed.agent, &cwd).await;
        }
        core_persist_json_file_if_enabled(state.sessions_file.as_ref(), &*sessions).map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(serde_json::json!({ "error": e.to_string() })))
        })?;
        Ok(AxumJson(serde_json::json!({
            "status": "ok",
            "message": format!("session '{}' dismissed", req.name)
        })))
    }

    async fn session_list_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
    ) -> Result<AxumJson<SessionListResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((StatusCode::UNAUTHORIZED, AxumJson(serde_json::json!({ "error": "unauthorized" }))));
        }
        let sessions = state.sessions.lock().await;
        let mut out = sessions
            .iter()
            .map(|(name, s)| SessionInfo {
                name: name.clone(),
                agent: s.agent.clone(),
                turns: s.history.len(),
            })
            .collect::<Vec<_>>();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(AxumJson(SessionListResponse { sessions: out }))
    }

    let token = core_ensure_daemon_token(&core_triumvirate_home_dir()?)?;
    let bind_addr = core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());
    let sessions_file = core_triumvirate_home_dir()
        .ok()
        .map(|home| core_sessions_file_path(&home));
    let sessions = sessions_file
        .as_ref()
        .and_then(|path| core_load_json_file_if_exists::<HashMap<String, SessionState>>(path).ok())
        .unwrap_or_default();
    let state = DaemonState {
        token,
        queues: Arc::new(Mutex::new(HashMap::new())),
        bind_addr: bind_addr.clone(),
        sessions: Arc::new(Mutex::new(sessions)),
        sessions_file,
        metrics: Arc::new(DaemonMetrics::new()?),
    };
    let app = Router::new()
        .route("/metrics", get(metrics_route))
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/ask-agent", post(ask_agent_route))
        .route("/memory/write", post(memory_write_route))
        .route("/memory/read", post(memory_read_route))
        .route("/scratchpad/write", post(scratchpad_write_route))
        .route("/scratchpad/list", post(scratchpad_list_route))
        .route("/outbox/recent", post(outbox_recent_route))
        .route("/fallback/list", post(fallback_list_route))
        .route("/fallback/ack", post(fallback_ack_route))
        .route("/fallback/gc", post(fallback_gc_route))
        .route("/session/spawn", post(session_spawn_route))
        .route("/session/ask", post(session_ask_route))
        .route("/session/dismiss", post(session_dismiss_route))
        .route("/session/list", get(session_list_route))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, metrics_middleware));
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tokio::spawn(async {
        prewarm_daemon_workers().await;
    });
    axum::serve(listener, app).await?;
    Ok(())
}

fn append_memory_entry(entry: &MemoryEntry) -> anyhow::Result<()> {
    core_append_memory_entry(&core_triumvirate_home_dir()?, entry)
}

fn read_memory_entries() -> anyhow::Result<Vec<MemoryEntry>> {
    core_read_memory_entries(&core_triumvirate_home_dir()?)
}

fn write_scratchpad(project: &str, topic: &str, content: &str) -> anyhow::Result<PathBuf> {
    core_write_scratchpad(
        &core_triumvirate_home_dir()?,
        project,
        topic,
        content,
        core_unix_time_ms(),
    )
}

fn list_scratchpad(project: &str) -> anyhow::Result<Vec<PathBuf>> {
    core_list_scratchpad(&core_triumvirate_home_dir()?, project)
}

fn spawn_dead_drop(
    agent: &str,
    message: &str,
    reason: &str,
    cwd: &Option<String>,
    repo: &Option<String>,
    branch: &Option<String>,
) -> anyhow::Result<PathBuf> {
    let id = Uuid::new_v4().to_string();
    create_dead_drop_fallback(
        agent,
        message,
        reason,
        cwd,
        repo,
        branch,
        &id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{ClientHandler, model::ClientInfo};
    use rmcp::model::{
        CallToolRequestParams, LoggingMessageNotificationParam, ProgressNotificationParam,
    };
    use std::{
        future::Future,
        fs,
        net::SocketAddr,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::{Arc, Mutex, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Debug, Clone, Default)]
    struct NoopClient;

    impl ClientHandler for NoopClient {
        fn get_info(&self) -> ClientInfo {
            ClientInfo::default()
        }
    }

    #[derive(Debug, Clone, Default)]
    struct RecordingClient {
        logging_messages: Arc<Mutex<Vec<String>>>,
        progress_messages: Arc<Mutex<Vec<String>>>,
    }

    impl ClientHandler for RecordingClient {
        fn get_info(&self) -> ClientInfo {
            ClientInfo::default()
        }

        fn on_progress(
            &self,
            params: ProgressNotificationParam,
            _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
        ) -> impl Future<Output = ()> + rmcp::service::MaybeSendFuture + '_ {
            let messages = self.progress_messages.clone();
            async move {
                if let Some(message) = params.message {
                    messages.lock().expect("progress lock poisoned").push(message);
                }
            }
        }

        fn on_logging_message(
            &self,
            params: LoggingMessageNotificationParam,
            _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
        ) -> impl Future<Output = ()> + rmcp::service::MaybeSendFuture + '_ {
            let messages = self.logging_messages.clone();
            async move {
                if let Some(message) = params.data.as_str() {
                    messages
                        .lock()
                        .expect("logging lock poisoned")
                        .push(message.to_string());
                }
            }
        }
    }

    #[tokio::test]
    async fn ping_tool_returns_pong() -> anyhow::Result<()> {
        let (server_transport, client_transport) = tokio::io::duplex(4096);

        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });

        let client = NoopClient.serve(client_transport).await?;
        let result = client.call_tool(CallToolRequestParams::new("ping")).await?;
        let text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");

        assert_eq!(text, "pong");

        client.cancel().await?;
        server_handle.await??;
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_emits_progress_notifications() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let script_path = write_mock_agent_script("gemini", 1.0)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
        }

        let (server_transport, client_transport) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });

        let client_handler = RecordingClient::default();
        let client = client_handler.clone().serve(client_transport).await?;
        {
            let tool_future = client.call_tool(
                CallToolRequestParams::new("ask_agent").with_arguments(
                    serde_json::json!({
                        "agent": "gemini",
                        "message": "progress check"
                    })
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
                ),
            );
            tokio::pin!(tool_future);

            let result = tool_future.as_mut().await?;
            let text = result
                .content
                .first()
                .and_then(|c| c.raw.as_text())
                .map(|t| t.text.clone())
                .unwrap_or_default();
            assert!(text.contains("\"state\":\"DONE\""));

            let all_logs = client_handler
                .logging_messages
                .lock()
                .expect("logging lock poisoned")
                .clone();
            assert!(all_logs.iter().any(|m| m.contains("Gemini: sent")));
            assert!(all_logs.iter().any(|m| m.contains("Gemini: responded")));
        }

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
        }
        let _ = fs::remove_file(script_path);
        client.cancel().await?;
        server_handle.await??;
        Ok(())
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn write_mock_gemini_script() -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-gemini-{now}.sh"));
        let script = r#"#!/bin/sh
echo '{"jsonrpc":"2.0","method":"session/ready","params":{"text":"mock ready"}}'
IFS= read -r _line
echo '{"jsonrpc":"2.0","id":1,"result":{"text":"mock-gemini received: test message"}}'
"#;
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_mock_agent_script(name: &str, delay_s: f32) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-{name}-{now}.sh"));
        let script = format!(
            "#!/bin/sh\n\
echo '{{\"jsonrpc\":\"2.0\",\"method\":\"session/ready\",\"params\":{{\"text\":\"{name} ready\"}}}}'\n\
IFS= read -r _line\n\
sleep {delay}\n\
echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"text\":\"{name} done\"}}}}'\n",
            name = name,
            delay = delay_s
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_mock_worker_warm_script(name: &str) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-{name}-warm-{now}.sh"));
        let script = format!(
            "#!/bin/sh\n\
if [ -z \"$TRIUMVIRATE_WORKER_SESSION_ID\" ]; then\n\
  sleep 0.8\n\
else\n\
  sleep 0.05\n\
fi\n\
IFS= read -r _line\n\
echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"text\":\"{name} warm worker done\"}}}}'\n",
            name = name
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_retry_agent_script(name: &str) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-{name}-retry-{now}.sh"));
        let state_path = std::env::temp_dir().join(format!("mock-{name}-retry-state-{now}.txt"));
        let script = format!(
            "#!/bin/sh\n\
state_file=\"{state_file}\"\n\
count=0\n\
if [ -f \"$state_file\" ]; then count=$(cat \"$state_file\"); fi\n\
count=$((count+1))\n\
echo \"$count\" > \"$state_file\"\n\
IFS= read -r _line\n\
if [ \"$count\" -eq 1 ]; then\n\
  echo '{{\"jsonrpc\":\"2.0\",\"method\":\"session/ready\",\"params\":{{\"text\":\"{name} attempt1 no result\"}}}}'\n\
  exit 0\n\
fi\n\
echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"text\":\"{name} recovered on retry\"}}}}'\n",
            state_file = state_path.display(),
            name = name
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_failing_agent_script(name: &str) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-{name}-fail-{now}.sh"));
        let script = format!(
            "#!/bin/sh\n\
echo '{{\"jsonrpc\":\"2.0\",\"method\":\"session/ready\",\"params\":{{\"text\":\"{name} ready\"}}}}'\n\
IFS= read -r _line\n\
exit 1\n",
            name = name
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_invalid_session_recovery_script(name: &str) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-{name}-invalid-session-{now}.sh"));
        let script = format!(
            "#!/bin/sh\n\
if [ -n \"$TRIUMVIRATE_WORKER_SESSION_ID\" ]; then\n\
  echo 'Error resuming session: Invalid session identifier \"'$TRIUMVIRATE_WORKER_SESSION_ID'\".' 1>&2\n\
  exit 1\n\
fi\n\
IFS= read -r _line\n\
echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"text\":\"{name} recovered with fresh session\"}}}}'\n",
            name = name
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    #[tokio::test]
    async fn ask_agent_gemini_happy_path_returns_lifecycle() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let script_path = write_mock_gemini_script()?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });

        let client = NoopClient.serve(client_transport).await?;

        let args = serde_json::json!({
            "agent": "gemini",
            "message": "test message",
            "cwd": "/tmp/project",
            "repo": "triumvirate",
            "branch": "feat/mcp-first"
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_agent")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();

        assert!(raw_text.contains("mock-gemini received: test message"));
        assert!(raw_text.contains("SPAWNED"));
        assert!(raw_text.contains("WORKING"));
        assert!(raw_text.contains("DONE"));

        client.cancel().await?;
        server_handle.await??;

        let _ = fs::remove_file(script_path);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_codex_happy_path_returns_lifecycle() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let script_path = write_mock_agent_script("codex", 0.0)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });

        let client = NoopClient.serve(client_transport).await?;

        let args = serde_json::json!({
            "agent": "codex",
            "message": "implement auth",
            "cwd": "/tmp/project",
            "repo": "triumvirate",
            "branch": "feat/mcp-first"
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_agent")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();

        assert!(raw_text.contains("codex done"));
        assert!(raw_text.contains("SPAWNED"));
        assert!(raw_text.contains("WORKING"));
        assert!(raw_text.contains("DONE"));

        client.cancel().await?;
        server_handle.await??;

        let _ = fs::remove_file(script_path);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_retries_and_recovers() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let script_path = write_retry_agent_script("gemini")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let args = serde_json::json!({
            "agent": "gemini",
            "message": "test retry",
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_agent")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(raw_text.contains("gemini recovered on retry"));
        assert!(raw_text.contains("RETRY"));
        assert!(raw_text.contains("DONE"));

        client.cancel().await?;
        server_handle.await??;
        let _ = fs::remove_file(script_path);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_invalid_stale_session_recovers_with_fresh_spawn() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        reset_worker_registry_for_tests().await;
        let script_path = write_invalid_session_recovery_script("gemini")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let cwd = "/tmp/invalid-session-recovery";
        let _ = acquire_worker("gemini", cwd).await;
        update_worker_session("gemini", cwd, Some("stale-session-id".to_string())).await;

        let response = execute_ask_agent(
            &AskAgentRequest {
                agent: "gemini".to_string(),
                message: "recover please".to_string(),
                cwd: Some(cwd.to_string()),
                repo: None,
                branch: None,
            },
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;

        assert!(response.response.contains("gemini recovered with fresh session"));
        assert!(response
            .lifecycle
            .iter()
            .any(|e| e.state == "SESSION_INVALIDATED"));
        assert!(response.lifecycle.iter().any(|e| e.state == "RETRY"));
        assert!(response.lifecycle.iter().any(|e| e.state == "DONE"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        let _ = fs::remove_file(script_path);
        Ok(())
    }

    #[tokio::test]
    async fn persistent_worker_reuse_second_call_is_faster_and_marked_reused() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        reset_worker_registry_for_tests().await;
        let script_path = write_mock_worker_warm_script("gemini")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let req = AskAgentRequest {
            agent: "gemini".to_string(),
            message: "first".to_string(),
            cwd: Some("/tmp/worker-reuse".to_string()),
            repo: None,
            branch: None,
        };
        let first_start = Instant::now();
        let first = execute_ask_agent(&req, None).await.map_err(anyhow::Error::msg)?;
        let first_elapsed = first_start.elapsed();

        let second_start = Instant::now();
        let second = execute_ask_agent(
            &AskAgentRequest {
                message: "second".to_string(),
                ..req
            },
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let second_elapsed = second_start.elapsed();

        assert!(first
            .lifecycle
            .iter()
            .any(|e| e.state == "SPAWNED"));
        assert!(second
            .lifecycle
            .iter()
            .any(|e| e.detail.contains("Reused gemini worker")));
        assert!(
            second_elapsed < first_elapsed,
            "expected second call ({second_elapsed:?}) to be faster than first ({first_elapsed:?})"
        );

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        let _ = fs::remove_file(script_path);
        Ok(())
    }

    #[tokio::test]
    async fn session_lifecycle_spawn_ask_list_dismiss() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let gemini_script = write_mock_agent_script("gemini", 0.0)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", gemini_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let (server_transport, client_transport) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let spawn_args = serde_json::json!({
            "agent": "gemini",
            "name": "my-research"
        });
        let _spawn = client
            .call_tool(
                CallToolRequestParams::new("spawn_session")
                    .with_arguments(spawn_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let ask_args = serde_json::json!({
            "name": "my-research",
            "message": "what is jwt?"
        });
        let ask = client
            .call_tool(
                CallToolRequestParams::new("ask_session")
                    .with_arguments(ask_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;
        let ask_text = ask
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(ask_text.contains("gemini done"));

        let list = client
            .call_tool(CallToolRequestParams::new("list_sessions"))
            .await?;
        let list_text = list
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(list_text.contains("my-research"));

        let dismiss_args = serde_json::json!({
            "name": "my-research"
        });
        let _dismiss = client
            .call_tool(
                CallToolRequestParams::new("dismiss_session")
                    .with_arguments(dismiss_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        client.cancel().await?;
        server_handle.await??;

        let _ = fs::remove_file(gemini_script);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn get_status_reports_active_sessions() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_DAEMON_BIND_ADDR", "127.0.0.1:7777") };

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let spawn_args = serde_json::json!({
            "agent": "gemini",
            "name": "status-session"
        });
        let _ = client
            .call_tool(
                CallToolRequestParams::new("spawn_session")
                    .with_arguments(spawn_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let status = client
            .call_tool(CallToolRequestParams::new("get_status"))
            .await?;
        let status_text = status
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(status_text.contains("\"active_sessions\":1"));
        assert!(status_text.contains("\"supported_agents\":[\"gemini\",\"codex\"]"));
        assert!(status_text.contains("\"daemon_bind_addr\":\"127.0.0.1:7777\""));

        client.cancel().await?;
        server_handle.await??;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_DAEMON_BIND_ADDR") };
        Ok(())
    }

    #[tokio::test]
    async fn get_status_includes_pending_fallback_tickets() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-status-fallbacks-{now}"));
        let dead_drop = test_home.join("dead-drop");
        fs::create_dir_all(&dead_drop)?;
        fs::write(dead_drop.join("ticket-1.md"), "fallback")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let status = client
            .call_tool(CallToolRequestParams::new("get_status"))
            .await?;
        let status_text = status
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(status_text.contains("\"pending_fallbacks\":1"));
        assert!(status_text.contains("ticket-1.md"));

        client.cancel().await?;
        server_handle.await??;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn get_status_reports_total_pending_even_when_ticket_list_is_truncated() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-status-fallbacks-total-{now}"));
        let dead_drop = test_home.join("dead-drop");
        fs::create_dir_all(&dead_drop)?;
        for i in 0..12 {
            fs::write(dead_drop.join(format!("ticket-{i}.md")), "fallback")?;
        }
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        let bridge = McpBridge::new_ephemeral();
        let status = bridge.get_status().await;
        let status_json = serde_json::to_string(&status.0)?;
        assert!(status_json.contains("\"pending_fallbacks\":12"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn get_status_uses_daemon_snapshot_when_proxy_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-status-daemon-snapshot-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "status-daemon-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
        }

        async fn health_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
        ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(serde_json::json!({
                "daemon_mode": "daemon-snapshot",
                "supported_agents": ["gemini", "codex", "claude"],
                "pending_fallbacks": 7,
                "fallback_tickets": ["x.md", "y.md"],
                "daemon_bind_addr": "127.0.0.1:9999"
            })))
        }

        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/status", get(health_handler))
            .with_state(TestState {
                token: token.to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var("TRIUMVIRATE_DAEMON_URL", format!("http://{addr}/status"));
        }

        let bridge = McpBridge::new_ephemeral();
        let status = bridge.get_status().await;
        assert_eq!(status.0.daemon_mode, "daemon-snapshot");
        assert_eq!(status.0.pending_fallbacks, 7);
        assert_eq!(status.0.fallback_tickets.len(), 2);
        assert!(status.0.supported_agents.contains(&"claude".to_string()));
        assert_eq!(status.0.daemon_bind_addr, "127.0.0.1:9999");

        server.abort();
        let _ = server.await;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn get_status_falls_back_local_when_daemon_snapshot_unreachable() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home =
            std::env::temp_dir().join(format!("triumvirate-status-daemon-fallback-{now}"));
        fs::create_dir_all(&test_home)?;
        let dead_drop = test_home.join("dead-drop");
        fs::create_dir_all(&dead_drop)?;
        fs::write(dead_drop.join("ticket-local.md"), "fallback")?;
        fs::write(test_home.join("daemon.token"), "local-token\n")?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var("TRIUMVIRATE_DAEMON_AUTOSTART", "false");
            std::env::set_var("TRIUMVIRATE_DAEMON_URL", "http://127.0.0.1:9/status");
        }

        let bridge = McpBridge::new_ephemeral();
        let status = bridge.get_status().await;
        assert_eq!(status.0.daemon_mode, "incremental-dev");
        assert_eq!(status.0.pending_fallbacks, 1);
        assert!(status
            .0
            .fallback_tickets
            .iter()
            .any(|p| p.contains("ticket-local.md")));
        assert_eq!(status.0.daemon_bind_addr, "127.0.0.1:8080");

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_AUTOSTART");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[test]
    fn daemon_token_is_created_and_reused() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-home-{now}"));
        fs::create_dir_all(&test_home)?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        let token_one = core_ensure_daemon_token(&core_triumvirate_home_dir()?)?;
        let token_two = core_ensure_daemon_token(&core_triumvirate_home_dir()?)?;
        assert_eq!(token_one, token_two);
        assert!(!token_one.is_empty());

        let token_path = test_home.join("daemon.token");
        assert!(token_path.exists());

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[test]
    fn bearer_auth_validation_works() {
        let token = "abc-123";
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .expect("header should parse"),
        );
        assert!(is_bearer_authorized(
            headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
            token
        ));
        assert!(!is_bearer_authorized(
            headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
            "wrong"
        ));
    }

    #[test]
    fn launch_agent_plist_contains_expected_values() {
        let plist = core_render_launch_agent_plist("/usr/local/bin/triumvirate", "/tmp/tri-home");
        assert!(plist.contains("com.triumvirate.daemon-v2"));
        assert!(plist.contains("<string>/usr/local/bin/triumvirate</string>"));
        assert!(plist.contains("<string>daemon</string>"));
        assert!(plist.contains("<string>/tmp/tri-home/daemon.log</string>"));
    }

    #[test]
    fn daemon_autostart_attempt_is_one_shot() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        reset_daemon_autostart_flag_for_tests();
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_DAEMON_AUTOSTART", "true");
            std::env::set_var("TRIUMVIRATE_DAEMON_AUTOSTART_DRYRUN", "true");
        }

        let first = attempt_daemon_autostart_once()?;
        let second = attempt_daemon_autostart_once()?;
        assert!(first, "first call should attempt autostart");
        assert!(!second, "second call should be suppressed");

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_AUTOSTART");
            std::env::remove_var("TRIUMVIRATE_DAEMON_AUTOSTART_DRYRUN");
        }
        Ok(())
    }

    #[test]
    fn cli_parses_status_subcommand() {
        let cli = Cli::try_parse_from(["triumvirate", "status"]).expect("status should parse");
        assert!(matches!(cli.command, CliCommand::Status));
    }

    #[test]
    fn cli_parses_uninstall_subcommand() {
        let cli = Cli::try_parse_from(["triumvirate", "uninstall"]).expect("uninstall should parse");
        assert!(matches!(cli.command, CliCommand::Uninstall));
    }

    #[test]
    fn cli_parses_doctor_subcommand() {
        let cli = Cli::try_parse_from(["triumvirate", "doctor"]).expect("doctor should parse");
        assert!(matches!(cli.command, CliCommand::Doctor));
    }

    #[test]
    fn project_queue_key_prefers_repo_then_cwd() {
        assert_eq!(
            core_project_queue_key(Some(&"/tmp/a".to_string()), Some(&"triumvirate".to_string())),
            "repo:triumvirate"
        );
        assert_eq!(
            core_project_queue_key(Some(&"/tmp/a".to_string()), None),
            "cwd:/tmp/a"
        );
        assert_eq!(core_project_queue_key(None, None), "global");
    }

    #[tokio::test]
    async fn daemon_health_uses_bearer_token() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-health-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "test-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
        }

        async fn status_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
        ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(serde_json::json!({
                "status": "ok",
                "service": "test-daemon",
                "mode": "test"
            })))
        }

        let app = Router::new()
            .route("/status", get(status_handler))
            .with_state(TestState {
                token: token.to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_DAEMON_HEALTH_URL", format!("http://{addr}/health"));
            std::env::set_var("TRIUMVIRATE_DAEMON_URL", format!("http://{addr}/status"));
        }

        let health = fetch_daemon_status().await?;
        assert_eq!(health.status, "ok");
        assert_eq!(health.service.as_deref(), Some("test-daemon"));

        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_DAEMON_HEALTH_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[test]
    fn build_status_report_reachable_includes_snapshot_bind_addr() {
        let report = super::build_status_report(
            "127.0.0.1:8080".to_string(),
            Some(DaemonHealthResponse {
                status: "ok".to_string(),
                service: Some("svc".to_string()),
                mode: Some("dev".to_string()),
                daemon: None,
                auth: None,
                daemon_bind_addr: None,
            }),
            Some(DaemonStatusSnapshot {
                daemon_mode: Some("daemon-snapshot".to_string()),
                supported_agents: Some(vec!["gemini".to_string()]),
                pending_fallbacks: Some(3),
                fallback_tickets: Some(vec!["a.md".to_string()]),
                daemon_bind_addr: Some("127.0.0.1:9999".to_string()),
            }),
            0,
            Vec::new(),
        );

        assert_eq!(report["daemon_reachable"], true);
        assert_eq!(report["daemon_bind_addr"], "127.0.0.1:8080");
        assert_eq!(report["snapshot"]["daemon_bind_addr"], "127.0.0.1:9999");
        assert_eq!(report["snapshot"]["pending_fallbacks"], 3);
    }

    #[test]
    fn build_status_report_fallback_includes_local_snapshot_and_bind_addr() {
        let report = super::build_status_report(
            "127.0.0.1:7777".to_string(),
            None,
            None,
            2,
            vec!["x.md".to_string(), "y.md".to_string()],
        );

        assert_eq!(report["daemon_reachable"], false);
        assert_eq!(report["daemon_bind_addr"], "127.0.0.1:7777");
        assert_eq!(report["snapshot"]["daemon_bind_addr"], "127.0.0.1:7777");
        assert_eq!(report["snapshot"]["pending_fallbacks"], 2);
    }

    #[tokio::test]
    async fn project_queue_serializes_same_project_requests() -> anyhow::Result<()> {
        let registry: QueueRegistry = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let key = "repo:triumvirate".to_string();
        let queue = core_acquire_project_queue(&registry, key.clone()).await;

        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

        let holder = tokio::spawn(async move {
            let _guard = queue.lock().await;
            let _ = started_tx.send(());
            let _ = release_rx.await;
        });

        let _ = started_rx.await;

        let registry_clone = registry.clone();
        let waiter = tokio::spawn(async move {
            let queue2 = core_acquire_project_queue(&registry_clone, key).await;
            let _guard = queue2.lock().await;
            "acquired"
        });

        let blocked = tokio::time::timeout(Duration::from_millis(75), waiter).await;
        assert!(blocked.is_err(), "second request should be queued while first holds lock");

        let _ = release_tx.send(());
        holder.await?;

        // Run a fresh waiter after releasing to confirm queue drains normally.
        let queue3 = core_acquire_project_queue(&registry, "repo:triumvirate".to_string()).await;
        let _guard = queue3.lock().await;
        Ok(())
    }

    #[tokio::test]
    async fn fetch_daemon_ask_agent_uses_bearer_token() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-ask-agent-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "agent-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
        }

        async fn ask_agent_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(req): AxumJson<AskAgentRequest>,
        ) -> Result<AxumJson<AskAgentResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(AskAgentResponse {
                request_id: "daemon-req-1".to_string(),
                agent: req.agent,
                response: format!("daemon echo: {}", req.message),
                lifecycle: vec![LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: "served by daemon".to_string(),
                }],
            }))
        }

        let app = Router::new()
            .route("/ask-agent", post(ask_agent_handler))
            .with_state(TestState {
                token: token.to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_ASK_AGENT_URL",
                format!("http://{addr}/ask-agent"),
            );
        }

        let out = fetch_daemon_ask_agent(&AskAgentRequest {
            agent: "gemini".to_string(),
            message: "run from daemon".to_string(),
            cwd: None,
            repo: None,
            branch: None,
        })
        .await?;
        assert_eq!(out.agent, "gemini");
        assert_eq!(out.response, "daemon echo: run from daemon");
        assert_eq!(out.lifecycle.first().map(|e| e.state.as_str()), Some("DONE"));

        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn mcp_ask_agent_uses_daemon_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-mcp-daemon-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "mcp-daemon-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
        }

        async fn ask_agent_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(req): AxumJson<AskAgentRequest>,
        ) -> Result<AxumJson<AskAgentResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(AskAgentResponse {
                request_id: "daemon-req-3".to_string(),
                agent: req.agent,
                response: "daemon path used".to_string(),
                lifecycle: vec![LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: "daemon served".to_string(),
                }],
            }))
        }

        let app = Router::new()
            .route("/ask-agent", post(ask_agent_handler))
            .with_state(TestState {
                token: token.to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_ASK_AGENT_URL",
                format!("http://{addr}/ask-agent"),
            );
        }

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let args = serde_json::json!({
            "agent": "gemini",
            "message": "should proxy"
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_agent")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(raw_text.contains("daemon path used"));
        assert!(raw_text.contains("daemon served"));

        client.cancel().await?;
        server_handle.await??;
        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn mcp_memory_tools_use_daemon_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-mcp-memory-daemon-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "mcp-memory-daemon-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
            entries: Arc<Mutex<Vec<MemoryEntry>>>,
        }

        async fn memory_write_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(req): AxumJson<MemoryWriteRequest>,
        ) -> Result<AxumJson<MemoryWriteResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let id = "daemon-memory-1".to_string();
            let mut entries = state.entries.lock().expect("entries lock poisoned");
            entries.push(MemoryEntry {
                id: id.clone(),
                namespace: req.namespace,
                key: req.key,
                value: req.value,
                ts_ms: core_unix_time_ms(),
            });
            Ok(AxumJson(MemoryWriteResponse {
                id,
                status: "ok".to_string(),
            }))
        }

        async fn memory_read_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(req): AxumJson<MemoryReadRequest>,
        ) -> Result<AxumJson<MemoryReadResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let entries = state.entries.lock().expect("entries lock poisoned");
            let mut out = entries
                .iter()
                .filter(|e| e.namespace == req.namespace)
                .cloned()
                .collect::<Vec<_>>();
            if let Some(key) = req.key {
                out.retain(|e| e.key == key);
            }
            Ok(AxumJson(MemoryReadResponse { entries: out }))
        }

        let app = Router::new()
            .route("/memory/write", post(memory_write_handler))
            .route("/memory/read", post(memory_read_handler))
            .with_state(TestState {
                token: token.to_string(),
                entries: Arc::new(Mutex::new(Vec::new())),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_MEMORY_WRITE_URL",
                format!("http://{addr}/memory/write"),
            );
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_MEMORY_READ_URL",
                format!("http://{addr}/memory/read"),
            );
        }

        let (server_transport, client_transport) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let write_args = serde_json::json!({
            "namespace": "proj-daemon",
            "key": "decision",
            "value": "ship-it"
        });
        let write = client
            .call_tool(
                CallToolRequestParams::new("memory_write")
                    .with_arguments(write_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;
        let write_text = write
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(write_text.contains("daemon-memory-1"));

        let read_args = serde_json::json!({
            "namespace": "proj-daemon",
            "key": "decision",
            "limit": 10
        });
        let read = client
            .call_tool(
                CallToolRequestParams::new("memory_read")
                    .with_arguments(read_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;
        let read_text = read
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(read_text.contains("ship-it"));

        client.cancel().await?;
        server_handle.await??;
        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_MEMORY_WRITE_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_MEMORY_READ_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn mcp_scratchpad_tools_use_daemon_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home =
            std::env::temp_dir().join(format!("triumvirate-mcp-scratchpad-daemon-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "mcp-scratchpad-daemon-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
            files: Arc<Mutex<Vec<String>>>,
        }

        async fn scratchpad_write_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(req): AxumJson<ScratchpadWriteRequest>,
        ) -> Result<AxumJson<ScratchpadWriteResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let path = format!("/tmp/daemon-scratch/{}/notes.md", req.project);
            state
                .files
                .lock()
                .expect("files lock poisoned")
                .push(path.clone());
            Ok(AxumJson(ScratchpadWriteResponse { path }))
        }

        async fn scratchpad_list_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(_req): AxumJson<ScratchpadListRequest>,
        ) -> Result<AxumJson<ScratchpadListResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let files = state.files.lock().expect("files lock poisoned").clone();
            Ok(AxumJson(ScratchpadListResponse { files }))
        }

        let app = Router::new()
            .route("/scratchpad/write", post(scratchpad_write_handler))
            .route("/scratchpad/list", post(scratchpad_list_handler))
            .with_state(TestState {
                token: token.to_string(),
                files: Arc::new(Mutex::new(Vec::new())),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_SCRATCHPAD_WRITE_URL",
                format!("http://{addr}/scratchpad/write"),
            );
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_SCRATCHPAD_LIST_URL",
                format!("http://{addr}/scratchpad/list"),
            );
        }

        let (server_transport, client_transport) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let write = client
            .call_tool(
                CallToolRequestParams::new("scratchpad_write").with_arguments(
                    serde_json::json!({
                        "project": "daemon-proj",
                        "topic": "notes",
                        "content": "hello"
                    })
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
                ),
            )
            .await?;
        let write_text = write
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(write_text.contains("/tmp/daemon-scratch/daemon-proj/notes.md"));

        let list = client
            .call_tool(
                CallToolRequestParams::new("scratchpad_list").with_arguments(
                    serde_json::json!({ "project": "daemon-proj" })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await?;
        let list_text = list
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(list_text.contains("/tmp/daemon-scratch/daemon-proj/notes.md"));

        client.cancel().await?;
        server_handle.await??;
        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_SCRATCHPAD_WRITE_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_SCRATCHPAD_LIST_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn mcp_fallback_tools_use_daemon_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home =
            std::env::temp_dir().join(format!("triumvirate-mcp-fallback-daemon-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "mcp-fallback-daemon-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
            tickets: Arc<Mutex<Vec<String>>>,
        }

        async fn fallback_list_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(_req): AxumJson<FallbackListRequest>,
        ) -> Result<AxumJson<FallbackListResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let tickets = state
                .tickets
                .lock()
                .expect("tickets lock poisoned")
                .clone();
            Ok(AxumJson(FallbackListResponse { tickets }))
        }

        async fn fallback_ack_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(req): AxumJson<FallbackAckRequest>,
        ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let mut tickets = state.tickets.lock().expect("tickets lock poisoned");
            tickets.retain(|t| t != &req.path);
            Ok(AxumJson(serde_json::json!({
                "status": "ok",
                "message": format!("acknowledged {}", req.path)
            })))
        }

        let app = Router::new()
            .route("/fallback/list", post(fallback_list_handler))
            .route("/fallback/ack", post(fallback_ack_handler))
            .with_state(TestState {
                token: token.to_string(),
                tickets: Arc::new(Mutex::new(vec!["ticket-z.md".to_string()])),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_FALLBACK_LIST_URL",
                format!("http://{addr}/fallback/list"),
            );
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_FALLBACK_ACK_URL",
                format!("http://{addr}/fallback/ack"),
            );
        }

        let (server_transport, client_transport) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let list = client
            .call_tool(
                CallToolRequestParams::new("fallback_list").with_arguments(
                    serde_json::json!({ "limit": 10 })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await?;
        let list_text = list
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(list_text.contains("ticket-z.md"));

        let _ack = client
            .call_tool(
                CallToolRequestParams::new("fallback_ack").with_arguments(
                    serde_json::json!({ "path": "ticket-z.md" })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await?;

        let list_after = client
            .call_tool(
                CallToolRequestParams::new("fallback_list").with_arguments(
                    serde_json::json!({ "limit": 10 })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await?;
        let list_after_text = list_after
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(!list_after_text.contains("ticket-z.md"));

        client.cancel().await?;
        server_handle.await??;
        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_FALLBACK_LIST_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_FALLBACK_ACK_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn mcp_fallback_gc_uses_daemon_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home =
            std::env::temp_dir().join(format!("triumvirate-mcp-fallback-gc-daemon-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "mcp-fallback-gc-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
        }

        async fn fallback_gc_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(_req): AxumJson<FallbackGcRequest>,
        ) -> Result<AxumJson<FallbackGcResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(FallbackGcResponse { removed: 2 }))
        }

        let app = Router::new()
            .route("/fallback/gc", post(fallback_gc_handler))
            .with_state(TestState {
                token: token.to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_FALLBACK_GC_URL",
                format!("http://{addr}/fallback/gc"),
            );
        }

        let (server_transport, client_transport) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let gc = client
            .call_tool(
                CallToolRequestParams::new("fallback_gc").with_arguments(
                    serde_json::json!({ "max_age_days": 7 })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await?;
        let gc_text = gc
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(gc_text.contains("\"removed\":2"));

        client.cancel().await?;
        server_handle.await??;
        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_FALLBACK_GC_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn mcp_outbox_recent_uses_daemon_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-mcp-outbox-daemon-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "mcp-outbox-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
        }

        async fn outbox_recent_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(_req): AxumJson<OutboxRecentRequest>,
        ) -> Result<AxumJson<OutboxRecentResponse>, StatusCode> {
            if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(OutboxRecentResponse {
                events: vec![OutboxEvent {
                    ts_ms: 123,
                    request_id: "daemon-event-1".to_string(),
                    tool: "ask_agent".to_string(),
                    status: "DONE".to_string(),
                    agent: Some("gemini".to_string()),
                    detail: "from daemon".to_string(),
                    cwd: None,
                    repo: None,
                    branch: None,
                }],
            }))
        }

        let app = Router::new()
            .route("/outbox/recent", post(outbox_recent_handler))
            .with_state(TestState {
                token: token.to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_OUTBOX_RECENT_URL",
                format!("http://{addr}/outbox/recent"),
            );
        }

        let (server_transport, client_transport) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            McpBridge::new_ephemeral()
                .serve(server_transport)
                .await?
                .waiting()
                .await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let out = client
            .call_tool(
                CallToolRequestParams::new("outbox_recent").with_arguments(
                    serde_json::json!({ "limit": 5 })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await?;
        let out_text = out
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(out_text.contains("daemon-event-1"));
        assert!(out_text.contains("from daemon"));

        client.cancel().await?;
        server_handle.await??;
        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_OUTBOX_RECENT_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_writes_outbox_events() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-outbox-{now}"));
        fs::create_dir_all(&test_home)?;
        let script_path = write_mock_agent_script("gemini", 0.0)?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let response = execute_ask_agent(&AskAgentRequest {
            agent: "gemini".to_string(),
            message: "outbox check".to_string(),
            cwd: Some("/tmp/project".to_string()),
            repo: Some("triumvirate".to_string()),
            branch: Some("feat/mcp-first".to_string()),
        }, None)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
        assert!(!response.request_id.is_empty());
        assert_eq!(response.agent, "gemini");

        let outbox = fs::read_to_string(test_home.join("outbox.jsonl"))?;
        assert!(outbox.contains("\"tool\":\"ask_agent\""));
        assert!(outbox.contains("\"status\":\"SPAWNED\""));
        assert!(outbox.contains("\"status\":\"DONE\""));
        assert!(outbox.contains(&response.request_id));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_failure_creates_dead_drop_ticket() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-dead-drop-{now}"));
        fs::create_dir_all(&test_home)?;
        let script_path = write_failing_agent_script("gemini")?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let err = execute_ask_agent(&AskAgentRequest {
            agent: "gemini".to_string(),
            message: "should fail".to_string(),
            cwd: Some("/tmp/project".to_string()),
            repo: Some("triumvirate".to_string()),
            branch: Some("feat/mcp-first".to_string()),
        }, None)
        .await
        .err()
        .unwrap_or_default();
        assert!(err.contains("dead drop launched"));

        let dead_drop_dir = test_home.join("dead-drop");
        assert!(dead_drop_dir.exists());
        let tickets = fs::read_dir(&dead_drop_dir)?
            .filter_map(Result::ok)
            .count();
        assert!(tickets >= 1);

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[test]
    fn count_pending_fallbacks_reads_dead_drop_directory() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-fallback-count-{now}"));
        let dead_drop_dir = test_home.join("dead-drop");
        fs::create_dir_all(&dead_drop_dir)?;
        fs::write(dead_drop_dir.join("a.md"), "x")?;
        fs::write(dead_drop_dir.join("b.md"), "x")?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };
        let count = count_pending_fallbacks()?;
        assert_eq!(count, 2);

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn memory_write_and_read_roundtrip() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-memory-{now}"));
        fs::create_dir_all(&test_home)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        let bridge = McpBridge::new_ephemeral();
        let _ = bridge
            .memory_write(Parameters(MemoryWriteRequest {
                namespace: "proj-a".to_string(),
                key: "decision".to_string(),
                value: "use oauth".to_string(),
            }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let read = bridge
            .memory_read(Parameters(MemoryReadRequest {
                namespace: "proj-a".to_string(),
                key: Some("decision".to_string()),
                limit: Some(10),
            }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(read.0.entries.len(), 1);
        assert_eq!(read.0.entries[0].value, "use oauth");

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn scratchpad_write_and_list_roundtrip() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-scratchpad-{now}"));
        fs::create_dir_all(&test_home)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        let bridge = McpBridge::new_ephemeral();
        let write = bridge
            .scratchpad_write(Parameters(ScratchpadWriteRequest {
                project: "tri-project".to_string(),
                topic: "notes".to_string(),
                content: "hello scratchpad".to_string(),
            }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert!(write.0.path.contains("scratchpad"));

        let list = bridge
            .scratchpad_list(Parameters(ScratchpadListRequest {
                project: "tri-project".to_string(),
            }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(list.0.files.len(), 1);
        assert!(list.0.files[0].contains("notes"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn outbox_recent_returns_latest_events() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-outbox-recent-{now}"));
        fs::create_dir_all(&test_home)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        append_outbox_event(&OutboxEvent {
            ts_ms: 10,
            request_id: "a".to_string(),
            tool: "ask_agent".to_string(),
            status: "SPAWNED".to_string(),
            agent: Some("gemini".to_string()),
            detail: "first".to_string(),
            cwd: None,
            repo: None,
            branch: None,
        })?;
        append_outbox_event(&OutboxEvent {
            ts_ms: 20,
            request_id: "b".to_string(),
            tool: "ask_agent".to_string(),
            status: "DONE".to_string(),
            agent: Some("gemini".to_string()),
            detail: "second".to_string(),
            cwd: None,
            repo: None,
            branch: None,
        })?;

        let bridge = McpBridge::new_ephemeral();
        let out = bridge
            .outbox_recent(Parameters(OutboxRecentRequest { limit: Some(1) }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(out.0.events.len(), 1);
        assert_eq!(out.0.events[0].request_id, "b");

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn fallback_list_and_ack_roundtrip() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-fallback-tools-{now}"));
        let dead_drop = test_home.join("dead-drop");
        fs::create_dir_all(&dead_drop)?;
        let ticket = dead_drop.join("ticket-a.md");
        fs::write(&ticket, "fallback ticket")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        let bridge = McpBridge::new_ephemeral();
        let list = bridge
            .fallback_list(Parameters(FallbackListRequest { limit: Some(10) }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(list.0.tickets.len(), 1);
        assert!(list.0.tickets[0].contains("ticket-a.md"));

        let _ = bridge
            .fallback_ack(Parameters(FallbackAckRequest {
                path: list.0.tickets[0].clone(),
            }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let list_after = bridge
            .fallback_list(Parameters(FallbackListRequest { limit: Some(10) }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(list_after.0.tickets.len(), 0);

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[test]
    fn fallback_ack_rejects_paths_outside_dead_drop() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-fallback-security-{now}"));
        let dead_drop = test_home.join("dead-drop");
        fs::create_dir_all(&dead_drop)?;
        let outside = test_home.join("outside.md");
        fs::write(&outside, "outside")?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };
        let err = acknowledge_fallback_path(&outside.display().to_string())
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("outside dead-drop"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn fallback_gc_removes_stale_tickets() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-fallback-gc-{now}"));
        let dead_drop = test_home.join("dead-drop");
        fs::create_dir_all(&dead_drop)?;
        fs::write(dead_drop.join("stale-a.md"), "old")?;
        fs::write(dead_drop.join("stale-b.md"), "old")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        // With max_age_days=0 all existing tickets are eligible for removal.
        let bridge = McpBridge::new_ephemeral();
        let out = bridge
            .fallback_gc(Parameters(FallbackGcRequest {
                max_age_days: Some(0),
            }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert!(out.0.removed >= 2);

        let list_after = bridge
            .fallback_list(Parameters(FallbackListRequest { limit: Some(10) }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(list_after.0.tickets.len(), 0);

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn sessions_persist_across_bridge_instances() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-sessions-{now}"));
        fs::create_dir_all(&test_home)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &test_home) };

        let first = McpBridge::new();
        {
            let mut sessions = first.sessions.lock().await;
            sessions.insert(
                "persisted".to_string(),
                SessionState {
                    agent: "gemini".to_string(),
                    cwd: None,
                    history: vec!["hello".to_string()],
                },
            );
            core_persist_json_file_if_enabled(first.sessions_file.as_ref(), &*sessions)?;
        }

        let second = McpBridge::new();
        let sessions = second.sessions.lock().await;
        let persisted = sessions.get("persisted");
        assert!(persisted.is_some());
        assert_eq!(persisted.map(|s| s.agent.as_str()), Some("gemini"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }
}
