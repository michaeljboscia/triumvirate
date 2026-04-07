use clap::{Parser, Subcommand};
use agent_worker::{
    WorkerAcquireMode, acquire_worker, dismiss_worker,
};
#[cfg(test)]
use agent_worker::{reset_worker_registry_for_tests, update_worker_session};
use daemon_core::{
    QueueRegistry,
    acquire_project_queue as core_acquire_project_queue,
    append_memory_entry as core_append_memory_entry,
    daemon_bind_addr as core_daemon_bind_addr,
    list_scratchpad as core_list_scratchpad, project_queue_key as core_project_queue_key,
    read_memory_entries as core_read_memory_entries,
    triumvirate_home_dir as core_triumvirate_home_dir,
    unix_time_ms as core_unix_time_ms, write_scratchpad as core_write_scratchpad,
    ensure_daemon_token as core_ensure_daemon_token,
    sessions_file_path as core_sessions_file_path,
    load_json_file_if_exists as core_load_json_file_if_exists,
    persist_json_file_if_enabled as core_persist_json_file_if_enabled,
};
#[cfg(test)]
use daemon_core::render_launch_agent_plist as core_render_launch_agent_plist;
use daemon_http::{
    fetch_daemon_ask_agent, fetch_daemon_fallback_ack,
    fetch_daemon_fallback_gc, fetch_daemon_fallback_list, fetch_daemon_memory_read,
    fetch_daemon_memory_write, fetch_daemon_outbox_recent, fetch_daemon_scratchpad_list,
    fetch_daemon_scratchpad_write, fetch_daemon_session_ask, fetch_daemon_session_dismiss,
    fetch_daemon_session_list, fetch_daemon_session_spawn, fetch_daemon_status,
    fetch_daemon_status_snapshot,
};
#[cfg(test)]
use daemon_http::{attempt_daemon_autostart_once, reset_daemon_autostart_flag_for_tests};
use fallback_outbox::{
    acknowledge_fallback_path, append_outbox_event, count_pending_fallbacks, gc_fallbacks,
    list_pending_fallback_paths, read_outbox_events, spawn_dead_drop as create_dead_drop_fallback,
};
use mcp_bridge::{
    is_bearer_authorized, is_supported_agent_name,
};
#[cfg(not(test))]
use mcp_bridge::use_daemon_for_mcp_from_env;
#[cfg(test)]
use mcp_bridge::should_use_daemon_proxy;
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
    FallbackListResponse, MemoryEntry, MemoryReadRequest, MemoryReadResponse,
    MemoryWriteRequest, MemoryWriteResponse, OutboxRecentRequest,
    OutboxRecentResponse, SessionInfo, SessionListResponse, SpawnSessionRequest,
    ScratchpadListRequest, ScratchpadListResponse, ScratchpadWriteRequest,
    ScratchpadWriteResponse, StatusResponse, DaemonHealthResponse,
    SessionState,
};
#[cfg(test)]
use shared_types::{DaemonStatusSnapshot, LifecycleEvent, OutboxEvent};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
    },
};
use tokio::{
    sync::Mutex,
    time::{Duration, Instant, sleep},
};
use tracing::info;
use uuid::Uuid;

mod agent_exec;
mod cli_ops;
mod tracing_setup;

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
        should_use_daemon_proxy(std::env::var("TRIUMVIRATE_MCP_USE_DAEMON").ok().as_deref())
    }
    #[cfg(not(test))]
    {
        use_daemon_for_mcp_from_env()
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
    agent_exec::execute_ask_agent(req, progress).await
}

async fn prewarm_daemon_workers() {
    agent_exec::prewarm_daemon_workers().await;
}

#[tool_handler]
impl ServerHandler for McpBridge {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Triumvirate MCP bridge. Use `ping` to verify connectivity.")
    }
}

fn init_tracing() -> anyhow::Result<()> {
    tracing_setup::init_tracing()
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
    cli_ops::run_install()
}

fn run_uninstall() -> anyhow::Result<()> {
    cli_ops::run_uninstall()
}

async fn run_doctor() -> anyhow::Result<()> {
    cli_ops::run_doctor().await
}

async fn run_status() -> anyhow::Result<()> {
    cli_ops::run_status().await
}

#[cfg(test)]
fn build_status_report(
    daemon_bind_addr: String,
    health: Option<DaemonHealthResponse>,
    snapshot: Option<DaemonStatusSnapshot>,
    pending_fallbacks: usize,
    fallback_tickets: Vec<String>,
) -> serde_json::Value {
    cli_ops::build_status_report(
        daemon_bind_addr,
        health,
        snapshot,
        pending_fallbacks,
        fallback_tickets,
    )
}

async fn run_daemon() -> anyhow::Result<()> {
    #[derive(Debug, Clone)]
    struct DaemonMetrics {
        registry: Registry,
        agent_requests_total: prometheus::IntCounter,
        agent_duration_seconds: prometheus::Histogram,
        agent_tokens_total: prometheus::IntCounter,
        ledger_events_ingested_total: prometheus::IntCounter,
        ledger_queue_lag_seconds: prometheus::Gauge,
        ledger_spool_size_bytes: IntGauge,
        fleet_active_total: IntGauge,
        reviews_total: prometheus::IntCounter,
        marker_parse_success_rate: prometheus::Gauge,
        http_requests_total: IntCounterVec,
        http_request_duration_seconds: HistogramVec,
        http_in_flight_requests: IntGauge,
    }

    impl DaemonMetrics {
        fn new() -> anyhow::Result<Self> {
            let registry = Registry::new();
            let agent_requests_total = prometheus::IntCounter::new(
                "triumvirate_agent_requests_total",
                "Total ask_agent requests handled by daemon",
            )?;
            let agent_duration_seconds = prometheus::Histogram::with_opts(
                prometheus::HistogramOpts::new(
                    "triumvirate_agent_duration_seconds",
                    "Duration of ask_agent requests in seconds",
                ),
            )?;
            let agent_tokens_total = prometheus::IntCounter::new(
                "triumvirate_agent_tokens_total",
                "Total tokens reported by ask_agent requests",
            )?;
            let ledger_events_ingested_total = prometheus::IntCounter::new(
                "triumvirate_ledger_events_ingested_total",
                "Total ledger events ingested",
            )?;
            let ledger_queue_lag_seconds = prometheus::Gauge::new(
                "triumvirate_ledger_queue_lag_seconds",
                "Ledger queue lag in seconds",
            )?;
            let ledger_spool_size_bytes = IntGauge::new(
                "triumvirate_ledger_spool_size_bytes",
                "Current ledger spool directory size in bytes",
            )?;
            let fleet_active_total = IntGauge::new(
                "triumvirate_fleet_active_total",
                "Active fleet count",
            )?;
            let reviews_total = prometheus::IntCounter::new(
                "triumvirate_reviews_total",
                "Total reviews completed",
            )?;
            let marker_parse_success_rate = prometheus::Gauge::new(
                "triumvirate_marker_parse_success_rate",
                "Marker parse success rate",
            )?;
            let http_requests_total = IntCounterVec::new(
                prometheus::Opts::new("triumvirate_http_requests_total", "HTTP requests by route and status"),
                &["route", "status"],
            )?;
            let http_request_duration_seconds = HistogramVec::new(
                prometheus::HistogramOpts::new(
                    "triumvirate_http_request_duration_seconds",
                    "HTTP request durations by route",
                ),
                &["route"],
            )?;
            let http_in_flight_requests = IntGauge::new(
                "triumvirate_http_requests_in_flight",
                "In-flight HTTP requests",
            )?;
            registry.register(Box::new(agent_requests_total.clone()))?;
            registry.register(Box::new(agent_duration_seconds.clone()))?;
            registry.register(Box::new(agent_tokens_total.clone()))?;
            registry.register(Box::new(ledger_events_ingested_total.clone()))?;
            registry.register(Box::new(ledger_queue_lag_seconds.clone()))?;
            registry.register(Box::new(ledger_spool_size_bytes.clone()))?;
            registry.register(Box::new(fleet_active_total.clone()))?;
            registry.register(Box::new(reviews_total.clone()))?;
            registry.register(Box::new(marker_parse_success_rate.clone()))?;
            registry.register(Box::new(http_requests_total.clone()))?;
            registry.register(Box::new(http_request_duration_seconds.clone()))?;
            registry.register(Box::new(http_in_flight_requests.clone()))?;
            marker_parse_success_rate.set(1.0);
            Ok(Self {
                registry,
                agent_requests_total,
                agent_duration_seconds,
                agent_tokens_total,
                ledger_events_ingested_total,
                ledger_queue_lag_seconds,
                ledger_spool_size_bytes,
                fleet_active_total,
                reviews_total,
                marker_parse_success_rate,
                http_requests_total,
                http_request_duration_seconds,
                http_in_flight_requests,
            })
        }

        fn snapshot_keepalive(&self) {
            let _ = self.agent_tokens_total.get();
            let _ = self.ledger_events_ingested_total.get();
            let _ = self.ledger_queue_lag_seconds.get();
            let _ = self.ledger_spool_size_bytes.get();
            let _ = self.fleet_active_total.get();
            let _ = self.reviews_total.get();
            let _ = self.marker_parse_success_rate.get();
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
        state.metrics.snapshot_keepalive();
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
        state.metrics.http_in_flight_requests.inc();
        let started = Instant::now();
        let response = next.run(request).await;
        let elapsed = started.elapsed().as_secs_f64();
        let status = response.status().as_u16().to_string();
        state
            .metrics
            .http_requests_total
            .with_label_values(&[route.as_str(), status.as_str()])
            .inc();
        state
            .metrics
            .http_request_duration_seconds
            .with_label_values(&[route.as_str()])
            .observe(elapsed);
        state.metrics.http_in_flight_requests.dec();
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
        let started = Instant::now();
        let result = execute_ask_agent(&req, None).await;
        state.metrics.agent_requests_total.inc();
        state.metrics.agent_duration_seconds.observe(started.elapsed().as_secs_f64());
        match result {
            Ok(response) => Ok(AxumJson(response)),
            Err(e) => Err((
                StatusCode::BAD_GATEWAY,
                AxumJson(serde_json::json!({ "error": e })),
            )),
        }
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
    info!(%bind_addr, "starting triumvirate daemon");
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
    info!(%bind_addr, "daemon listener bound");
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
#[allow(clippy::await_holding_lock)]
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
