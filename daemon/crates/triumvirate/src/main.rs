// Pre-existing lint debt acknowledged in PR #29. Each allow has a tracking
// issue for follow-up cleanup; remove the allow once the underlying lint is fixed.
#![allow(clippy::collapsible_if, clippy::too_many_arguments, dead_code)]

use clap::{Parser, Subcommand};
use agent_worker::{
    WorkerAcquireMode, acquire_worker, dismiss_worker,
};
#[cfg(test)]
use agent_worker::{reset_worker_registry_for_tests, update_worker_session};
use daemon_core::{
    DaemonState,
    daemon_bind_addr as core_daemon_bind_addr,
    observability::ObservabilityBus,
    publish_ws_event,
    metrics::DaemonMetrics,
    triumvirate_home_dir as core_triumvirate_home_dir,
    ensure_daemon_token as core_ensure_daemon_token,
    sessions_file_path as core_sessions_file_path,
    load_json_file_if_exists as core_load_json_file_if_exists,
    persist_json_file_if_enabled as core_persist_json_file_if_enabled,
};
#[cfg(test)]
#[cfg(test)]
use daemon_core::render_launch_agent_plist as core_render_launch_agent_plist;
#[cfg(test)]
use daemon_http::{fetch_daemon_ask_agent, fetch_daemon_status};
#[cfg(test)]
use daemon_http::{attempt_daemon_autostart_once, reset_daemon_autostart_flag_for_tests};
use daemon_http::DaemonHttpState;
use fallback_outbox::{
    append_outbox_event, count_pending_fallbacks, list_pending_fallback_paths,
    spawn_dead_drop as create_dead_drop_fallback,
};
use ledger::LedgerStore;
#[cfg(test)]
use peer_review::PeerReviewEngine;
use mcp_bridge::{
    codex_command, is_bearer_authorized, is_supported_agent_name, normalize_agent_name,
};
#[cfg(not(test))]
use mcp_bridge::use_daemon_for_mcp_from_env;
#[cfg(test)]
use mcp_bridge::should_use_daemon_proxy;
use mcp_tools::{
    aliases as mcp_aliases,
    ProgressEmitter, fleet as mcp_fleet,
    gemini_query as mcp_gemini_query, knowledge, review as mcp_review,
    token_tools as mcp_token_tools,
};
use axum::{
    Json as AxumJson, Router,
    body::Body,
    extract::State,
    handler::Handler,
    http::{HeaderMap, Request, StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::Response,
    routing::{get, get_service, post, post_service},
};
use rmcp::{
    ErrorData as McpError, Json, ServerHandler, ServiceExt,
    handler::server::{
        router::tool::ToolRouter,
        tool::ToolCallContext,
        wrapper::Parameters,
    },
    model::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
    tool, tool_router,
    transport::stdio,
};
use shared_types::{
    AskAgentRequest, AskAgentResponse, AskSessionRequest,
    CancelTaskRequest as AbeCancelTaskRequest, CancelTaskResponse as AbeCancelTaskResponse,
    DispatchCodexRequest, DispatchCodexResponse,
    DispatchCodexWorktreeRequest, DispatchCodexWorktreeResponse, GetTaskOutputRequest,
    GetTaskOutputResponse, GetTaskStatusRequest, GetTaskStatusResponse,
    QueryGeminiRequest, QueryGeminiResponse, QueryGeminiReviewRequest, QueryGeminiReviewResponse,
    TaskCompleteRequest,
    DismissSessionRequest,
    FallbackAckRequest, FallbackGcRequest, FallbackGcResponse, FallbackListRequest,
    FallbackListResponse, LedgerQueryRequest, LedgerQueryResponse, LedgerSessionRequest,
    FleetCancelRequest, FleetCancelResponse, FleetClaimTaskRequest, FleetClaimTaskResponse,
    FleetSpawnRequest, FleetSpawnResponse, FleetStatusRequest, FleetStatusResponse, FleetTaskListRequest,
    FleetTaskListResponse,
    ReviewRequestResponse, ReviewRequestTool, ReviewStatusRequest, ReviewStatusResponse,
    ReviewSubmitRequest,
    LessonAddResponse, LessonListRequest, LessonListResponse, LessonQueryRequest, LessonQueryResponse,
    LessonValidateRequest, ManualRecord,
    MemoryReadRequest, MemoryReadResponse,
    NewLesson,
    MemoryWriteRequest, MemoryWriteResponse, OutboxRecentRequest,
    OutboxRecentResponse, SessionInfo, SessionListResponse, SpawnSessionRequest,
    ScratchpadListRequest, ScratchpadListResponse, ScratchpadWriteRequest,
    ScratchpadWriteResponse, StatusResponse, DaemonHealthResponse,
    GcResult, HealthStatus, SessionDetail,
    SessionState,
};
use token_economics::{TokenDb, TokenRecord};
#[cfg(test)]
use shared_types::GeminiReviewVerdict;
#[cfg(test)]
use shared_types::{DaemonStatusSnapshot, LifecycleEvent, OutboxEvent};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::{
        Arc,
        OnceLock,
    },
};
use tokio::{
    sync::{Mutex, broadcast},
    time::{Duration, Instant, sleep},
};
use tracing::{info, warn};
use uuid::Uuid;

mod agent_exec;
mod agy;
mod streaming;
mod http_mcp;
mod abe;
mod cli_ops;
mod git_ops_impl;
mod proxy;
mod tracing_setup;
mod watch;

#[derive(Debug, Parser)]
#[command(name = "triumvirate")]
#[command(about = "Triumvirate v2 daemon + MCP bridge binary")]
#[command(version)]
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
    /// Bridge stdio MCP to daemon HTTP endpoint.
    Proxy,
    /// Watch live agent streaming events.
    Watch(watch::WatchArgs),
}

#[derive(Debug, Clone)]
struct McpBridge {
    tool_router: ToolRouter<Self>,
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    sessions_file: Option<PathBuf>,
    fleet_states: Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    abe_tasks: abe::task_tracker::TaskTracker,
    metrics: Arc<DaemonMetrics>,
    ws_events: broadcast::Sender<String>,
    token_db: Option<Arc<TokenDb>>,
}

static PROCESS_METRICS: OnceLock<Arc<DaemonMetrics>> = OnceLock::new();
static PROCESS_TOKEN_DB: OnceLock<Arc<TokenDb>> = OnceLock::new();

pub(crate) fn process_metrics() -> Option<&'static Arc<DaemonMetrics>> {
    PROCESS_METRICS.get()
}

pub(crate) fn process_token_db() -> Option<&'static Arc<TokenDb>> {
    PROCESS_TOKEN_DB.get()
}

fn set_process_metrics(metrics: Arc<DaemonMetrics>) {
    let _ = PROCESS_METRICS.set(metrics);
}

fn init_process_token_db() -> anyhow::Result<Arc<TokenDb>> {
    if let Some(existing) = PROCESS_TOKEN_DB.get() {
        return Ok(existing.clone());
    }

    let home = core_triumvirate_home_dir()?;
    let db_path = home.join("token-economics.db");
    let db = token_economics::open(&db_path)?;
    // T-003 (REQ-DS-009): seed the DeepSeek price rows on first run so the runner's
    // synchronous per-consult cost calc (calculate_cost_usd) finds them. Idempotent —
    // safe to call on every daemon boot.
    if let Err(err) = token_economics::ensure_deepseek_prices(&db) {
        tracing::warn!(?err, "deepseek price seeding failed; per-consult cost calc may return None until resolved");
    }
    let db = Arc::new(db);
    let _ = PROCESS_TOKEN_DB.set(db.clone());
    Ok(db)
}

pub(crate) fn record_daemon_tokens(db: &TokenDb, record: &TokenRecord) -> Result<(), String> {
    if record.agent.trim().is_empty() {
        return Err("token record agent must be non-empty".to_string());
    }
    if record.session_id.trim().is_empty() {
        return Err("token record session_id must be non-empty".to_string());
    }
    token_economics::record_daemon_tokens(db, record.clone()).map_err(|err| err.to_string())
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

fn daemon_bind_port(bind_addr: &str) -> Option<u16> {
    bind_addr.rsplit(':').next()?.parse::<u16>().ok()
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
        let metrics = Arc::new(DaemonMetrics::new().expect("failed to initialize daemon metrics"));
        let ws_events = broadcast::channel(256).0;
        set_process_metrics(metrics.clone());
        if let Err(err) = init_process_token_db() {
            warn!("failed to initialize token DB for MCP bridge: {err}");
        }
        let token_db = process_token_db().cloned();
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
            fleet_states: Arc::new(Mutex::new(HashMap::new())),
            abe_tasks: abe::task_tracker::TaskTracker::with_observability(
                metrics.clone(),
                Some(ws_events.clone()),
            ),
            metrics,
            ws_events,
            token_db,
        }
    }

    fn abe_callbacks(&self) -> mcp_tools::abe::AbeCallbacks {
        let metrics = self.metrics.clone();
        let setup_metrics = metrics.clone();
        let timeout_metrics = metrics.clone();
        let validate_metrics = metrics.clone();
        mcp_tools::abe::AbeCallbacks {
            metrics: metrics.clone(),
            codex_command: Arc::new(codex_command),
            setup_worktree: Arc::new(move |req| {
                abe::worktree_setup::setup_worktree_with_metrics(
                    &abe::worktree_setup::WorktreeSetupRequest {
                    project_root: req.project_root,
                    sha: req.sha,
                    task_id: req.task_id,
                    briefing_content: req.briefing_content,
                    contract_fields: req.contract_fields,
                    },
                    Some(setup_metrics.as_ref()),
                )
                .map(|result| mcp_tools::abe::WorktreeSetupResult {
                    worktree_path: result.worktree_path,
                })
                .map_err(|e| e.to_string())
            }),
            spawn_background: Arc::new(|spec| {
                Box::pin(async move {
                    abe::codex_spawn::spawn_background(abe::codex_spawn::SpawnSpec {
                        cmd: spec.cmd,
                        args: spec.args,
                        cwd: spec.cwd,
                        envs: spec.envs,
                        output_log_dir: spec.output_log_dir,
                    })
                    .await
                    .map_err(|e| e.to_string())
                })
            }),
            enforce_timeout: Arc::new(move |child, timeout_sec, cwd| {
                let metrics = timeout_metrics.clone();
                Box::pin(async move {
                    abe::codex_spawn::enforce_timeout_with_metrics(
                        child,
                        timeout_sec,
                        &cwd,
                        Some(metrics.as_ref()),
                    )
                        .await
                        .map_err(|e| e.to_string())
                })
            }),
            resolve_commit_outputs: Arc::new(abe::codex_spawn::resolve_commit_outputs),
            validate_commit: Arc::new(move |worktree_path, contract, start_sha| {
                let result = abe::post_exit_validator::validate_commit_with_metrics(
                    worktree_path,
                    contract,
                    start_sha,
                    Some(validate_metrics.as_ref()),
                );
                mcp_tools::abe::PostExitValidation {
                    passed: result.passed,
                    violations: result.violations,
                }
            }),
            rollback_worktree: Arc::new(|project_root, worktree_path| {
                abe::worktree_setup::rollback_worktree(project_root, worktree_path)
                    .map_err(|e| e.to_string())
            }),
            completion_env: Arc::new(|| {
                let mut envs = HashMap::new();
                if let Ok(home) = core_triumvirate_home_dir() {
                    if let Ok(token) = core_ensure_daemon_token(&home) {
                        envs.insert("TRIUMVIRATE_TOKEN".to_string(), token);
                    }
                }
                let bind_addr =
                    core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());
                if let Some(port) = daemon_bind_port(&bind_addr) {
                    envs.insert("TRIUMVIRATE_HTTP_PORT".to_string(), port.to_string());
                }
                envs
            }),
        }
    }
}

impl mcp_tools::abe::AbeTaskTracker for abe::task_tracker::TaskTracker {
    fn register(
        &self,
        task_id: String,
        wave: u32,
        child: Arc<Mutex<tokio::process::Child>>,
        worktree_path: Option<PathBuf>,
        parent_session_id: Option<String>,
        root_session_id: Option<String>,
        dispatch_surface: Option<&'static str>,
        dispatch_repo: Option<String>,
        dispatch_started_at: std::time::Instant,
    ) -> mcp_tools::abe::BoxFuture<()> {
        let tracker = self.clone();
        Box::pin(async move {
            tracker
                .register(
                    task_id,
                    wave,
                    child,
                    worktree_path,
                    parent_session_id,
                    root_session_id,
                    dispatch_surface,
                    dispatch_repo,
                    dispatch_started_at,
                )
                .await
        })
    }

    fn mark_completed(
        &self,
        task_id: String,
        commit_sha: String,
        modified_files: Vec<String>,
        stdout: String,
        validation_log: Option<String>,
        test_output: Option<String>,
    ) -> mcp_tools::abe::BoxFuture<bool> {
        let tracker = self.clone();
        Box::pin(async move {
            tracker
                .mark_completed(
                    &task_id,
                    commit_sha,
                    modified_files,
                    stdout,
                    validation_log,
                    test_output,
                )
                .await
                == abe::task_tracker::TransitionOutcome::Transitioned
        })
    }

    fn mark_failed(
        &self,
        task_id: String,
        exit_code: Option<i32>,
        error_message: String,
    ) -> mcp_tools::abe::BoxFuture<bool> {
        let tracker = self.clone();
        Box::pin(async move {
            tracker.mark_failed(&task_id, exit_code, error_message).await
                == abe::task_tracker::TransitionOutcome::Transitioned
        })
    }

    fn mark_timeout(&self, task_id: String) -> mcp_tools::abe::BoxFuture<bool> {
        let tracker = self.clone();
        Box::pin(async move {
            tracker.mark_timeout(&task_id).await
                == abe::task_tracker::TransitionOutcome::Transitioned
        })
    }

    fn mark_stuck(&self, task_id: String, error_message: String) -> mcp_tools::abe::BoxFuture<bool> {
        let tracker = self.clone();
        Box::pin(async move {
            tracker.mark_stuck(&task_id, error_message).await
                == abe::task_tracker::TransitionOutcome::Transitioned
        })
    }

    fn register_setup_failed(
        &self,
        task_id: String,
        error_message: String,
    ) -> mcp_tools::abe::BoxFuture<()> {
        let tracker = self.clone();
        Box::pin(async move { tracker.register_setup_failed(task_id, error_message).await })
    }

    fn get_status(&self, task_id: String) -> mcp_tools::abe::BoxFuture<Option<GetTaskStatusResponse>> {
        let tracker = self.clone();
        Box::pin(async move { tracker.get_status(&task_id).await })
    }

    fn get_output(&self, task_id: String) -> mcp_tools::abe::BoxFuture<Option<GetTaskOutputResponse>> {
        let tracker = self.clone();
        Box::pin(async move { tracker.get_output(&task_id).await })
    }

    fn cancel(&self, task_id: String) -> mcp_tools::abe::BoxFuture<Option<AbeCancelTaskResponse>> {
        let tracker = self.clone();
        Box::pin(async move { tracker.cancel(&task_id).await })
    }
}

#[tool_router]
impl McpBridge {
    #[tool(description = "Health check tool for MCP connectivity")]
    async fn ping(&self) -> String {
        "pong".to_string()
    }

    #[tool(description = "Send a task to a specific agent. Supported: 'antigravity' (aliases: agy, gemini), 'codex', 'deepseek', 'claude'.")]
    // The root span of a call, in the MCP process. daemon-http injects this span's context as a
    // W3C `traceparent`, and the daemon adopts it — so one logical call is ONE trace across two
    // processes instead of two unrelated ones. Without a span here there is nothing to inject.
    #[tracing::instrument(skip_all, name = "mcp_ask_agent")]
    async fn ask_agent(
        &self,
        Parameters(req): Parameters<AskAgentRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<AskAgentResponse>, String> {
        let local_test_execution_allowed = cfg!(test) && !mcp_daemon_proxy_enabled();
        mcp_tools::inter_agent::ask_agent(
            &req,
            &context,
            local_test_execution_allowed,
            execute_ask_agent_boxed,
        )
            .await
    }

    #[tool(description = "Create a persistent named session for an agent.")]
    async fn spawn_session(
        &self,
        Parameters(req): Parameters<SpawnSessionRequest>,
    ) -> Result<String, String> {
        mcp_tools::inter_agent::spawn_session(
            &self.sessions,
            self.sessions_file.as_ref(),
            &req,
            mcp_daemon_proxy_enabled(),
        )
        .await
    }

    #[tool(description = "Ask within a named persistent session.")]
    async fn ask_session(
        &self,
        Parameters(req): Parameters<AskSessionRequest>,
    ) -> Result<String, String> {
        mcp_tools::inter_agent::ask_session(
            &self.sessions,
            self.sessions_file.as_ref(),
            &req,
            mcp_daemon_proxy_enabled(),
            execute_ask_agent_boxed,
        )
        .await
    }

    #[tool(description = "Dismiss a named session.")]
    async fn dismiss_session(
        &self,
        Parameters(req): Parameters<DismissSessionRequest>,
    ) -> Result<String, String> {
        mcp_tools::inter_agent::dismiss_session(
            &self.sessions,
            self.sessions_file.as_ref(),
            &req,
            mcp_daemon_proxy_enabled(),
        )
        .await
    }

    #[tool(description = "List active sessions.")]
    async fn list_sessions(&self) -> Json<SessionListResponse> {
        mcp_tools::inter_agent::list_sessions(&self.sessions, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Get current system status snapshot.")]
    async fn get_status(&self) -> Json<StatusResponse> {
        mcp_tools::inter_agent::get_status(&self.sessions, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Alias for spawn_session using legacy spawn_daemon schema.")]
    async fn spawn_daemon(
        &self,
        Parameters(req): Parameters<mcp_aliases::SpawnDaemonParams>,
    ) -> Result<String, String> {
        info!(old_name = "spawn_daemon", new_name = "spawn_session", "tool_alias");
        let mapped = mcp_aliases::map_spawn_daemon_params(req).map_err(|e| e.to_string())?;
        let name = mapped
            .name
            .ok_or_else(|| mcp_aliases::AliasMappingError::MissingRequired("session_name").to_string())?;
        self.spawn_session(Parameters(SpawnSessionRequest {
            agent: mapped.agent,
            name,
            cwd: mapped.cwd,
        }))
        .await
    }

    #[tool(description = "Alias for ask_session using legacy ask_daemon schema.")]
    async fn ask_daemon(
        &self,
        Parameters(req): Parameters<mcp_aliases::AskDaemonParams>,
    ) -> Result<String, String> {
        info!(old_name = "ask_daemon", new_name = "ask_session", "tool_alias");
        let mapped = mcp_aliases::map_ask_daemon_params(req).map_err(|e| e.to_string())?;
        self.ask_session(Parameters(AskSessionRequest {
            name: mapped.name,
            message: mapped.message,
        }))
        .await
    }

    #[tool(description = "Alias for dismiss_session using legacy dismiss_daemon schema.")]
    async fn dismiss_daemon(
        &self,
        Parameters(req): Parameters<mcp_aliases::DismissDaemonParams>,
    ) -> Result<String, String> {
        info!(old_name = "dismiss_daemon", new_name = "dismiss_session", "tool_alias");
        let mapped = mcp_aliases::map_dismiss_daemon_params(req).map_err(|e| e.to_string())?;
        self.dismiss_session(Parameters(DismissSessionRequest { name: mapped.name }))
            .await
    }

    #[tool(description = "Alias for list_sessions using legacy list_daemons schema.")]
    async fn list_daemons(
        &self,
        Parameters(req): Parameters<mcp_aliases::ListDaemonsParams>,
    ) -> Result<Json<SessionListResponse>, String> {
        info!(old_name = "list_daemons", new_name = "list_sessions", "tool_alias");
        let mapped = mcp_aliases::map_list_daemons_params(req).map_err(|e| e.to_string())?;
        let mut response = self.list_sessions().await.0;
        if let Some(target) = mapped.target {
            response.sessions.retain(|session| session.agent == target);
        }
        Ok(Json(response))
    }

    #[tool(description = "Alias for ask_session using legacy send_message schema.")]
    async fn send_message(
        &self,
        Parameters(req): Parameters<mcp_aliases::SendMessageParams>,
    ) -> Result<String, String> {
        info!(old_name = "send_message", new_name = "ask_session", "tool_alias");
        let mapped = mcp_aliases::map_send_message_params(req).map_err(|e| e.to_string())?;
        let ask_req = AskSessionRequest {
            name: mapped.name.clone(),
            message: mapped.message,
        };
        match self.ask_session(Parameters(ask_req.clone())).await {
            Ok(response) => Ok(response),
            Err(err) if err.contains("not found") => {
                self.spawn_session(Parameters(SpawnSessionRequest {
                    agent: mapped.name.clone(),
                    name: mapped.name,
                    cwd: None,
                }))
                .await?;
                self.ask_session(Parameters(ask_req)).await
            }
            Err(err) => Err(err),
        }
    }

    #[tool(description = "Alias shim for get_response; use ask_session directly.")]
    async fn get_response(
        &self,
        Parameters(req): Parameters<mcp_aliases::GetResponseParams>,
    ) -> Result<String, String> {
        info!(old_name = "get_response", new_name = "ask_session", "tool_alias");
        let mapped = mcp_aliases::map_get_response_params(req).map_err(|e| e.to_string())?;
        Ok(mapped.message)
    }

    #[tool(description = "Alias for get_status using legacy list_jobs schema.")]
    async fn list_jobs(
        &self,
        Parameters(req): Parameters<mcp_aliases::ListJobsParams>,
    ) -> Result<Json<StatusResponse>, String> {
        info!(old_name = "list_jobs", new_name = "get_status", "tool_alias");
        let _mapped = mcp_aliases::map_list_jobs_params(req).map_err(|e| e.to_string())?;
        Ok(self.get_status().await)
    }

    #[tool(description = "Query daemon HTTP status using local bearer token.")]
    async fn daemon_health(&self) -> Result<Json<DaemonHealthResponse>, String> {
        mcp_tools::inter_agent::daemon_health().await
    }

    #[tool(description = "Return token summary across agents with optional since/until/agent filters.")]
    async fn get_token_summary(
        &self,
        Parameters(req): Parameters<mcp_token_tools::GetTokenSummaryRequest>,
    ) -> Result<Json<mcp_token_tools::GetTokenSummaryResponse>, String> {
        let db_path = daemon_core::triumvirate_home_dir()
            .map(|p| p.join("token-economics.db"))
            .map_err(|e| format!("token db path: {e}"))?;
        mcp_token_tools::get_token_summary(&db_path, req).map(Json)
    }

    #[tool(description = "Return per-task token and cost breakdown for a build_id.")]
    async fn get_build_cost(
        &self,
        Parameters(req): Parameters<mcp_token_tools::GetBuildCostRequest>,
    ) -> Result<Json<mcp_token_tools::GetBuildCostResponse>, String> {
        let db_path = daemon_core::triumvirate_home_dir()
            .map(|p| p.join("token-economics.db"))
            .map_err(|e| format!("token db path: {e}"))?;
        mcp_token_tools::get_build_cost(&db_path, req).map(Json)
    }

    #[tool(description = "Write a shared memory entry.")]
    async fn memory_write(
        &self,
        Parameters(req): Parameters<MemoryWriteRequest>,
    ) -> Result<Json<MemoryWriteResponse>, String> {
        knowledge::memory_write(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Read shared memory entries.")]
    async fn memory_read(
        &self,
        Parameters(req): Parameters<MemoryReadRequest>,
    ) -> Result<Json<MemoryReadResponse>, String> {
        knowledge::memory_read(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Write a scratchpad file in the shared workspace.")]
    async fn scratchpad_write(
        &self,
        Parameters(req): Parameters<ScratchpadWriteRequest>,
    ) -> Result<Json<ScratchpadWriteResponse>, String> {
        knowledge::scratchpad_write(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Alias for scratchpad_write using legacy write_scratchpad schema.")]
    async fn write_scratchpad(
        &self,
        Parameters(req): Parameters<mcp_aliases::WriteScratchpadParams>,
    ) -> Result<Json<ScratchpadWriteResponse>, String> {
        info!(old_name = "write_scratchpad", new_name = "scratchpad_write", "tool_alias");
        let mapped = mcp_aliases::map_write_scratchpad_params(req).map_err(|e| e.to_string())?;
        self.scratchpad_write(Parameters(ScratchpadWriteRequest {
            project: mapped.owner,
            topic: mapped.filename_stem,
            content: mapped.content,
        }))
        .await
    }

    #[tool(description = "List scratchpad files for a project.")]
    async fn scratchpad_list(
        &self,
        Parameters(req): Parameters<ScratchpadListRequest>,
    ) -> Result<Json<ScratchpadListResponse>, String> {
        knowledge::scratchpad_list(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Alias for scratchpad_list using legacy list_scratchpad schema.")]
    async fn list_scratchpad(
        &self,
        Parameters(req): Parameters<mcp_aliases::ListScratchpadParams>,
    ) -> Result<Json<ScratchpadListResponse>, String> {
        info!(old_name = "list_scratchpad", new_name = "scratchpad_list", "tool_alias");
        let mapped = mcp_aliases::map_list_scratchpad_params(req).map_err(|e| e.to_string())?;
        let project = mapped.cwd.unwrap_or_else(|| "inter-agent".to_string());
        self.scratchpad_list(Parameters(ScratchpadListRequest { project }))
            .await
    }

    #[tool(description = "Read recent outbox lifecycle events.")]
    async fn outbox_recent(
        &self,
        Parameters(req): Parameters<OutboxRecentRequest>,
    ) -> Result<Json<OutboxRecentResponse>, String> {
        knowledge::outbox_recent(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "List pending dead-drop fallback tickets.")]
    async fn fallback_list(
        &self,
        Parameters(req): Parameters<FallbackListRequest>,
    ) -> Result<Json<FallbackListResponse>, String> {
        knowledge::fallback_list(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Acknowledge a dead-drop fallback ticket by deleting it.")]
    async fn fallback_ack(&self, Parameters(req): Parameters<FallbackAckRequest>) -> Result<String, String> {
        knowledge::fallback_ack(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Garbage collect stale dead-drop fallback tickets.")]
    async fn fallback_gc(
        &self,
        Parameters(req): Parameters<FallbackGcRequest>,
    ) -> Result<Json<FallbackGcResponse>, String> {
        knowledge::fallback_gc(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Get Ledger health status for the current project.")]
    async fn ledger_health(&self) -> Result<Json<HealthStatus>, String> {
        knowledge::ledger_health().await
    }

    #[tool(description = "Search ledger summaries via FTS5.")]
    async fn ledger_query(
        &self,
        Parameters(req): Parameters<LedgerQueryRequest>,
    ) -> Result<Json<LedgerQueryResponse>, String> {
        knowledge::ledger_query(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Fetch full ledger session reconstruction for a session_id.")]
    async fn ledger_session(
        &self,
        Parameters(req): Parameters<LedgerSessionRequest>,
    ) -> Result<Json<SessionDetail>, String> {
        knowledge::ledger_session(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Insert a manual high-signal summary record into ledger.")]
    async fn ledger_record(&self, Parameters(req): Parameters<ManualRecord>) -> Result<String, String> {
        knowledge::ledger_record(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Run ledger garbage collection for stale events and dead-drop tickets.")]
    async fn ledger_gc(&self) -> Result<Json<GcResult>, String> {
        knowledge::ledger_gc(mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Add a reusable lesson to the ledger knowledge base.")]
    async fn lesson_add(&self, Parameters(req): Parameters<NewLesson>) -> Result<Json<LessonAddResponse>, String> {
        knowledge::lesson_add(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Query lessons using full-text search and confidence filtering.")]
    async fn lesson_query(
        &self,
        Parameters(req): Parameters<LessonQueryRequest>,
    ) -> Result<Json<LessonQueryResponse>, String> {
        knowledge::lesson_query(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Mark a lesson as validated and reset confidence decay anchor.")]
    async fn lesson_validate(
        &self,
        Parameters(req): Parameters<LessonValidateRequest>,
    ) -> Result<String, String> {
        knowledge::lesson_validate(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "List lessons with optional tag and staleness filters.")]
    async fn lesson_list(
        &self,
        Parameters(req): Parameters<LessonListRequest>,
    ) -> Result<Json<LessonListResponse>, String> {
        knowledge::lesson_list(req, mcp_daemon_proxy_enabled()).await
    }

    #[tool(description = "Dispatch a one-off Codex task without worktree isolation.")]
    async fn dispatch_codex(
        &self,
        Parameters(req): Parameters<DispatchCodexRequest>,
    ) -> Result<Json<DispatchCodexResponse>, String> {
        mcp_tools::abe::dispatch_codex(self.abe_tasks.clone(), req, self.abe_callbacks())
            .await
            .map(Json)
    }

    #[tool(description = "Dispatch a one-off Codex task in an isolated worktree with .triumvirate artifacts.")]
    async fn dispatch_codex_worktree(
        &self,
        Parameters(req): Parameters<DispatchCodexWorktreeRequest>,
    ) -> Result<Json<DispatchCodexWorktreeResponse>, String> {
        mcp_tools::abe::dispatch_codex_worktree(self.abe_tasks.clone(), req, self.abe_callbacks())
            .await
            .map(Json)
    }

    #[tool(description = "Query the Antigravity sibling synchronously and return response text.")]
    async fn query_antigravity(
        &self,
        Parameters(req): Parameters<QueryGeminiRequest>,
    ) -> Result<Json<QueryGeminiResponse>, String> {
        let response = mcp_gemini_query::query_gemini(req, |ask_req| async move {
            execute_ask_agent(&ask_req, None).await
        })
        .await?;
        Ok(Json(response))
    }

    #[tool(description = "Query the Antigravity sibling for code review verdicts on pass/failure contexts.")]
    async fn query_antigravity_review(
        &self,
        Parameters(req): Parameters<QueryGeminiReviewRequest>,
    ) -> Result<Json<QueryGeminiReviewResponse>, String> {
        let response = mcp_gemini_query::query_gemini_review(req, |ask_req| async move {
            execute_ask_agent(&ask_req, None).await
        })
        .await?;
        Ok(Json(response))
    }

    #[tool(description = "Deprecated alias of query_antigravity — kept for back-compat. Query the Antigravity sibling synchronously.")]
    async fn query_gemini(
        &self,
        Parameters(req): Parameters<QueryGeminiRequest>,
    ) -> Result<Json<QueryGeminiResponse>, String> {
        let response = mcp_gemini_query::query_gemini(req, |ask_req| async move {
            execute_ask_agent(&ask_req, None).await
        })
        .await?;
        Ok(Json(response))
    }

    #[tool(description = "Deprecated alias of query_antigravity_review — kept for back-compat.")]
    async fn query_gemini_review(
        &self,
        Parameters(req): Parameters<QueryGeminiReviewRequest>,
    ) -> Result<Json<QueryGeminiReviewResponse>, String> {
        let response = mcp_gemini_query::query_gemini_review(req, |ask_req| async move {
            execute_ask_agent(&ask_req, None).await
        })
        .await?;
        Ok(Json(response))
    }

    #[tool(description = "Get status for a dispatched ABE task.")]
    async fn get_task_status(
        &self,
        Parameters(req): Parameters<GetTaskStatusRequest>,
    ) -> Result<Json<GetTaskStatusResponse>, String> {
        mcp_tools::abe::get_task_status(self.abe_tasks.clone(), req)
            .await
            .map(Json)
    }

    #[tool(description = "Get output details for a completed ABE task.")]
    async fn get_task_output(
        &self,
        Parameters(req): Parameters<GetTaskOutputRequest>,
    ) -> Result<Json<GetTaskOutputResponse>, String> {
        mcp_tools::abe::get_task_output(self.abe_tasks.clone(), req)
            .await
            .map(Json)
    }

    #[tool(description = "Cancel a running ABE task.")]
    async fn cancel_task(
        &self,
        Parameters(req): Parameters<AbeCancelTaskRequest>,
    ) -> Result<Json<AbeCancelTaskResponse>, String> {
        mcp_tools::abe::cancel_task(self.abe_tasks.clone(), req)
            .await
            .map(Json)
    }

    #[tool(description = "Spawn a multi-agent fleet (dry_run defaults to true).")]
    async fn fleet_spawn(
        &self,
        Parameters(req): Parameters<FleetSpawnRequest>,
    ) -> Result<Json<FleetSpawnResponse>, String> {
        let response = mcp_fleet::fleet_spawn(
            &self.fleet_states,
            &self.metrics,
            Some(&self.ws_events),
            req,
            |project_root| {
                let git_ops = git_ops_impl::RealGitOps::new(project_root)
                    .map_err(|e| format!("fleet_spawn gitops init failed: {e}"))?;
                Ok(fleet::orchestrator::FleetOrchestrator::new(git_ops))
            },
        )
        .await?;
        Ok(Json(response))
    }

    #[tool(description = "Return fleet status by fleet_id.")]
    async fn fleet_status(
        &self,
        Parameters(req): Parameters<FleetStatusRequest>,
    ) -> Result<Json<FleetStatusResponse>, String> {
        Ok(Json(mcp_fleet::fleet_status(&self.fleet_states, req).await?))
    }

    #[tool(description = "List known fleet task IDs for a fleet.")]
    async fn fleet_task_list(
        &self,
        Parameters(req): Parameters<FleetTaskListRequest>,
    ) -> Result<Json<FleetTaskListResponse>, String> {
        Ok(Json(mcp_fleet::fleet_task_list(&self.fleet_states, req).await?))
    }

    #[tool(description = "Claim a fleet task in SQLite.")]
    async fn fleet_claim_task(
        &self,
        Parameters(req): Parameters<FleetClaimTaskRequest>,
    ) -> Result<Json<FleetClaimTaskResponse>, String> {
        Ok(Json(mcp_fleet::fleet_claim_task(req).await?))
    }

    #[tool(description = "Cancel a fleet by fleet_id.")]
    async fn fleet_cancel(
        &self,
        Parameters(req): Parameters<FleetCancelRequest>,
    ) -> Result<Json<FleetCancelResponse>, String> {
        Ok(Json(
            mcp_fleet::fleet_cancel(&self.fleet_states, &self.metrics, Some(&self.ws_events), req)
                .await?,
        ))
    }

    #[tool(description = "Request a peer review and receive assigned reviewer + review_id.")]
    async fn review_request(
        &self,
        Parameters(req): Parameters<ReviewRequestTool>,
    ) -> Result<Json<ReviewRequestResponse>, String> {
        let response = mcp_review::review_request(req)?;
        Ok(Json(response))
    }

    #[tool(description = "Submit peer review verdict and comments.")]
    async fn review_submit(
        &self,
        Parameters(req): Parameters<ReviewSubmitRequest>,
    ) -> Result<String, String> {
        mcp_review::review_submit(&self.metrics, req)
    }

    #[tool(description = "Get current peer review status by review_id.")]
    async fn review_status(
        &self,
        Parameters(req): Parameters<ReviewStatusRequest>,
    ) -> Result<Json<ReviewStatusResponse>, String> {
        let response = mcp_review::review_status(req)?;
        Ok(Json(response))
    }

    #[tool(description = "Alias for review_request using legacy code_review schema.")]
    async fn code_review(
        &self,
        Parameters(req): Parameters<mcp_aliases::CodeReviewParams>,
    ) -> Result<Json<ReviewRequestResponse>, String> {
        info!(old_name = "code_review", new_name = "review_request", "tool_alias");
        let mapped = mcp_aliases::map_code_review_params(req).map_err(|e| e.to_string())?;
        let artifact = build_code_review_artifact(&mapped)?;
        self.review_request(Parameters(ReviewRequestTool {
            project_root: mapped.cwd.clone(),
            fleet_id: None,
            author_agent: "codex".to_string(),
            artifact,
            review_type: "code".to_string(),
        }))
        .await
    }
}

async fn execute_ask_agent(
    req: &AskAgentRequest,
    progress: Option<ProgressEmitter>,
) -> Result<AskAgentResponse, String> {
    if let Err(err) = init_process_token_db() {
        warn!("failed to initialize token DB for ask_agent: {err}");
    }
    agent_exec::execute_ask_agent(req, progress).await
}

fn execute_ask_agent_boxed<'a>(
    req: &'a AskAgentRequest,
    progress: Option<ProgressEmitter>,
) -> Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>> {
    Box::pin(execute_ask_agent(req, progress))
}

fn build_code_review_artifact(req: &mcp_aliases::ReviewRequestLike) -> Result<String, String> {
    let cwd = req.cwd.clone().unwrap_or_else(|| ".".to_string());
    let mut command = Command::new("git");
    command.arg("-C").arg(&cwd).arg("--no-pager");
    if let Some(commit_sha) = req.commit_sha.as_deref() {
        command.args(["show", "--no-color", commit_sha]);
    } else if let Some(base_branch) = req.base_branch.as_deref() {
        command.args(["diff", "--no-color", &format!("{base_branch}...HEAD")]);
    } else if req.uncommitted.unwrap_or(false) {
        command.args(["diff", "--no-color", "HEAD"]);
    } else {
        command.args(["diff", "--no-color"]);
    }

    let output = command
        .output()
        .map_err(|e| format!("code_review failed to run git diff/show: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "code_review git command failed".to_string()
        } else {
            format!("code_review git command failed: {stderr}")
        });
    }

    let artifact = String::from_utf8_lossy(&output.stdout).to_string();
    if artifact.trim().is_empty() {
        Ok("No diff content found for the selected review scope.".to_string())
    } else {
        Ok(artifact)
    }
}

async fn prewarm_daemon_workers() {
    agent_exec::prewarm_daemon_workers().await;
}

/// Hand-rolled `ServerHandler` impl. Replaces `#[tool_handler]` so we can
/// intercept `call_tool` and extract Pantheon session lineage from the
/// JSON-RPC `_meta` field before delegating to the generated tool router.
///
/// FEAT-014 (REQ-010, REQ-033) T-004 — stdio transport half.
///
/// The HTTP transport extracts lineage from the `X-Pantheon-Session-Id` /
/// `X-Pantheon-Root-Session-Id` headers in `http_mcp::bearer_auth_middleware`
/// and scopes the downstream handler chain in `PANTHEON_SESSION.scope(...)`.
/// The stdio transport has no headers, so Pantheon's MCP proxy passes the
/// same identifiers through the MCP protocol's `_meta` object instead:
///
/// ```json
/// {
///   "method": "tools/call",
///   "params": {
///     "name": "dispatch_codex",
///     "_meta": {
///       "pantheon.session_id": "sess-xyz",
///       "pantheon.root_session_id": "sess-root"
///     },
///     "arguments": { ... }
///   }
/// }
/// ```
///
/// `call_tool` reads those fields, constructs a `PantheonSessionContext`,
/// and wraps the inner `tool_router.call(...)` in `PANTHEON_SESSION.scope`
/// so that `dispatch_codex`/`dispatch_codex_worktree` see the same task-local
/// state they would on the HTTP path. `list_tools` and `get_tool` are copied
/// verbatim from the `#[tool_handler]` expansion.
/// The MCP session id for this stdio process, minted once and stable for its lifetime.
///
/// PostHog's MCP Analytics is per-session; stdio has one client connection per process, so a
/// stable per-process `ses_<hex>` IS the session (the rotate-after-idle logic the SDK uses is
/// an HTTP/SSE concern that does not apply to a single stdio connection).
fn mcp_session_id() -> &'static str {
    static SID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SID.get_or_init(|| format!("ses_{}", Uuid::new_v4().simple()))
}

/// Client name/version from the MCP initialize handshake, for the `$mcp_client_*` properties.
/// `None` if the peer has not completed initialize (should not happen for a tool call, but the
/// telemetry must never assume).
fn mcp_client_identity(
    context: &RequestContext<RoleServer>,
) -> (Option<String>, Option<String>) {
    match context.peer.peer_info() {
        Some(info) => (
            // Cap both: a buggy client sending a timestamp/build-hash as its version would be an
            // unbounded property (Antigravity). Client name/version are short in practice.
            Some(info.client_info.name.chars().take(64).collect()),
            Some(info.client_info.version.chars().take(64).collect()),
        ),
        None => (None, None),
    }
}

/// Emit `$mcp_initialize` exactly once per process (stdio = one session per process), the first
/// time we see a tool call. rmcp handles the initialize handshake internally, so there is no
/// ServerHandler hook for it; this is the faithful stand-in.
fn emit_mcp_initialize_once(session_id: &str, client_name: Option<&str>, client_version: Option<&str>) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    mcp_bridge::posthog::record_mcp_initialize(
        session_id,
        client_name.unwrap_or("unknown"),
        client_version.unwrap_or("unknown"),
    );
}

/// Map a JSON-RPC error code to PostHog's `$mcp_error_type` enum. Triumvirate's tool failures
/// mostly surface as invalid-params (validation) or server errors (internal); anything
/// unrecognized is "internal" rather than an invented category.
fn classify_mcp_error(code: i32) -> &'static str {
    match code {
        -32602 => "validation",            // invalid params
        -32601 => "validation",            // method/tool not found
        -32700 | -32600 => "validation",   // parse / invalid request
        _ => "internal",
    }
}

/// Server-side fallback intent for a tool call when the agent did not author a `context` string.
/// Mirrors @posthog/mcp's `intentFallback`: a short, human-readable "why" derived from the tool and
/// the SHAPE of its arguments. Deliberately never echoes raw argument VALUES (a prompt/message):
/// those are the caller's content, already handled (scrubbed) by `$mcp_parameters`. Only structural
/// hints like the target agent name are used, so an inferred intent cannot leak a prompt.
fn infer_mcp_intent(tool_name: &str, params: Option<&serde_json::Value>) -> String {
    let arg = |k: &str| -> Option<String> {
        params
            .and_then(|p| p.get(k))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    match tool_name {
        "ask_agent" => match arg("agent") {
            Some(a) => format!("Consulting the {a} sibling for analysis or review via ask_agent"),
            None => "Consulting a sibling agent via ask_agent".to_string(),
        },
        "ask_daemon" | "ask_session" => {
            "Continuing a multi-turn exchange with a sibling agent".to_string()
        }
        "dispatch_codex" | "dispatch_codex_worktree" => {
            "Dispatching Codex to write code in a repo".to_string()
        }
        "query_gemini" | "query_antigravity" => {
            "Querying Antigravity for analysis".to_string()
        }
        "query_gemini_review" | "query_antigravity_review" | "code_review"
        | "review_request" | "review_submit" => {
            "Requesting or submitting a code review".to_string()
        }
        "spawn_daemon" | "spawn_session" => {
            "Spawning a persistent sibling worker for a longer exchange".to_string()
        }
        n if n.starts_with("fleet_") => "Coordinating the parallel worker fleet".to_string(),
        n if n.starts_with("ledger_") => "Recording or querying the build ledger".to_string(),
        n if n.starts_with("memory_") || n.starts_with("scratchpad_") => {
            "Reading or writing shared agent memory".to_string()
        }
        "ping" | "daemon_health" | "get_status" | "get_token_summary" => {
            "Health, status, or usage probe".to_string()
        }
        other => format!("Calling the {other} MCP tool"),
    }
}

/// Inject the optional `context` argument into a tool's advertised input schema so an agent can
/// author an intent string (captured as `$mcp_intent`, source `context_parameter`). Added to
/// `properties` ONLY, never to `required`, and `call_tool` STRIPS it from the arguments before
/// dispatch, so a handler that never declared it is unaffected. Mirrors @posthog/mcp's default
/// `context: true` behavior. A malformed (non-object) schema is left untouched rather than risking
/// a broken advertisement.
fn inject_intent_arg(mut tool: Tool) -> Tool {
    let mut schema = tool.input_schema.as_ref().clone();
    let props = schema
        .entry("properties")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let Some(props) = props.as_object_mut() {
        props.insert(
            "context".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional. In one sentence, why you are calling this tool right now \
                    (the task or goal it serves). Recorded for observability; it does not change \
                    the tool's behavior."
            }),
        );
        tool.input_schema = std::sync::Arc::new(schema);
    }
    tool
}

impl ServerHandler for McpBridge {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(format!(
                "Triumvirate MCP bridge v{}. Use `ping` to verify connectivity.",
                daemon_core::VERSION
            ))
    }

    async fn call_tool(
        &self,
        mut request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // FEAT-014 (REQ-010, REQ-033) T-004 stdio half: pull Pantheon
        // lineage out of the _meta envelope. Missing/blank values collapse
        // to None, which means "not a Pantheon caller" and propagates
        // through as absent lineage on the emitted WorkerLifecycle events.
        let scope_value = extract_pantheon_scope_from_meta(&request);

        // MCP Analytics: capture identity BEFORE `request`/`context` are moved into the tool
        // context. This one choke point covers all 54 tools. Params are Rung-1 sanitized inside
        // posthog.rs (truncated preview, key-name + token-prefix redaction, paths basenamed).
        let sid = mcp_session_id();
        let tool_name = request.name.to_string();
        // MCP Analytics agent intent. The agent may author a `context` string (list_tools injects
        // that arg into every tool schema). Pull it out and STRIP it from the arguments before
        // dispatch, so the tool handler never receives an arg it did not declare. If the agent did
        // not supply one, fall back to an inferred intent derived from the tool + its args.
        let client_intent = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("context"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty());
        if let Some(args) = request.arguments.as_mut() {
            args.remove("context");
        }
        let params = request
            .arguments
            .as_ref()
            .map(|a| serde_json::Value::Object(a.clone()));
        let (intent, intent_source): (String, &'static str) = match &client_intent {
            Some(c) => (c.clone(), "context_parameter"),
            None => (infer_mcp_intent(&tool_name, params.as_ref()), "inferred"),
        };
        let (client_name, client_version) = mcp_client_identity(&context);
        emit_mcp_initialize_once(sid, client_name.as_deref(), client_version.as_deref());
        let started = std::time::Instant::now();

        let tcc = ToolCallContext::new(self, request, context);
        let fut = self.tool_router.call(tcc);
        let result = daemon_core::PANTHEON_SESSION.scope(scope_value, fut).await;

        let duration_ms = started.elapsed().as_millis() as u64;
        // A tool "error" is EITHER a transport-level Err OR an Ok result flagged is_error (MCP
        // lets a tool return a successful envelope that marks itself failed). No $mcp_error_status:
        // JSON-RPC codes are not HTTP statuses (Codex); $mcp_error_type carries the class.
        let (is_error, error_type, response_json) = match &result {
            Ok(r) => {
                let is_err = r.is_error.unwrap_or(false);
                let et = if is_err { Some("internal") } else { None };
                (is_err, et, serde_json::to_value(r).ok())
            }
            Err(e) => (true, Some(classify_mcp_error(e.code.0)), None),
        };
        // Bound $mcp_tool_name: a hallucinated tool name fails with method-not-found (-32601),
        // and emitting the raw hallucinated string would let a misbehaving agent mint unbounded
        // property values and pollute PostHog's dictionary (Antigravity). A real tool is always
        // one of the registered set; collapse the unknown case to a single label.
        let emitted_tool = match &result {
            Err(e) if e.code.0 == -32601 => "unknown",
            _ => tool_name.as_str(),
        };
        mcp_bridge::posthog::record_mcp_tool_call(
            sid,
            emitted_tool,
            duration_ms,
            is_error,
            error_type,
            client_name.as_deref(),
            client_version.as_deref(),
            params.as_ref(),
            response_json.as_ref(),
            Some(intent.as_str()),
            Some(intent_source),
        );
        result
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        // Advertise every tool with the injected optional `context` arg (agent-intent capture),
        // then emit the inventory. The injection only touches the advertised schema; dispatch is
        // unchanged because call_tool strips `context` back out before running the handler.
        let tools: Vec<Tool> = self
            .tool_router
            .list_all()
            .into_iter()
            .map(inject_intent_arg)
            .collect();
        // The advertised-vs-called signal: emit the full tool inventory so PostHog can show
        // which of the 54 tools agents actually use.
        let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        let (client_name, client_version) = mcp_client_identity(&_context);
        mcp_bridge::posthog::record_mcp_tools_list(
            mcp_session_id(),
            &names,
            client_name.as_deref(),
            client_version.as_deref(),
        );
        Ok(ListToolsResult {
            tools,
            meta: None,
            next_cursor: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

/// FEAT-014 (REQ-010, REQ-033) T-004 stdio half.
///
/// Pull `_meta.pantheon.session_id` and optional `_meta.pantheon.root_session_id`
/// out of a tool-call request and produce the `PANTHEON_SESSION` scope value.
///
/// Returns:
/// - `Some(Arc<ctx>)` if a non-empty session ID is present.
/// - `None` if `_meta` is absent, the fields are missing, non-string, or blank.
///
/// Broken out so it can be unit-tested without standing up a full MCP server.
fn extract_pantheon_scope_from_meta(
    request: &CallToolRequestParams,
) -> Option<Arc<daemon_core::PantheonSessionContext>> {
    // Canonical keys, namespaced per MCP `_meta` convention.
    const PARENT_KEY: &str = "pantheon.session_id";
    const ROOT_KEY: &str = "pantheon.root_session_id";

    let meta = request.meta.as_ref()?;
    let obj = &meta.0;

    let parent = obj
        .get(PARENT_KEY)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    let root = obj
        .get(ROOT_KEY)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let ctx = match root {
        Some(r) => daemon_core::PantheonSessionContext::with_root(parent, r),
        None => daemon_core::PantheonSessionContext::new(parent),
    };
    Some(Arc::new(ctx))
}

fn init_tracing() -> anyhow::Result<()> {
    tracing_setup::init_tracing()
}

#[cfg(unix)]
fn maybe_install_mcp_stdout_tap() -> anyhow::Result<()> {
    let Some(log_path) = std::env::var("TRIUMVIRATE_MCP_STDOUT_TAP").ok().filter(|v| !v.trim().is_empty()) else {
        return Ok(());
    };

    let mut fds = [0i32; 2];
    // SAFETY: valid pointer to two-int array.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(anyhow::anyhow!("pipe() failed: {}", std::io::Error::last_os_error()));
    }
    let read_fd = fds[0];
    let write_fd = fds[1];

    // SAFETY: dup stdout fd; returns new fd or -1.
    let saved_stdout = unsafe { libc::dup(libc::STDOUT_FILENO) };
    if saved_stdout < 0 {
        // SAFETY: close fds opened above.
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(anyhow::anyhow!("dup(stdout) failed: {}", std::io::Error::last_os_error()));
    }

    // SAFETY: redirect fd1 to pipe write-end.
    if unsafe { libc::dup2(write_fd, libc::STDOUT_FILENO) } < 0 {
        // SAFETY: close fds opened above.
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
            libc::close(saved_stdout);
        }
        return Err(anyhow::anyhow!("dup2(pipe->stdout) failed: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: write end now duplicated onto stdout.
    unsafe {
        libc::close(write_fd);
    }

    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| anyhow::anyhow!("failed to open stdout tap log {}: {e}", log_path))?;
    writeln!(
        log,
        "=== triumvirate mcp stdout tap start pid={} ts={} ===",
        std::process::id(),
        chrono::Utc::now().to_rfc3339()
    )?;
    let _ = log.flush();

    std::thread::spawn(move || {
        let mut log = match std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let mut buf = vec![0u8; 16384];
        let mut seq: u64 = 0;
        loop {
            // SAFETY: read into valid mutable buffer.
            let n = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            let n = n as usize;
            let chunk = &buf[..n];

            // Forward exact bytes back to original stdout fd.
            let mut off = 0usize;
            while off < n {
                // SAFETY: write from valid slice pointer/len.
                let wrote = unsafe {
                    libc::write(
                        saved_stdout,
                        chunk[off..].as_ptr() as *const libc::c_void,
                        n - off,
                    )
                };
                if wrote <= 0 {
                    break;
                }
                off += wrote as usize;
            }

            let preview_len = chunk.len().min(240);
            let preview = String::from_utf8_lossy(&chunk[..preview_len]).replace('\n', "\\n");
            let _ = writeln!(
                log,
                "STDOUT_CHUNK #{seq} bytes={} preview=\"{}\"",
                chunk.len(),
                preview
            );
            let _ = log.flush();
            seq = seq.saturating_add(1);
        }

        let _ = writeln!(
            log,
            "=== triumvirate mcp stdout tap end ts={} ===",
            chrono::Utc::now().to_rfc3339()
        );
        let _ = log.flush();
        // SAFETY: cleanup duplicated fds owned by this thread.
        unsafe {
            libc::close(read_fd);
            libc::close(saved_stdout);
        }
    });

    Ok(())
}

#[cfg(not(unix))]
fn maybe_install_mcp_stdout_tap() -> anyhow::Result<()> {
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing()?;

    match Cli::parse().command {
        CliCommand::Mcp => {
            maybe_install_mcp_stdout_tap()?;
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
        CliCommand::Proxy => {
            proxy::run_proxy().await?;
        }
        CliCommand::Watch(args) => {
            watch::run_watch(args).await?;
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

/// T-004 stdio-transport reality tests for `extract_pantheon_scope_from_meta`
/// and the composed `call_tool` scope wrap.
///
/// FEAT-014 (REQ-010, REQ-033).
///
/// These tests live in a dedicated module so they run cleanly regardless of
/// the state of the larger legacy `tests` module (which has several stale
/// tests disabled via `#[cfg(any())]` pending issue #24).
#[cfg(test)]
mod mcp_intent_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inferred_intent_is_tool_specific_and_never_echoes_values() {
        // ask_agent uses the agent NAME (a structural hint), never the message VALUE.
        let p = json!({ "agent": "codex", "message": "SECRET PROMPT do not leak" });
        let intent = infer_mcp_intent("ask_agent", Some(&p));
        assert!(intent.contains("codex"), "names the target agent: {intent}");
        assert!(!intent.contains("SECRET PROMPT"), "must not echo the prompt: {intent}");

        // Family prefixes collapse to one readable label.
        assert!(infer_mcp_intent("fleet_spawn", None).contains("fleet"));
        assert!(infer_mcp_intent("memory_write", None).contains("memory"));
        // Unknown tool still yields a bounded, readable fallback.
        assert_eq!(infer_mcp_intent("some_new_tool", None), "Calling the some_new_tool MCP tool");
    }

    #[test]
    fn inject_intent_arg_adds_optional_context_without_requiring_it() {
        let schema = serde_json::Map::from_iter([
            ("type".to_string(), json!("object")),
            ("properties".to_string(), json!({ "message": { "type": "string" } })),
            ("required".to_string(), json!(["message"])),
        ]);
        let tool = Tool::new(
            std::borrow::Cow::Borrowed("ask_agent"),
            std::borrow::Cow::Borrowed("desc"),
            std::sync::Arc::new(schema),
        );
        let injected = inject_intent_arg(tool);
        let s = injected.input_schema.as_ref();
        // context is present in properties...
        assert!(
            s["properties"].get("context").is_some(),
            "context injected into properties"
        );
        assert_eq!(s["properties"]["context"]["type"], json!("string"));
        // ...but NOT added to required (agents may omit it).
        assert_eq!(s["required"], json!(["message"]), "context stays optional");
        // The tool's own arg is untouched.
        assert!(s["properties"].get("message").is_some(), "original arg preserved");
    }
}

#[cfg(test)]
mod pantheon_stdio_meta_tests {
    use super::*;
    use rmcp::model::{CallToolRequestParams, Meta};
    use serde_json::{Value, json};
    use std::borrow::Cow;

    fn req_with_meta(meta_obj: serde_json::Map<String, Value>) -> CallToolRequestParams {
        // CallToolRequestParams is #[non_exhaustive]; use the builder path.
        let mut p = CallToolRequestParams::new(Cow::Borrowed("ping"));
        p.meta = Some(Meta(meta_obj));
        p
    }

    fn req_no_meta() -> CallToolRequestParams {
        CallToolRequestParams::new(Cow::Borrowed("ping"))
    }

    #[test]
    fn extract_returns_none_when_meta_absent() {
        assert!(extract_pantheon_scope_from_meta(&req_no_meta()).is_none());
    }

    #[test]
    fn extract_returns_none_when_pantheon_session_id_absent() {
        let mut m = serde_json::Map::new();
        m.insert("some.other.key".into(), json!("foo"));
        assert!(extract_pantheon_scope_from_meta(&req_with_meta(m)).is_none());
    }

    #[test]
    fn extract_returns_none_when_session_id_empty_string() {
        let mut m = serde_json::Map::new();
        m.insert("pantheon.session_id".into(), json!(""));
        assert!(extract_pantheon_scope_from_meta(&req_with_meta(m)).is_none());
    }

    #[test]
    fn extract_returns_none_when_session_id_whitespace_only() {
        let mut m = serde_json::Map::new();
        m.insert("pantheon.session_id".into(), json!("   "));
        assert!(extract_pantheon_scope_from_meta(&req_with_meta(m)).is_none());
    }

    #[test]
    fn extract_returns_none_when_session_id_not_a_string() {
        let mut m = serde_json::Map::new();
        m.insert("pantheon.session_id".into(), json!(42));
        assert!(extract_pantheon_scope_from_meta(&req_with_meta(m)).is_none());
    }

    #[test]
    fn extract_populates_parent_and_defaults_root_to_parent() {
        let mut m = serde_json::Map::new();
        m.insert("pantheon.session_id".into(), json!("stdio-sess-abc"));
        let ctx = extract_pantheon_scope_from_meta(&req_with_meta(m))
            .expect("scope should be populated");
        assert_eq!(ctx.parent_session_id, "stdio-sess-abc");
        assert_eq!(ctx.root_session_id, "stdio-sess-abc");
    }

    #[test]
    fn extract_honors_explicit_root_session_id() {
        let mut m = serde_json::Map::new();
        m.insert("pantheon.session_id".into(), json!("intermediate-worker"));
        m.insert("pantheon.root_session_id".into(), json!("pantheon-root"));
        let ctx = extract_pantheon_scope_from_meta(&req_with_meta(m))
            .expect("scope should be populated");
        assert_eq!(ctx.parent_session_id, "intermediate-worker");
        assert_eq!(ctx.root_session_id, "pantheon-root");
    }

    #[test]
    fn extract_blank_root_falls_back_to_parent() {
        let mut m = serde_json::Map::new();
        m.insert("pantheon.session_id".into(), json!("stdio-sess-xyz"));
        m.insert("pantheon.root_session_id".into(), json!("   "));
        let ctx = extract_pantheon_scope_from_meta(&req_with_meta(m))
            .expect("scope should be populated");
        assert_eq!(ctx.parent_session_id, "stdio-sess-xyz");
        // Blank root collapses to None, which defaults to parent.
        assert_eq!(ctx.root_session_id, "stdio-sess-xyz");
    }

    /// T-004 stdio half — end-to-end composition reality test.
    ///
    /// Exercises the SAME two-step pipeline `McpBridge::call_tool` uses:
    ///   (1) `extract_pantheon_scope_from_meta(&request)`
    ///   (2) `PANTHEON_SESSION.scope(scope_value, inner_future).await`
    ///
    /// The inner future reads `current_pantheon_session()` — the exact API
    /// that ABE dispatch code uses to pick up the lineage. If either step
    /// regresses (the extractor returns None, or the scope wrap is removed
    /// from `call_tool`), this test fails.
    #[tokio::test]
    async fn call_tool_pipeline_propagates_meta_to_task_local() {
        let mut m = serde_json::Map::new();
        m.insert("pantheon.session_id".into(), json!("stdio-e2e-parent"));
        m.insert("pantheon.root_session_id".into(), json!("stdio-e2e-root"));
        let req = req_with_meta(m);

        // Mirror McpBridge::call_tool's lineage pipeline exactly.
        let scope_value = extract_pantheon_scope_from_meta(&req);
        assert!(
            scope_value.is_some(),
            "extractor must produce Some for a valid _meta"
        );

        let captured = daemon_core::PANTHEON_SESSION
            .scope(scope_value, async {
                daemon_core::current_pantheon_session()
            })
            .await;

        let ctx = captured.expect("task-local must be visible inside scope");
        assert_eq!(ctx.parent_session_id, "stdio-e2e-parent");
        assert_eq!(ctx.root_session_id, "stdio-e2e-root");
    }

    /// Reverse assertion: a request without _meta produces a None scope,
    /// and `current_pantheon_session()` returns None even inside the
    /// scope wrap. This is the legacy (non-Pantheon) stdio caller path.
    #[tokio::test]
    async fn call_tool_pipeline_leaves_task_local_none_for_legacy_callers() {
        let req = req_no_meta();
        let scope_value = extract_pantheon_scope_from_meta(&req);
        assert!(scope_value.is_none());

        let captured = daemon_core::PANTHEON_SESSION
            .scope(scope_value, async {
                daemon_core::current_pantheon_session()
            })
            .await;
        assert!(captured.is_none());
    }
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

/// FEAT-012 (REQ-017) T-008: Runtime state alias used by the Pantheon v3.9.0
/// REST surface. Kept at module scope so both `run_daemon` and the
/// `pantheon_rest_tests` module can type the handlers identically.
type DaemonRuntimeState = DaemonState<abe::task_tracker::TaskTracker>;

/// FEAT-012 (REQ-017) T-008: GET /api/workers — return every ABE worker
/// currently tracked by `state.abe_tasks.snapshot_workers()`. SessionState
/// entries live on the existing `/session/list` route; they are NOT
/// aggregated here (they carry no `started_at`/`elapsed_ms`, so mixing them
/// in would require fabricating values — flagged and rejected in Phase 5.3
/// round 1 audit).
async fn api_workers(
    State(state): State<DaemonRuntimeState>,
    headers: HeaderMap,
) -> Result<AxumJson<shared_types::WorkersResponse>, StatusCode> {
    if !is_bearer_authorized(
        headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
        &state.token,
    ) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let workers = state.abe_tasks.snapshot_workers().await;
    Ok(AxumJson(shared_types::WorkersResponse { workers }))
}

/// FEAT-012 (REQ-017) T-008: GET /api/fleet — return every FleetBuild
/// currently registered in `state.fleet_v2_states`. Returns an empty
/// `{"builds":[]}` array (never null) when no builds are active.
async fn api_fleet(
    State(state): State<DaemonRuntimeState>,
    headers: HeaderMap,
) -> Result<AxumJson<shared_types::FleetResponse>, StatusCode> {
    if !is_bearer_authorized(
        headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
        &state.token,
    ) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let builds: Vec<shared_types::FleetBuild> = {
        let guard = state.fleet_v2_states.lock().await;
        guard.values().cloned().collect()
    };
    Ok(AxumJson(shared_types::FleetResponse { builds }))
}

/// FEAT-012 (REQ-017) T-008: GET /api/fleet/{build_id} — return a single
/// FleetBuild by id, or 404 if the build does not exist. Uses axum 0.8
/// path-segment syntax `{build_id}` (NOT `:build_id`, which panics at
/// router construction time in axum 0.8).
async fn api_fleet_by_id(
    State(state): State<DaemonRuntimeState>,
    axum::extract::Path(build_id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Result<AxumJson<shared_types::FleetBuild>, StatusCode> {
    if !is_bearer_authorized(
        headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
        &state.token,
    ) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let guard = state.fleet_v2_states.lock().await;
    match guard.get(&build_id).cloned() {
        Some(build) => Ok(AxumJson(build)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// FEAT-013 (REQ-020) T-009: GET /api/state — full daemon state snapshot.
/// Bearer-auth-gated per-handler, same pattern as health/status and the
/// T-008 REST routes. Returns the frozen T-002 StateResponse shape, which
/// has NO `sessions` field — named MCP sessions stay on the existing
/// `/session/list` route. Sources:
///   - version: daemon_core::VERSION.to_string() (pinned against the
///     compile-time constant, never a hardcoded literal)
///   - uptime_ms: state.started_at.elapsed() saturated into u64
///   - workers: state.abe_tasks.snapshot_workers() (ABE workers only,
///     same source /api/workers uses)
///   - fleet: state.fleet_v2_states.lock().await.values().cloned()
///   - last_event_seq: state.last_event_seq atomic load
async fn api_state(
    State(state): State<DaemonRuntimeState>,
    headers: HeaderMap,
) -> Result<AxumJson<shared_types::StateResponse>, StatusCode> {
    if !is_bearer_authorized(
        headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
        &state.token,
    ) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let version = daemon_core::VERSION.to_string();
    let uptime_ms = state
        .started_at
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let workers = state.abe_tasks.snapshot_workers().await;
    let fleet = {
        let guard = state.fleet_v2_states.lock().await;
        guard
            .values()
            .cloned()
            .collect::<Vec<shared_types::FleetBuild>>()
    };
    let last_event_seq = state
        .last_event_seq
        .load(std::sync::atomic::Ordering::Relaxed);

    Ok(AxumJson(shared_types::StateResponse {
        version,
        uptime_ms,
        workers,
        fleet,
        last_event_seq,
    }))
}

/// FEAT-013 (REQ-020) T-009: GET /ws/v2 — replay-aware WebSocket upgrade.
/// Auth is checked on the upgrade request BEFORE `ws.on_upgrade(...)` —
/// closing the socket inside the upgraded closure is too late, because the
/// 101 Switching Protocols response has already been sent by then. The
/// reality test #8 explicitly asserts a 401 response (not a connected-then-
/// closed socket).
///
/// Inside the upgraded socket we follow the canonical subscribe-before-read
/// pattern. See `ws_v2_handshake` for the step-by-step state machine.
async fn ws_v2(
    State(state): State<DaemonRuntimeState>,
    headers: HeaderMap,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    if !is_bearer_authorized(
        headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
        &state.token,
    ) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let replay_buffer = state.replay_buffer.clone();
    let ws_events = state.ws_events.clone();

    ws.on_upgrade(move |socket| async move {
        ws_v2_handshake(socket, replay_buffer, ws_events).await;
    })
}

/// Body of the /ws/v2 upgraded-socket handler, extracted so the handshake
/// logic can be reasoned about independently of the axum extractor wrapping.
///
/// Wire-format rule (Phase 5.3 R1 audit finding — non-negotiable):
/// historical replay frames AND live tail frames use the SAME
/// `daemon_core::encode_ws_event("agent_stream", payload)` envelope. Only
/// the two ReplayResponse handshake frames (ack and out_of_range) are bare
/// JSON, and clients distinguish them by the top-level "replay" field.
async fn ws_v2_handshake(
    mut socket: axum::extract::ws::WebSocket,
    replay_buffer: std::sync::Arc<daemon_core::replay::EventReplayBuffer>,
    ws_events: tokio::sync::broadcast::Sender<String>,
) {
    use axum::extract::ws::Message;
    use daemon_core::replay::ReplayResult;
    // SinkExt brings `close` into scope on `WebSocket` — axum's WebSocket
    // is a futures Sink and the close method lives on that trait.
    use futures::SinkExt;
    use tokio::sync::broadcast::error::RecvError;

    // STEP 1: subscribe to the broadcast channel FIRST, before we read the
    // replay buffer. This is the race-condition fix. Any event that lands
    // on ws_events after this subscribe() but before we drain the
    // historical buffer will be captured by `live_rx`; the `max_sent`
    // dedup below catches the overlap.
    let mut live_rx = ws_events.subscribe();

    // STEP 2: read the client's first message. Anything other than a
    // well-formed ReplayRequest Text frame closes the socket.
    let first = match socket.recv().await {
        Some(Ok(Message::Text(text))) => text,
        _ => {
            let _ = socket.close().await;
            return;
        }
    };
    let req: shared_types::ReplayRequest = match serde_json::from_str(first.as_str()) {
        Ok(r) => r,
        Err(_) => {
            let _ = socket.close().await;
            return;
        }
    };
    if req.action != "subscribe" {
        let _ = socket.close().await;
        return;
    }

    // STEP 3: snapshot the replay buffer.
    let replay = replay_buffer.replay_since(req.last_seq);

    // STEP 4: branch on ReplayResult.
    match replay {
        ReplayResult::OutOfRange { oldest_seq } => {
            // Send a BARE ReplayResponse (no envelope). Clients distinguish
            // handshake frames from event frames by the presence of the
            // top-level "replay" field. After sending, close the socket —
            // the client will fetch /api/state and reconnect with a
            // fresher last_seq.
            let resp = shared_types::ReplayResponse {
                replay: "out_of_range".to_string(),
                oldest_seq: Some(oldest_seq),
            };
            if let Ok(json) = serde_json::to_string(&resp) {
                let _ = socket.send(Message::Text(json.into())).await;
            }
            let _ = socket.close().await;
        }
        ReplayResult::Events(events) => {
            // Send the "ok" handshake ack (bare JSON, no envelope).
            let ack = shared_types::ReplayResponse {
                replay: "ok".to_string(),
                oldest_seq: None,
            };
            match serde_json::to_string(&ack) {
                Ok(json) => {
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = socket.close().await;
                    return;
                }
            }

            // Track the max seq we've sent so we can dedup overlap between
            // the historical replay and the live tail. A live event whose
            // seq is <= max_sent is one the client already has and must
            // not be re-emitted.
            let mut max_sent = req.last_seq;

            // Wrap every historical event in the SAME envelope the live
            // tail uses. Sending bare AgentStreamEvent JSON here was the
            // Round 1 audit's critical finding — it would force clients to
            // switch parsers at the replay→live boundary. One wire format.
            for event in &events {
                let payload = match serde_json::to_value(event) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let envelope = daemon_core::encode_ws_event("agent_stream", payload);
                if socket.send(Message::Text(envelope.into())).await.is_err() {
                    return;
                }
                let seq = event.seq();
                if seq > max_sent {
                    max_sent = seq;
                }
            }

            // STEP 5: live tail. Envelopes are already encoded by the
            // publisher (TaskTracker etc.), so we forward the raw string
            // unchanged after the dedup check. We parse a serde_json::Value
            // out of each envelope ONLY to extract the seq — we do NOT
            // re-serialize, which would risk drifting the ts_ms field.
            loop {
                match live_rx.recv().await {
                    Ok(envelope) => {
                        let mut skip = false;
                        if let Ok(value) =
                            serde_json::from_str::<serde_json::Value>(envelope.as_str())
                        {
                            if value.get("type").and_then(|v| v.as_str())
                                == Some("agent_stream")
                            {
                                if let Some(payload) = value.get("payload") {
                                    if let Ok(event) = serde_json::from_value::<
                                        shared_types::AgentStreamEvent,
                                    >(
                                        payload.clone()
                                    ) {
                                        let seq = event.seq();
                                        if seq <= max_sent {
                                            skip = true;
                                        } else {
                                            max_sent = seq;
                                        }
                                    }
                                }
                            }
                        }
                        if skip {
                            continue;
                        }
                        if socket.send(Message::Text(envelope.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {
                        // Canonical close-on-lag. The client reconnects
                        // with its current last_seq and the handshake
                        // starts over. Do NOT try to recover in place.
                        let _ = socket.close().await;
                        break;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn run_daemon() -> anyhow::Result<()> {

    async fn metrics_middleware(
        State(state): State<DaemonRuntimeState>,
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
        State(state): State<DaemonRuntimeState>,
        headers: HeaderMap,
    ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let mut body = serde_json::json!({
            "status": "ok",
            "service": "triumvirate-daemon-v2",
            "mode": "incremental-dev",
            "daemon_bind_addr": state.bind_addr,
            "version": daemon_core::VERSION
        });
        // REQ-056: surface agy health (only when the agy backend is selected).
        if matches!(mcp_bridge::gemini_backend(), mcp_bridge::GeminiBackend::Agy) {
            let h = mcp_bridge::agy_resilience::agy_health_snapshot();
            body["agy_capture_health"] = serde_json::json!(h.capture_health);
            body["agy_backend_health"] = serde_json::json!(h.backend_health);
            body["agy_health_detail"] = serde_json::json!(h.detail);
            body["agy_health_last_probe_unix_ms"] = serde_json::json!(h.last_probe_unix_ms);
        }
        Ok(AxumJson(body))
    }

    async fn status(
        State(state): State<DaemonRuntimeState>,
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
            "supported_agents": mcp_bridge::supported_agent_names(),
            "pending_fallbacks": pending,
            "fallback_tickets": tickets,
            "daemon_bind_addr": state.bind_addr
        })))
    }

    fn ledger_sweep_interval() -> Duration {
        std::env::var("TRIUMVIRATE_LEDGER_SWEEP_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(60))
    }

    fn spool_dir_size_bytes(path: &Path) -> i64 {
        if !path.exists() {
            return 0;
        }
        let mut total: u128 = 0;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.filter_map(Result::ok) {
                let file_type = match entry.file_type() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if file_type.is_file() {
                    if let Ok(meta) = entry.metadata() {
                        total = total.saturating_add(meta.len() as u128);
                    }
                } else if file_type.is_dir() {
                    total = total.saturating_add(spool_dir_size_bytes(&entry.path()) as u128);
                }
            }
        }
        total.min(i64::MAX as u128) as i64
    }

    async fn run_ledger_sweep_once(state: &DaemonRuntimeState) {
        let project_roots = {
            let lru = state.ledger_project_lru.lock().await;
            lru.iter().cloned().collect::<Vec<_>>()
        };
        for project_root in project_roots {
            let result = tokio::task::spawn_blocking({
                let project_root = project_root.clone();
                move || -> anyhow::Result<(shared_types::DrainResult, f64, i64)> {
                    let store = LedgerStore::open(project_root.clone())?;
                    let spool_dir = project_root.join(".triumvirate").join("spool");
                    let drained = store.drain_spool(&spool_dir)?;
                    let lag = store.queue_lag_seconds()?;
                    let spool_size_bytes = spool_dir_size_bytes(&spool_dir);
                    Ok((drained, lag, spool_size_bytes))
                }
            })
            .await;

            match result {
                Ok(Ok((drain_result, lag, spool_size_bytes))) => {
                    state
                        .metrics
                        .ledger_events_ingested_total
                        .inc_by(drain_result.ingested_count as u64);
                    state.metrics.ledger_queue_lag_seconds.set(lag);
                    state.metrics.ledger_spool_size_bytes.set(spool_size_bytes);
                }
                Ok(Err(err)) => {
                    tracing::warn!(
                        "ledger background sweep failed for {}: {err}",
                        project_root.display()
                    );
                    // Silent until now (warn-only). A ledger that stops draining is a data-
                    // integrity problem that hides behind a green dashboard.
                    mcp_bridge::posthog::record_maintenance("ledger_sweep", "failed", 0);
                }
                Err(err) => {
                    tracing::warn!(
                        "ledger background sweep join failure for {}: {err}",
                        project_root.display()
                    );
                    mcp_bridge::posthog::record_maintenance("ledger_sweep", "failed", 0);
                }
            }
        }
    }

    async fn run_startup_gc_if_needed(state: &DaemonRuntimeState) {
        let Some(project_root) = std::env::current_dir().ok() else {
            tracing::warn!("startup GC skipped: unable to resolve current_dir");
            return;
        };

        let result = tokio::task::spawn_blocking({
            let project_root = project_root.clone();
            move || -> anyhow::Result<Option<GcResult>> {
                let store = LedgerStore::open(project_root)?;
                if !store.should_run_startup_gc()? {
                    return Ok(None);
                }
                Ok(Some(store.gc()?))
            }
        })
        .await;

        match result {
            Ok(Ok(Some(gc_result))) => {
                tracing::info!(
                    events_scanned = gc_result.events_scanned,
                    events_deleted = gc_result.events_deleted,
                    space_reclaimed_bytes = gc_result.space_reclaimed_bytes,
                    dead_drop_deleted = gc_result.dead_drop_deleted,
                    "startup ledger GC completed"
                );
                if let Ok(payload) = serde_json::to_value(&gc_result) {
                    publish_ws_event(state, "ledger_gc", payload);
                }
            }
            Ok(Ok(None)) => {
                tracing::debug!("startup ledger GC skipped");
            }
            Ok(Err(err)) => {
                tracing::warn!("startup ledger GC failed: {err}");
            }
            Err(err) => {
                tracing::warn!("startup ledger GC join failure: {err}");
            }
        }
    }

    async fn session_spawn_route(
        State(state): State<DaemonRuntimeState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<SpawnSessionRequest>,
    ) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((StatusCode::UNAUTHORIZED, AxumJson(serde_json::json!({ "error": "unauthorized" }))));
        }
        let agent = normalize_agent_name(&req.agent);
        if !is_supported_agent_name(&agent) {
            return Err((StatusCode::BAD_REQUEST, AxumJson(serde_json::json!({ "error": "spawn_session supports only 'antigravity' (aliases: agy, gemini), 'codex', or 'deepseek'" }))));
        }
        let cwd = req.cwd.clone().unwrap_or_else(|| ".".to_string());
        // Respawn starts a NEW conversation: clear any prior CLI session for this name first.
        agent_worker::reset_worker_session(&agent, &cwd, Some(req.name.as_str())).await;
        let worker = acquire_worker(&agent, &cwd, Some(req.name.as_str())).await;
        let mut sessions = state.sessions.lock().await;
        sessions.insert(
            req.name.clone(),
            SessionState {
                agent: agent.clone(),
                cwd: Some(cwd),
                history: Vec::new(),
                // FEAT-011 (REQ-010, REQ-033): Pantheon lineage fields,
                // populated later during MCP dispatch in T-004.
                parent_session_id: None,
                root_session_id: None,
                pantheon_session_id: None,
            cli_session_id: None,
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
        State(state): State<DaemonRuntimeState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<AskSessionRequest>,
    ) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((StatusCode::UNAUTHORIZED, AxumJson(serde_json::json!({ "error": "unauthorized" }))));
        }
        let (agent, cwd, prior_cli_session) = {
            let sessions = state.sessions.lock().await;
            let session = sessions.get(&req.name).ok_or_else(|| {
                (StatusCode::NOT_FOUND, AxumJson(serde_json::json!({ "error": format!("session '{}' not found", req.name) })))
            })?;
            (session.agent.clone(), session.cwd.clone(), session.cli_session_id.clone())
        };
        let response = execute_ask_agent(
            &AskAgentRequest {
                agent,
                message: req.message.clone(),
                cwd,
                repo: None,
                branch: None,
                // This is the HTTP twin of mcp-tools' ask_session: a NAMED session, so it must
                // resume. Without this it silently loses multi-turn memory while the MCP path
                // keeps it — the two surfaces would disagree about what a session is.
                reuse_session: Some(true),
                // Without this the worker key is (agent, cwd) and two named sessions in one
                // directory resume each other. Demonstrated live before the fix.
                session_key: Some(req.name.clone()),
                // Prefer the id the SESSION owns. The worker registry is the legacy fallback and
                // is only consulted when this is None, which is how pre-migration sessions keep
                // working.
                prior_cli_session_id: prior_cli_session.clone(),
                ..Default::default()
            },
            None,
        )
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, AxumJson(serde_json::json!({ "error": e }))))?;
        let cli_session_id = response.cli_session_id.clone();
        let response = response.response;

        let mut sessions = state.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&req.name) {
            session.history.push(format!("user: {}", req.message));
            session.history.push(format!("assistant: {response}"));
            // The session OWNS its CLI id. Persisting it here, beside the history it belongs to,
            // is what makes the two consistent: previously the history lived in this map while
            // the id lived in a worker keyed on (agent, cwd), and nothing kept them in step.
            if cli_session_id.is_some() {
                session.cli_session_id = cli_session_id;
            }
        }
        core_persist_json_file_if_enabled(state.sessions_file.as_ref(), &*sessions).map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(serde_json::json!({ "error": e.to_string() })))
        })?;

        Ok(AxumJson(serde_json::json!({ "status": "ok", "response": response })))
    }

    async fn session_dismiss_route(
        State(state): State<DaemonRuntimeState>,
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
        // Each named session owns its worker record now, so dismissing one can never strand
        // another. The old "does another session share (agent, cwd)?" guard existed only because
        // they DID share, which is the cross-session leak this key change removes.
        let cwd = removed.cwd.clone().unwrap_or_else(|| ".".to_string());
        let _ = dismiss_worker(&removed.agent, &cwd, Some(req.name.as_str())).await;
        core_persist_json_file_if_enabled(state.sessions_file.as_ref(), &*sessions).map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(serde_json::json!({ "error": e.to_string() })))
        })?;
        Ok(AxumJson(serde_json::json!({
            "status": "ok",
            "message": format!("session '{}' dismissed", req.name)
        })))
    }

    async fn session_list_route(
        State(state): State<DaemonRuntimeState>,
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

    async fn abe_task_complete_route(
        State(state): State<DaemonRuntimeState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<TaskCompleteRequest>,
    ) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
            return Err((StatusCode::UNAUTHORIZED, AxumJson(serde_json::json!({ "error": "unauthorized" }))));
        }
        let Some(worktree_path) = state.abe_tasks.worktree_path_for(&req.task_id).await else {
            return Err((StatusCode::NOT_FOUND, AxumJson(serde_json::json!({ "error": "unknown task_id" }))));
        };

        let head_sha = std::process::Command::new("git")
            .arg("-C")
            .arg(&worktree_path)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if head_sha != req.commit_sha {
            return Err((
                StatusCode::CONFLICT,
                AxumJson(serde_json::json!({
                    "error": "commit sha mismatch",
                    "expected": head_sha,
                    "received": req.commit_sha
                })),
            ));
        }

        let modified_files = std::process::Command::new("git")
            .arg("-C")
            .arg(&worktree_path)
            .args(["show", "--name-only", "--pretty=format:", "HEAD"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|body| {
                body.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let outcome = state
            .abe_tasks
            .mark_completed(
                &req.task_id,
                req.commit_sha.clone(),
                modified_files,
                String::new(),
                None,
                None,
            )
            .await;
        match outcome {
            abe::task_tracker::TransitionOutcome::Transitioned
            | abe::task_tracker::TransitionOutcome::AlreadyTerminal => {
                Ok(AxumJson(serde_json::json!({"status":"ok"})))
            }
            abe::task_tracker::TransitionOutcome::NotFound => Err((
                StatusCode::NOT_FOUND,
                AxumJson(serde_json::json!({ "error": "unknown task_id" })),
            )),
        }
    }

    // FEAT-015 (REQ-019): Acquire PID file FIRST, before any other setup.
    // This is the single-instance guarantee — if another daemon is running,
    // we fail loudly here instead of binding to the port (which would fail
    // later anyway with a less-helpful error).
    // The PidFile is held for the lifetime of run_daemon() — dropping it
    // releases the flock and removes the file.
    let triumvirate_home = core_triumvirate_home_dir()?;
    let _pid_file = daemon_core::PidFile::acquire(&triumvirate_home)
        .map_err(|e| anyhow::anyhow!("failed to acquire daemon pid file (another daemon may be running): {e:#}"))?;

    let token = core_ensure_daemon_token(&triumvirate_home)?;
    let bind_addr = core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());
    info!(%bind_addr, "starting triumvirate daemon");

    // Report the RESOLVED config, to the log and to PostHog, before serving anything. The
    // daemon is the only process that dispatches, and it picks its backend from its own
    // env, so this line is the difference between "the config says agy" and "this process
    // will use agy". Those were different for four days and nothing said so.
    let resolved_backend = match mcp_bridge::gemini_backend() {
        mcp_bridge::GeminiBackend::Agy => "agy",
        mcp_bridge::GeminiBackend::GeminiCli => "gemini-cli",
    };
    let (agy_bin, _) = mcp_bridge::agy_command();
    let (agy_max_concurrent, agy_max_rpm) = mcp_bridge::agy_resilience::agy_limits();
    if resolved_backend == "gemini-cli" {
        warn!(
            backend = resolved_backend,
            "DEAD BACKEND: this daemon resolved gemini-cli, which is retired and does not \
             work. TRIUMVIRATE_GEMINI_BACKEND=agy is missing from THIS process's env. Start \
             it with scripts/start-daemon.sh."
        );
    } else {
        info!(backend = resolved_backend, %agy_bin, agy_max_concurrent, agy_max_rpm, "daemon config resolved");
    }
    mcp_bridge::posthog::record_daemon_started(resolved_backend, &agy_bin, agy_max_concurrent, agy_max_rpm);

    // Probe the codex binary so agent_exec can make version-aware flag-injection
    // decisions. Fire-and-forget is fine — if it hasn't completed by the first
    // codex call, the safe `unknown()` fallback is used. In practice the probe
    // finishes in well under a second.
    let (codex_bin, _codex_default_args) = mcp_bridge::codex_command();
    tokio::spawn(async move {
        mcp_bridge::probe_and_cache_codex_capabilities(&codex_bin).await;
    });

    let sessions_file = core_triumvirate_home_dir()
        .ok()
        .map(|home| core_sessions_file_path(&home));
    let sessions = sessions_file
        .as_ref()
        .and_then(|path| core_load_json_file_if_exists::<HashMap<String, SessionState>>(path).ok())
        .unwrap_or_default();
    let metrics = Arc::new(DaemonMetrics::new()?);
    let ws_events = broadcast::channel(256).0;
    let token_db = init_process_token_db()?;
    let observability_bus =
        ObservabilityBus::new(metrics.clone(), ws_events.clone(), token_db.clone());

    // Forward the token scanner's per-cycle summary to PostHog. The scanner lives in
    // token-economics, which mcp-bridge depends on, so it cannot call posthog directly
    // without a dependency cycle; it publishes "token_scan" to this bus and the daemon layer
    // (which sees both) forwards it. This is the only PostHog visibility into the money/quota
    // attribution loop, which was otherwise entirely silent.
    {
        let mut rx = ws_events.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg) else { continue };
                        if v.get("type").and_then(|t| t.as_str()) != Some("token_scan") {
                            continue;
                        }
                        let p = v.get("payload").cloned().unwrap_or_default();
                        mcp_bridge::posthog::record_token_scan(
                            p.get("source").and_then(|s| s.as_str()).unwrap_or("unknown"),
                            p.get("records").and_then(|n| n.as_u64()).unwrap_or(0),
                            p.get("tokens").and_then(|n| n.as_u64()).unwrap_or(0),
                            p.get("cost_usd").and_then(|n| n.as_f64()).unwrap_or(0.0),
                            p.get("scan_duration_ms").and_then(|n| n.as_u64()).unwrap_or(0),
                        );
                    }
                    // A lagged broadcast receiver dropped some messages under a burst. Keep
                    // going; losing an aggregate telemetry event must never stop the loop.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    let state = DaemonRuntimeState::new(
        token,
        Arc::new(Mutex::new(HashMap::new())),
        bind_addr.clone(),
        Arc::new(Mutex::new(sessions)),
        sessions_file,
        abe::task_tracker::TaskTracker::with_observability(
            metrics.clone(),
            Some(ws_events.clone()),
        ),
        Arc::new(Mutex::new(VecDeque::new())),
        Arc::new(Mutex::new(VecDeque::new())),
        metrics.clone(),
        ws_events,
    );
    set_process_metrics(state.metrics.clone());

    // FEAT-013 (REQ-020) T-007.5: Subscribe-before-read.
    // Acquire the buffer-fill receiver NOW, before the HTTP server starts
    // listening and before any ABE dispatch can publish events. The subscribe
    // call must come before the first event is sent on the broadcast channel —
    // broadcast::Receiver only buffers events that arrive AFTER subscription.
    // Since this runs during run_daemon init, no producer is active yet, so
    // we are guaranteed to capture every subsequent event.
    let buffer_rx = state.ws_events.subscribe();
    tokio::spawn(daemon_core::run_replay_buffer_fill(
        buffer_rx,
        state.replay_buffer.clone(),
        state.last_event_seq.clone(),
    ));
    let token_db_path = core_triumvirate_home_dir()?.join("token-economics.db");
    let token_db = daemon_http::open_token_db(&token_db_path)?;
    let scanner_token_db = token_db.clone();
    let http_state = DaemonHttpState {
        token: state.token.clone(),
        queues: state.queues.clone(),
        ledger_project_lru: state.ledger_project_lru.clone(),
        marker_parse_window: state.marker_parse_window.clone(),
        metrics: state.metrics.clone(),
        ws_events: state.ws_events.clone(),
        token_db,
        ask_agent_executor: Arc::new(|req| execute_ask_agent_boxed(req, None)),
    };
    run_startup_gc_if_needed(&state).await;
    let app = Router::new()
        .route("/", get(daemon_http::dashboard_root_route))
        .route("/assets/{*path}", get(daemon_http::dashboard_assets_route))
        .route(
            "/metrics",
            get_service(daemon_http::metrics_route.with_state(http_state.clone())),
        )
        .route(
            "/api/tokens/summary",
            get_service(daemon_http::token_summary_route.with_state(http_state.clone())),
        )
        .route(
            "/api/tokens/by-build",
            get_service(daemon_http::token_by_build_route.with_state(http_state.clone())),
        )
        .route(
            "/api/tokens/by-session",
            get_service(daemon_http::token_by_session_route.with_state(http_state.clone())),
        )
        // FEAT-012 (REQ-017) T-008: Pantheon v3.9.0 REST endpoints.
        // These take the runtime `state` (DaemonRuntimeState), not http_state,
        // because they read `abe_tasks` and `fleet_v2_states` directly.
        .route("/api/workers", get(api_workers))
        .route("/api/fleet", get(api_fleet))
        .route("/api/fleet/{build_id}", get(api_fleet_by_id))
        // FEAT-013 (REQ-020) T-009: Pantheon v3.9.0 state snapshot +
        // replay-aware WebSocket. Both handlers take State<DaemonRuntimeState>
        // (not DaemonHttpState), so they register with plain `get(...)`
        // rather than `get_service(...)`. Auth is per-handler via
        // is_bearer_authorized — NOT middleware. The legacy /ws route below
        // is untouched so `triumvirate watch` still works.
        .route("/api/state", get(api_state))
        .route("/ws/v2", get(ws_v2))
        .route(
            "/ws",
            get_service(daemon_http::ws_route.with_state(http_state.clone())),
        )
        .route("/health", get(health))
        .route("/status", get(status))
        .route(
            "/ledger/wake",
            post_service(daemon_http::ledger_wake_route.with_state(http_state.clone())),
        )
        .route(
            "/ledger/health",
            get_service(daemon_http::ledger_health_route.with_state(http_state.clone())),
        )
        .route(
            "/ledger/query",
            post_service(daemon_http::ledger_query_route.with_state(http_state.clone())),
        )
        .route(
            "/ledger/session",
            post_service(daemon_http::ledger_session_route.with_state(http_state.clone())),
        )
        .route(
            "/ledger/record",
            post_service(daemon_http::ledger_record_route.with_state(http_state.clone())),
        )
        .route(
            "/ledger/gc",
            post_service(daemon_http::ledger_gc_route.with_state(http_state.clone())),
        )
        .route(
            "/lesson/add",
            post_service(daemon_http::lesson_add_route.with_state(http_state.clone())),
        )
        .route(
            "/lesson/query",
            post_service(daemon_http::lesson_query_route.with_state(http_state.clone())),
        )
        .route(
            "/lesson/validate",
            post_service(daemon_http::lesson_validate_route.with_state(http_state.clone())),
        )
        .route(
            "/lesson/list",
            post_service(daemon_http::lesson_list_route.with_state(http_state.clone())),
        )
        .route(
            "/ask-agent",
            post_service(daemon_http::ask_agent_route.with_state(http_state.clone())),
        )
        .route(
            "/memory/write",
            post_service(daemon_http::memory_write_route.with_state(http_state.clone())),
        )
        .route(
            "/memory/read",
            post_service(daemon_http::memory_read_route.with_state(http_state.clone())),
        )
        .route(
            "/scratchpad/write",
            post_service(daemon_http::scratchpad_write_route.with_state(http_state.clone())),
        )
        .route(
            "/scratchpad/list",
            post_service(daemon_http::scratchpad_list_route.with_state(http_state.clone())),
        )
        .route(
            "/outbox/recent",
            post_service(daemon_http::outbox_recent_route.with_state(http_state.clone())),
        )
        .route(
            "/fallback/list",
            post_service(daemon_http::fallback_list_route.with_state(http_state.clone())),
        )
        .route(
            "/fallback/ack",
            post_service(daemon_http::fallback_ack_route.with_state(http_state.clone())),
        )
        .route(
            "/fallback/gc",
            post_service(daemon_http::fallback_gc_route.with_state(http_state.clone())),
        )
        .route("/session/spawn", post(session_spawn_route))
        .route("/session/ask", post(session_ask_route))
        .route("/session/dismiss", post(session_dismiss_route))
        .route("/session/list", get(session_list_route))
        .route("/abe/task-complete", post(abe_task_complete_route))
        .nest_service("/mcp", {
            let mcp_bridge = McpBridge::new();
            let cancel = tokio_util::sync::CancellationToken::new();
            http_mcp::build_mcp_router(mcp_bridge, state.token.clone(), cancel)
        })
        .route("/{*path}", get(daemon_http::dashboard_spa_fallback_route))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state.clone(), metrics_middleware));
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(%bind_addr, "daemon listener bound");
    tokio::spawn(async {
        prewarm_daemon_workers().await;
    });
    // M9: periodically reap leaked agy temp files (the fleet path can't RAII-clean them
    // because its child outlives the invocation). Cheap; runs regardless of backend.
    tokio::spawn(async {
        loop {
            mcp_bridge::agy::sweep_stale_temp_files();
            tokio::time::sleep(Duration::from_secs(1800)).await;
        }
    });
    // REQ-056: periodic agy health probe (only when the agy backend is selected). Runs
    // the production capture path and records capture/backend health for /health; never
    // touches request traffic, so it catches a silent stdout-drop regression that real
    // dispatches cannot distinguish from a legitimate empty answer.
    if matches!(mcp_bridge::gemini_backend(), mcp_bridge::GeminiBackend::Agy) {
        tokio::spawn(async {
            let interval = mcp_bridge::agy_resilience::agy_health_probe_interval();
            loop {
                tokio::time::sleep(interval).await;
                agy::health_probe().await;
            }
        });
    }
    tokio::spawn({
        let scanner_bus = observability_bus.clone();
        async move {
            token_economics::run_scanner_loop(scanner_token_db, scanner_bus).await;
        }
    });
    tokio::spawn({
        let sweep_state = state.clone();
        async move {
            let interval = ledger_sweep_interval();
            loop {
                sleep(interval).await;
                run_ledger_sweep_once(&sweep_state).await;
            }
        }
    });
    axum::serve(listener, app).await?;
    Ok(())
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            // Label flip: the sibling now presents as Antigravity (agent="gemini"
            // is the internal key; the rendered progress label is the product name).
            assert!(all_logs.iter().any(|m| m.contains("Antigravity: sent")));
            assert!(all_logs.iter().any(|m| m.contains("Antigravity: responded")));
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

    fn write_mock_gemini_marker_probe_script() -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-gemini-marker-probe-{now}.sh"));
        let script = r#"#!/bin/sh
echo '{"jsonrpc":"2.0","method":"session/ready","params":{"text":"mock ready"}}'
payload="$(cat)"
echo "$payload" | grep -q '<triumvirate_tool name="ledger_record">'
if [ $? -eq 0 ]; then
  echo '{"jsonrpc":"2.0","id":1,"result":{"text":"marker_probe=present"}}'
else
  echo '{"jsonrpc":"2.0","id":1,"result":{"text":"marker_probe=missing"}}'
fi
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

    fn write_codex_args_capture_script(args_path: &std::path::Path, response_text: &str) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("codex-args-capture-{now}.sh"));
        let script = format!(
            "#!/bin/sh\n\
printf '%s\n' \"$@\" > \"{args_path}\"\n\
IFS= read -r _line\n\
echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"text\":\"{response_text}\"}}}}'\n",
            args_path = args_path.display(),
            response_text = response_text
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_gemini_stream_usage_script() -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("gemini-stream-usage-{now}.sh"));
        let script = r#"#!/bin/sh
IFS= read -r _line
echo '{"type":"init","session_id":"session-usage-1","model":"gemini-pro"}'
echo '{"type":"tool_use","tool_id":"tool-1","tool_name":"read_file","parameters":{"path":"src/lib.rs"}}'
echo '{"type":"tool_result","tool_id":"tool-1","status":"success"}'
echo '{"type":"message","role":"assistant","content":"stream usage done"}'
echo '{"type":"result","stats":{"input_tokens":123,"output_tokens":45,"cached":10,"total_tokens":178}}'
"#;
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_codex_worktree_commit_script(
        rel_path: &str,
        content: &str,
        commit_message: &str,
    ) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("codex-worktree-commit-{now}.sh"));
        let parent = std::path::Path::new(rel_path)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let mkdir_line = if parent.is_empty() {
            String::new()
        } else {
            format!("mkdir -p \"{parent}\"\n")
        };
        let script = format!(
            "#!/usr/bin/env bash\n\
set -euo pipefail\n\
{mkdir_line}cat > \"{rel_path}\" <<'PAYLOAD'\n\
{content}\n\
PAYLOAD\n\
git add \"{rel_path}\"\n\
git commit -m \"{commit_message}\"\n",
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_codex_sleep_script(seconds: u64) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("codex-sleep-{now}.sh"));
        let script = format!(
            "#!/usr/bin/env bash\n\
set -euo pipefail\n\
sleep {seconds}\n\
exit 0\n"
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_codex_custom_script(script_body: &str) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("codex-custom-{now}.sh"));
        let script = format!("#!/usr/bin/env bash\nset -euo pipefail\n{script_body}\n");
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

        assert!(raw_text.contains("mock-gemini received:"));
        assert!(raw_text.contains("test message"));
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
    async fn rollback_gemini_cli_receives_stream_json_and_parses_ndjson() -> anyhow::Result<()> {
        // REQ-083: with the backend explicitly set to gemini-cli (rollback), the
        // gemini-cli path still passes `-o stream-json` AND parses the NDJSON stream —
        // i.e. the selector did NOT route to agy. A non-"mock-" script name forces the
        // real `run_gemini_cli_process_with_session` (not the test mock connector).
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let argv_file = std::env::temp_dir().join(format!("gemini-rollback-argv-{now}.txt"));
        let script_path = std::env::temp_dir().join(format!("gemini-rollback-{now}.sh"));
        let script = format!(
            "#!/bin/sh\n\
echo \"$@\" > \"{argv}\"\n\
echo '{{\"type\":\"init\",\"session_id\":\"sess-rollback\",\"model\":\"gemini-2.5-pro\"}}'\n\
echo '{{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"rollback NDJSON parsed OK\"}}'\n\
echo '{{\"type\":\"result\",\"stats\":{{\"input_tokens\":10,\"output_tokens\":5,\"total_tokens\":15}}}}'\n",
            argv = argv_file.display()
        );
        fs::write(&script_path, &script)?;
        let mut perms = fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::set_var("TRIUMVIRATE_GEMINI_BACKEND", "gemini-cli");
        }

        let resp = crate::agent_exec::execute_ask_agent(
            &shared_types::AskAgentRequest {
                agent: "gemini".to_string(),
                message: "test message".to_string(),
                cwd: Some("/tmp".to_string()),
                repo: None,
                branch: None,
                ..Default::default()
            },
            None,
        )
        .await
        .map_err(|e| anyhow::anyhow!("execute_ask_agent failed: {e}"))?;

        // The NDJSON was parsed by the real GeminiStreamParser (not agy plain text).
        assert!(
            resp.response.contains("rollback NDJSON parsed OK"),
            "expected parsed NDJSON, got: {}",
            resp.response
        );
        // The gemini-cli path passed `-o stream-json` (REQ-083).
        let argv = fs::read_to_string(&argv_file)?;
        assert!(
            argv.contains("-o") && argv.contains("stream-json"),
            "gemini-cli must receive -o stream-json; argv was: {argv}"
        );

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_BACKEND");
        }
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(&argv_file);
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_gemini_injects_tool_marker_instructions() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let script_path = write_mock_gemini_marker_probe_script()?;
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
            "cwd": "/tmp/project"
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
        assert!(raw_text.contains("marker_probe=present"));

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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    async fn ask_agent_codex_adds_full_auto_only_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-codex-full-auto-{now}"));
        fs::create_dir_all(&test_home)?;
        let args_file = test_home.join("codex-args.txt");
        let script_path = write_codex_args_capture_script(&args_file, "codex args capture done")?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
            std::env::set_var("TRIUMVIRATE_CODEX_AUTO_APPROVE", "1");
            // Exercise the workspace-write policy path: yolo (default) suppresses it.
            std::env::set_var("TRIUMVIRATE_CODEX_SANDBOX", "1");
            std::env::remove_var("TRIUMVIRATE_REQUIRE_PEER_REVIEW");
        }

        let req = AskAgentRequest {
            agent: "codex".to_string(),
            message: "capture args".to_string(),
            cwd: Some(test_home.display().to_string()),
            repo: Some("triumvirate".to_string()),
            branch: Some("feat/mcp-first".to_string()),
            ..Default::default()
        };
        let _ = execute_ask_agent(&req, None).await.map_err(anyhow::Error::msg)?;
        let captured = fs::read_to_string(&args_file)?;
        // 0.145: inject the explicit workspace-write policy, not the deprecated --full-auto.
        assert!(captured.lines().any(|line| line == "--sandbox"), "sandbox flag injected");
        assert!(captured.lines().any(|line| line == "workspace-write"));
        assert!(captured.lines().any(|line| line == "--ask-for-approval"));
        assert!(captured.lines().any(|line| line == "never"));
        assert!(!captured.lines().any(|line| line == "--full-auto"), "no deprecated flag");

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_CODEX_AUTO_APPROVE") };
        let _ = execute_ask_agent(&req, None).await.map_err(anyhow::Error::msg)?;
        let captured_without = fs::read_to_string(&args_file)?;
        assert!(!captured_without.lines().any(|line| line == "--sandbox"));

        // Yolo (default — sandbox off): codex gets the no-sandbox bypass flag, and
        // --full-auto is suppressed (codex rejects it alongside another policy flag).
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_CODEX_SANDBOX") };
        let _ = execute_ask_agent(&req, None).await.map_err(anyhow::Error::msg)?;
        let captured_yolo = fs::read_to_string(&args_file)?;
        assert!(
            captured_yolo
                .lines()
                .any(|line| line == "--dangerously-bypass-approvals-and-sandbox"),
            "yolo default passes the no-sandbox bypass flag"
        );
        assert!(!captured_yolo.lines().any(|line| line == "--sandbox"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
            std::env::remove_var("TRIUMVIRATE_CODEX_AUTO_APPROVE");
            std::env::remove_var("TRIUMVIRATE_CODEX_SANDBOX");
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_codex_auto_approve_writes_ledger_record() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let test_home =
            std::env::temp_dir().join(format!("triumvirate-codex-auto-approve-ledger-{now}"));
        fs::create_dir_all(&test_home)?;
        let args_file = test_home.join("codex-args.txt");
        let script_path = write_codex_args_capture_script(&args_file, "codex auto approve done")?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
            std::env::set_var("TRIUMVIRATE_CODEX_AUTO_APPROVE", "1");
            std::env::remove_var("TRIUMVIRATE_REQUIRE_PEER_REVIEW");
        }

        let req = AskAgentRequest {
            agent: "codex".to_string(),
            message: "auto approve ledger".to_string(),
            cwd: Some(test_home.display().to_string()),
            repo: Some("triumvirate".to_string()),
            branch: Some("feat/mcp-first".to_string()),
            ..Default::default()
        };
        let _ = execute_ask_agent(&req, None).await.map_err(anyhow::Error::msg)?;

        let store = LedgerStore::open(test_home.clone())?;
        let summaries = store.query("Codex", 10)?;
        assert!(summaries.iter().any(|summary| summary.summary_type == "auto_approved"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
            std::env::remove_var("TRIUMVIRATE_CODEX_AUTO_APPROVE");
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_invalid_stale_session_recovers_with_fresh_spawn() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_worker_registry_for_tests().await;
        let script_path = write_invalid_session_recovery_script("gemini")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let cwd = "/tmp/invalid-session-recovery";
        let _ = acquire_worker("gemini", cwd, None).await;
        update_worker_session("gemini", cwd, None, Some("stale-session-id".to_string())).await;

        let response = execute_ask_agent(
            &AskAgentRequest {
                agent: "gemini".to_string(),
                message: "recover please".to_string(),
                cwd: Some(cwd.to_string()),
                repo: None,
                branch: None,
                // Stale-session recovery only applies to a caller that RESUMES. A one-shot
                // ask_agent no longer reads the cached session at all (that inheritance was the
                // bug: the worker registry is keyed only by (agent, cwd), so a one-shot would
                // silently adopt — and get billed for — whatever named session last ran in this
                // directory). So this scenario belongs to the session-scoped path.
                reuse_session: Some(true),
                ..Default::default()
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

    /// The other half of the same rule, and the half that costs money: a ONE-SHOT ask_agent must
    /// never adopt the session cached for (agent, cwd).
    ///
    /// The worker registry is keyed only by (agent, cwd), so before this a one-shot would resume
    /// whatever named session last ran in that directory — replaying its entire transcript as
    /// input on every call (measured: 189,930 input tokens to answer "ok", against 26,215 fresh)
    /// and then clobbering that session's id on the way out.
    ///
    /// If this test ever fails, the cached session has leaked back into the one-shot path.
    #[tokio::test]
    async fn one_shot_ask_agent_does_not_inherit_a_cached_session() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_worker_registry_for_tests().await;
        let script_path = write_invalid_session_recovery_script("gemini")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let cwd = "/tmp/one-shot-no-inherit";
        let _ = acquire_worker("gemini", cwd, None).await;
        update_worker_session("gemini", cwd, None, Some("stale-session-id".to_string())).await;

        let response = execute_ask_agent(
            &AskAgentRequest {
                agent: "gemini".to_string(),
                message: "recover please".to_string(),
                cwd: Some(cwd.to_string()),
                repo: None,
                branch: None,
                // reuse_session deliberately unset -> this is a one-shot.
                ..Default::default()
            },
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;

        // It never touched the stale session, so there was nothing to invalidate.
        assert!(
            !response
                .lifecycle
                .iter()
                .any(|e| e.state == "SESSION_INVALIDATED"),
            "one-shot must not resume the cached session"
        );
        assert!(response.lifecycle.iter().any(|e| e.state == "DONE"));

        // And it must not have overwritten the session a named ask_session depends on.
        let worker = acquire_worker("gemini", cwd, None).await;
        assert_eq!(
            worker.session_id.as_deref(),
            Some("stale-session-id"),
            "one-shot must not clobber the cached session id"
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
    async fn ask_agent_requires_peer_review_when_env_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let script_path = write_mock_gemini_script()?;
        let temp = tempfile::tempdir()?;
        let project_root = temp.path().join("peer-review-required");
        fs::create_dir_all(&project_root)?;
        let project_root_str = project_root.display().to_string();

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::set_var("TRIUMVIRATE_REQUIRE_PEER_REVIEW", "1");
        }

        let reviewed = execute_ask_agent(
            &AskAgentRequest {
                agent: "gemini".to_string(),
                message: "test message".to_string(),
                cwd: Some(project_root_str.clone()),
                repo: None,
                branch: None,
                ..Default::default()
            },
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        assert!(reviewed.response.contains("mock-gemini received"));
        assert!(reviewed
            .lifecycle
            .iter()
            .any(|event| event.state == "REVIEW_PENDING"));
        assert!(reviewed
            .lifecycle
            .iter()
            .any(|event| event.state == "REVIEW_DONE"));
        let pending_detail = reviewed
            .lifecycle
            .iter()
            .find(|event| event.state == "REVIEW_PENDING")
            .map(|event| event.detail.clone())
            .expect("review pending lifecycle detail");
        let review_id = pending_detail
            .split(": ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("review id in lifecycle detail");
        let engine = PeerReviewEngine::new(project_root.clone())?;
        let stored = engine
            .get_review(review_id)?
            .expect("stored review should exist");
        assert_eq!(stored.state, "done");
        assert_eq!(stored.verdict.as_deref(), Some("approve"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_REQUIRE_PEER_REVIEW");
        }
        let unreviewed = execute_ask_agent(
            &AskAgentRequest {
                agent: "gemini".to_string(),
                message: "test message".to_string(),
                cwd: Some(project_root_str),
                repo: None,
                branch: None,
                ..Default::default()
            },
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        assert!(unreviewed.response.contains("mock-gemini received"));
        assert!(!unreviewed
            .lifecycle
            .iter()
            .any(|event| event.state == "REVIEW_PENDING"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::remove_var("TRIUMVIRATE_REQUIRE_PEER_REVIEW");
        }
        let _ = fs::remove_file(script_path);
        Ok(())
    }

    #[tokio::test]
    async fn persistent_worker_reuse_second_call_is_faster_and_marked_reused() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            ..Default::default()
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
        // Lifecycle details are read by a human, so they carry the PRODUCT name. The internal
        // dispatch key is still `gemini` — this asserts the presentation layer, not the key.
        assert!(second
            .lifecycle
            .iter()
            .any(|e| e.detail.contains("Reused Antigravity worker")));
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        // REQ-GROK-003: /status renders mcp_bridge::supported_agent_names() verbatim. Asserting a
        // hand-written literal here is what let this test pin an answer two agents stale while
        // `claude` was dispatchable and advertised nowhere. Build the expectation from the source
        // of truth so the test cannot drift from it again.
        let expected_agents = format!(
            "\"supported_agents\":[{}]",
            mcp_bridge::supported_agent_names()
                .iter()
                .map(|a| format!("\"{a}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(
            status_text.contains(&expected_agents),
            "status must advertise exactly the dispatchable set; wanted {expected_agents} in {status_text}"
        );
        assert!(status_text.contains("\"daemon_bind_addr\":\"127.0.0.1:7777\""));

        client.cancel().await?;
        server_handle.await??;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_DAEMON_BIND_ADDR") };
        Ok(())
    }

    #[tokio::test]
    async fn get_status_includes_pending_fallback_tickets() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

    // TODO(issue #24): core_project_queue_key was refactored out.
    // This test needs updating for the current architecture.
    #[cfg(any())]
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
                version: Some("3.9.0".to_string()),
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

    // TODO(issue #24): QueueRegistry / core_acquire_project_queue refactored out.
    #[cfg(any())]
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            Ok(AxumJson(AskAgentResponse::direct(
                "daemon-req-1".to_string(),
                req.agent,
                format!("daemon echo: {}", req.message),
                vec![LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: "served by daemon".to_string(),
                }],
            )))
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
            ..Default::default()
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            Ok(AxumJson(AskAgentResponse::direct(
                "daemon-req-3".to_string(),
                req.agent,
                "daemon path used".to_string(),
                vec![LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: "daemon served".to_string(),
                }],
            )))
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
    async fn mcp_ask_agent_returns_daemon_recovery_error_when_unreachable() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-mcp-daemon-down-{now}"));
        fs::create_dir_all(&test_home)?;
        fs::write(test_home.join("daemon.token"), "daemon-down-token\n")?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var("TRIUMVIRATE_DAEMON_AUTOSTART", "0");
            std::env::set_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL", "http://127.0.0.1:9/ask-agent");
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
            "message": "daemon down case"
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_agent")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await
            .expect("call should return MCP error payload");
        assert_eq!(result.is_error, Some(true));
        let err_text = result
            .content
            .first()
            .and_then(|content| content.raw.as_text())
            .map(|text| text.text.clone())
            .unwrap_or_default();
        assert!(err_text.contains("daemon"));
        assert!(err_text.contains("triumvirate daemon"));

        client.cancel().await?;
        server_handle.await??;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_AUTOSTART");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
    }

    // TODO(issue #24): MemoryEntry refactored out of shared-types.
    #[cfg(any())]
    #[tokio::test]
    async fn mcp_memory_tools_use_daemon_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    working_state: None,
                    token_usage: None,
                    tool_name: None,
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

    // TODO(issue #24): TokenDb.db_path was refactored out.
    #[cfg(any())]
    #[tokio::test]
    async fn ask_agent_writes_outbox_events() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-outbox-{now}"));
        fs::create_dir_all(&test_home)?;
        let script_path = write_gemini_stream_usage_script()?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", &test_home);
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        fs::write(
            test_home.join("BUILD_STATE.json"),
            serde_json::json!({
                "build_id": "abe-v3-main"
            })
            .to_string(),
        )?;
        fs::create_dir_all(test_home.join(".triumvirate"))?;
        fs::write(
            test_home.join(".triumvirate").join("contract.json"),
            serde_json::json!({
                "task_id": "T-114",
                "wave": 3
            })
            .to_string(),
        )?;

        let response = execute_ask_agent(&AskAgentRequest {
            agent: "gemini".to_string(),
            message: "outbox check".to_string(),
            cwd: Some(test_home.display().to_string()),
            repo: Some("triumvirate".to_string()),
            branch: Some("feat/mcp-first".to_string()),
            ..Default::default()
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
        assert!(outbox.contains("\"tool_name\":\"read_file\""));
        assert!(outbox.contains("\"token_usage\":{\"input\":123,\"output\":45,\"cached\":10,\"total\":178}"));

        let token_db = process_token_db().ok_or_else(|| anyhow::anyhow!("token db not initialized"))?;
        let query = "SELECT COUNT(*) FROM token_records \
            WHERE agent='gemini' \
              AND session_id='session-usage-1' \
              AND total_tokens=178 \
              AND build_id='abe-v3-main' \
              AND task_id='T-114' \
              AND wave=3;";
        let sqlite_output = std::process::Command::new("sqlite3")
            .arg(&token_db.db_path)
            .arg(query)
            .output()?;
        assert!(sqlite_output.status.success());
        let count = String::from_utf8_lossy(&sqlite_output.stdout)
            .trim()
            .parse::<u64>()
            .unwrap_or(0);
        assert!(count >= 1);

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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            ..Default::default()
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            working_state: None,
            token_usage: None,
            tool_name: None,
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
            working_state: None,
            token_usage: None,
            tool_name: None,
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

    // TODO(issue #24): acknowledge_fallback_path refactored out.
    #[cfg(any())]
    #[test]
    fn fallback_ack_rejects_paths_outside_dead_drop() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    parent_session_id: None,
                    root_session_id: None,
                    pantheon_session_id: None,
            cli_session_id: None,
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

    #[tokio::test]
    async fn ledger_health_tool_returns_health_payload() -> anyhow::Result<()> {
        let bridge = McpBridge::new_ephemeral();
        let out = bridge.ledger_health().await.map_err(anyhow::Error::msg)?;
        assert!(!out.0.status.is_empty());
        assert!(out.0.db_size_bytes >= 0);
        Ok(())
    }

    #[tokio::test]
    async fn ledger_record_and_query_tools_round_trip() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_cwd = std::env::current_dir()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let project_root = std::env::temp_dir().join(format!("triumvirate-ledger-mcp-{now}"));
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))?;
        std::env::set_current_dir(&project_root)?;

        let bridge = McpBridge::new_ephemeral();
        bridge
            .ledger_record(Parameters(ManualRecord {
                session_id: None,
                title: "test".to_string(),
                narrative: "round trip".to_string(),
                facts_json: None,
                concepts_json: None,
                affected_files_json: None,
                summary_type: "architecture_decision".to_string(),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        let out = bridge
            .ledger_query(Parameters(LedgerQueryRequest {
                query: "test".to_string(),
                limit: Some(10),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(out.0.summaries.iter().any(|summary| summary.title == "test"));

        std::env::set_current_dir(&original_cwd)?;
        let _ = fs::remove_dir_all(project_root);
        Ok(())
    }

    #[tokio::test]
    async fn ledger_gc_tool_returns_gc_counts() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_cwd = std::env::current_dir()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let project_root = std::env::temp_dir().join(format!("triumvirate-ledger-gc-mcp-{now}"));
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))?;
        std::env::set_current_dir(&project_root)?;

        let _store = LedgerStore::open(project_root.clone())?;

        let bridge = McpBridge::new_ephemeral();
        let gc = bridge.ledger_gc().await.map_err(anyhow::Error::msg)?;
        assert_eq!(gc.0.events_scanned, 0);
        assert_eq!(gc.0.events_deleted, 0);
        assert_eq!(gc.0.dead_drop_deleted, 0);

        std::env::set_current_dir(&original_cwd)?;
        let _ = fs::remove_dir_all(project_root);
        Ok(())
    }

    #[tokio::test]
    async fn lesson_tools_round_trip() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_cwd = std::env::current_dir()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let project_root = std::env::temp_dir().join(format!("triumvirate-lesson-mcp-{now}"));
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))?;
        std::env::set_current_dir(&project_root)?;

        let bridge = McpBridge::new_ephemeral();
        let lesson_id = bridge
            .lesson_add(Parameters(NewLesson {
                title: "WAL lesson".to_string(),
                body: "Use WAL for contention".to_string(),
                source_session_id: Some("sess-1".to_string()),
                initial_confidence: 0.8,
                tags_json: Some("[\"sqlite\",\"wal\"]".to_string()),
                req_ids_json: Some("[\"REQ-021\"]".to_string()),
            }))
            .await
            .map_err(anyhow::Error::msg)?
            .0
            .lesson_id;

        let queried = bridge
            .lesson_query(Parameters(LessonQueryRequest {
                query: "WAL".to_string(),
                min_confidence: Some(0.1),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(queried.0.lessons.iter().any(|lesson| lesson.lesson_id == lesson_id));

        bridge
            .lesson_validate(Parameters(LessonValidateRequest { lesson_id }))
            .await
            .map_err(anyhow::Error::msg)?;
        let listed = bridge
            .lesson_list(Parameters(LessonListRequest {
                tags: Some(vec!["sqlite".to_string()]),
                stale_days: None,
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        let after = listed
            .0
            .lessons
            .iter()
            .find(|lesson| lesson.lesson_id == lesson_id)
            .map(|lesson| lesson.last_validated_at.clone())
            .unwrap_or_default();
        assert!(!after.is_empty());

        std::env::set_current_dir(&original_cwd)?;
        let _ = fs::remove_dir_all(project_root);
        Ok(())
    }

    #[tokio::test]
    async fn fleet_spawn_and_status_tools_round_trip() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_cwd = std::env::current_dir()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let project_root = std::env::temp_dir().join(format!("triumvirate-fleet-mcp-{now}"));
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))?;
        std::process::Command::new("git")
            .arg("init")
            .arg(&project_root)
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "user.email", "fleet@test.local"])
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "user.name", "Fleet Test"])
            .status()?;
        fs::write(project_root.join("README.md"), "fleet test\n")?;
        fs::write(project_root.join(".gitignore"), ".triumvirate/\n")?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["add", "README.md", ".gitignore"])
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["commit", "-m", "init"])
            .status()?;

        std::env::set_current_dir(&project_root)?;
        let bridge = McpBridge::new_ephemeral();
        let dry = bridge
            .fleet_spawn(Parameters(FleetSpawnRequest {
                project_root: None,
                agents: Some(vec!["codex".to_string(), "gemini".to_string()]),
                dry_run: Some(true),
                wait: Some(false),
                task_description: Some("test".to_string()),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(dry.0.plan.contains("agent count: 2"));
        assert!(dry.0.plan.contains("head sha:"));

        let execute = bridge
            .fleet_spawn(Parameters(FleetSpawnRequest {
                project_root: None,
                agents: Some(vec!["codex".to_string(), "gemini".to_string()]),
                dry_run: Some(false),
                wait: Some(true),
                task_description: Some("test".to_string()),
            }))
            .await
            .map_err(anyhow::Error::msg)?;

        let status = bridge
            .fleet_status(Parameters(FleetStatusRequest {
                fleet_id: execute.0.fleet_id.clone(),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(status.0.fleet_id, execute.0.fleet_id);
        assert_eq!(status.0.state, "running");
        assert_eq!(status.0.worktree_paths.len(), 2);

        std::env::set_current_dir(&original_cwd)?;
        let _ = fs::remove_dir_all(project_root);
        Ok(())
    }

    #[tokio::test]
    async fn review_tools_round_trip() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_cwd = std::env::current_dir()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let project_root = std::env::temp_dir().join(format!("triumvirate-review-mcp-{now}"));
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))?;
        let _ = LedgerStore::open(project_root.clone())?;
        std::env::set_current_dir(&project_root)?;

        let bridge = McpBridge::new_ephemeral();
        let requested = bridge
            .review_request(Parameters(ReviewRequestTool {
                project_root: None,
                fleet_id: Some("fleet-1".to_string()),
                author_agent: "codex".to_string(),
                artifact: "diff -- fake".to_string(),
                review_type: "code".to_string(),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(!requested.0.review_id.is_empty());
        assert_ne!(requested.0.reviewer_agent.as_deref(), Some("codex"));

        bridge
            .review_submit(Parameters(ReviewSubmitRequest {
                project_root: None,
                review_id: requested.0.review_id.clone(),
                verdict: "approve".to_string(),
                comments: Some("looks good".to_string()),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        let status = bridge
            .review_status(Parameters(ReviewStatusRequest {
                project_root: None,
                review_id: requested.0.review_id.clone(),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(status.0.state, "done");
        assert_eq!(status.0.verdict.as_deref(), Some("approve"));

        std::env::set_current_dir(&original_cwd)?;
        let _ = fs::remove_dir_all(project_root);
        Ok(())
    }

    #[tokio::test]
    async fn abe_phase1_dispatch_poll_output_review_and_cancel() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_cwd = std::env::current_dir()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let project_root = std::env::temp_dir().join(format!("triumvirate-abe-phase1-{now}"));
        fs::create_dir_all(&project_root)?;

        std::process::Command::new("git")
            .arg("init")
            .arg(&project_root)
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "user.email", "abe@test.local"])
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "user.name", "ABE Test"])
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "extensions.worktreeConfig", "true"])
            .status()?;
        fs::write(project_root.join("README.md"), "abe phase1\n")?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["add", "README.md"])
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["commit", "-m", "init"])
            .status()?;

        let head_sha = String::from_utf8(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&project_root)
                .args(["rev-parse", "HEAD"])
                .output()?
                .stdout,
        )?
        .trim()
        .to_string();

        std::env::set_current_dir(&project_root)?;

        let codex_commit_script = write_codex_worktree_commit_script(
            "src/phase1.rs",
            "pub fn phase1_ready() -> bool { true }",
            "T-021: complete phase1 acceptance",
        )?;
        let gemini_script = write_mock_gemini_script()?;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", codex_commit_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", gemini_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let bridge = McpBridge::new_ephemeral();
        let contract = shared_types::ContractFields {
            task_id: "T-021".to_string(),
            req_ids: vec!["REQ-A1.1".to_string(), "REQ-A1.2".to_string()],
            wave: 6,
            file_policy: shared_types::FilePolicy::DefaultDeny,
            allowed_files: vec!["src/phase1.rs".to_string()],
            forbidden_files: vec![],
            allowed_commands: vec![vec!["true".to_string()]],
            forbidden_commands: vec![],
            commit_format: "^T-021:".to_string(),
            test_command: "true".to_string(),
            // Do not use the PRODUCTION timeout as this test's liveness budget.
            //
            // This is a happy-path test: it asserts the task reaches Completed. At 1s it
            // raced its own timeout watcher and lost, once, under `cargo test --workspace`.
            // The mock worker is not a no-op — it writes a file and runs real `git add` +
            // `git commit`, and a slow commit can exceed one second on a loaded machine. The
            // watcher then marks Timeout, and a terminal status can never be overwritten by
            // the completion that arrives a moment later, so the assertion fails.
            //
            // 10s sits between the two bounds that matter: far above real git work, and still
            // inside the poll loop's 14s budget (140 x 100ms) below, so a genuinely wedged
            // task still surfaces as Timeout instead of hanging the suite. Timeout BEHAVIOR
            // is covered by its own test; this one should not depend on losing a race.
            task_timeout_sec: 10,
            done_when: "phase 1 e2e verified".to_string(),
            reality_test: "dispatch->status->output->review->cancel".to_string(),
            sandbox_permissions: None,
        };

        let dispatched = bridge
            .dispatch_codex_worktree(Parameters(DispatchCodexWorktreeRequest {
                project_root: Some(project_root.display().to_string()),
                sha: head_sha.clone(),
                briefing_content: "Do the required change and commit".to_string(),
                contract_fields: contract.clone(),
                keep_failed_worktree: Some(true),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(dispatched.0.status, "dispatched");
        assert!(!dispatched.0.worktree_path.is_empty());

        let mut final_status = None;
        for _ in 0..140 {
            let status = bridge
                .get_task_status(Parameters(GetTaskStatusRequest {
                    task_id: dispatched.0.task_id.clone(),
                }))
                .await
                .map_err(anyhow::Error::msg)?;
            if matches!(
                status.0.status,
                shared_types::TaskStatus::Completed
                    | shared_types::TaskStatus::Failed
                    | shared_types::TaskStatus::Timeout
                    | shared_types::TaskStatus::SetupFailed
                    | shared_types::TaskStatus::Cancelled
            ) {
                final_status = Some(status.0);
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        let status = final_status.expect("task should finish");
        assert!(
            matches!(status.status, shared_types::TaskStatus::Completed),
            "expected completed status, got {:?}, error={:?}",
            status.status,
            status.error_message
        );
        assert!(status.commit_sha.unwrap_or_default().len() >= 7);

        let out = bridge
            .get_task_output(Parameters(GetTaskOutputRequest {
                task_id: dispatched.0.task_id.clone(),
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(!out.0.commit_sha.is_empty());
        assert!(out.0.modified_files.iter().any(|p| p == "src/phase1.rs"));

        let review = bridge
            .query_gemini_review(Parameters(QueryGeminiReviewRequest {
                diff: "diff --git a/src/phase1.rs b/src/phase1.rs".to_string(),
                mode: shared_types::GeminiReviewMode::Pass,
                briefing: None,
                contract: None,
                failure_details: None,
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(matches!(
            review.0.verdict,
            GeminiReviewVerdict::Clean
                | GeminiReviewVerdict::Concerns
                | GeminiReviewVerdict::Regression
        ));

        let codex_sleep_script = write_codex_sleep_script(30)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", codex_sleep_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }

        let long_task = bridge
            .dispatch_codex(Parameters(DispatchCodexRequest {
                prompt: "long running task".to_string(),
                cwd: Some(project_root.display().to_string()),
                timeout_sec: Some(120),
                sandbox: None,
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        sleep(Duration::from_millis(150)).await;
        let cancelled = bridge
            .cancel_task(Parameters(AbeCancelTaskRequest {
                task_id: long_task.0.task_id,
            }))
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(cancelled.0.status, "cancelled");

        std::env::set_current_dir(&original_cwd)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        let _ = fs::remove_file(codex_commit_script);
        let _ = fs::remove_file(codex_sleep_script);
        let _ = fs::remove_file(gemini_script);
        let _ = fs::remove_dir_all(project_root);
        Ok(())
    }

    #[tokio::test]
    async fn abe_red_team_enforcement_blocks_non_compliant_worker() -> anyhow::Result<()> {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_cwd = std::env::current_dir()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let project_root = std::env::temp_dir().join(format!("triumvirate-abe-red-team-{now}"));
        fs::create_dir_all(&project_root)?;

        std::process::Command::new("git")
            .arg("init")
            .arg(&project_root)
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "user.email", "red-team@test.local"])
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "user.name", "Red Team Test"])
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "extensions.worktreeConfig", "true"])
            .status()?;
        fs::write(project_root.join("README.md"), "abe red team\n")?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["add", "README.md"])
            .status()?;
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["commit", "-m", "init"])
            .status()?;

        let head_sha = String::from_utf8(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&project_root)
                .args(["rev-parse", "HEAD"])
                .output()?
                .stdout,
        )?
        .trim()
        .to_string();
        std::env::set_current_dir(&project_root)?;
        let bridge = McpBridge::new_ephemeral();

        let mk_contract = |task_id: &str| shared_types::ContractFields {
            task_id: task_id.to_string(),
            req_ids: vec!["REQ-A2.3".to_string()],
            wave: 4,
            file_policy: shared_types::FilePolicy::DefaultDeny,
            allowed_files: vec!["src/allowed.rs".to_string()],
            forbidden_files: vec!["src/forbidden.rs".to_string()],
            allowed_commands: vec![vec!["true".to_string()]],
            forbidden_commands: vec![vec!["rm".to_string(), "-rf".to_string()]],
            commit_format: format!("^{task_id}:"),
            test_command: "true".to_string(),
            task_timeout_sec: 1,
            done_when: "red team rejection observed".to_string(),
            reality_test: "enforcement stack blocks violations".to_string(),
            sandbox_permissions: None,
        };

        let dispatch_and_expect_failed = |script_path: PathBuf, task_id: String| {
            let bridge = bridge.clone();
            let head_sha = head_sha.clone();
            let project_root = project_root.clone();
            async move {
            // SAFETY: test controls env var lifecycle under lock.
            unsafe {
                std::env::set_var("TRIUMVIRATE_CODEX_BIN", script_path.as_os_str());
                std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
            }
            let dispatched = bridge
                .dispatch_codex_worktree(Parameters(DispatchCodexWorktreeRequest {
                    project_root: Some(project_root.display().to_string()),
                    sha: head_sha.clone(),
                    briefing_content: "Non-compliant attempt for red-team validation".to_string(),
                    contract_fields: mk_contract(&task_id),
                    keep_failed_worktree: Some(true),
                }))
                .await
                .map_err(anyhow::Error::msg)?;
            for _ in 0..120 {
                let status = bridge
                    .get_task_status(Parameters(GetTaskStatusRequest {
                        task_id: dispatched.0.task_id.clone(),
                    }))
                    .await
                    .map_err(anyhow::Error::msg)?;
                if matches!(
                    status.0.status,
                    shared_types::TaskStatus::Failed
                        | shared_types::TaskStatus::Timeout
                        | shared_types::TaskStatus::SetupFailed
                        | shared_types::TaskStatus::Cancelled
                        | shared_types::TaskStatus::Completed
                ) {
                    return Ok::<shared_types::TaskStatus, anyhow::Error>(status.0.status);
                }
                sleep(Duration::from_millis(100)).await;
            }
                anyhow::bail!("task did not reach terminal state in time");
            }
        };

        let forbidden_file_script = write_codex_custom_script(
            "mkdir -p src\n\
             echo 'pub fn bad() {}' > src/forbidden.rs\n\
             git add src/forbidden.rs\n\
             git commit -m 'T-016A: forbidden file write'",
        )?;
        let bad_commit_script = write_codex_custom_script(
            "mkdir -p src\n\
             echo 'pub fn ok() {}' > src/allowed.rs\n\
             git add src/allowed.rs\n\
             git commit -m 'wrong-format commit message'",
        )?;
        let stub_script = write_codex_custom_script(
            "mkdir -p src\n\
            echo '// pending: stub' > src/allowed.rs\n\
             git add src/allowed.rs\n\
             git commit -m 'T-016C: stub marker present'",
        )?;

        let s1 = dispatch_and_expect_failed(forbidden_file_script.clone(), "T-016A".to_string()).await?;
        assert!(!matches!(s1, shared_types::TaskStatus::Completed));
        let s2 = dispatch_and_expect_failed(bad_commit_script.clone(), "T-016B".to_string()).await?;
        assert!(!matches!(s2, shared_types::TaskStatus::Completed));
        let s3 = dispatch_and_expect_failed(stub_script.clone(), "T-016C".to_string()).await?;
        assert!(!matches!(s3, shared_types::TaskStatus::Completed));

        let command_hook = std::env::var("HOME")
            .map(PathBuf::from)?
            .join(".claude")
            .join("hooks")
            .join("enforce-command-scope.sh");
        let contract_path = project_root.join(".triumvirate").join("contract-red-team.json");
        fs::create_dir_all(contract_path.parent().unwrap_or(&project_root))?;
        fs::write(
            &contract_path,
            serde_json::to_string_pretty(&mk_contract("T-016D"))?,
        )?;
        let out = std::process::Command::new(command_hook)
            .current_dir(&project_root)
            .env("TRIUMVIRATE_CONTRACT_PATH", &contract_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(stdin) = child.stdin.as_mut() {
                    use std::io::Write as _;
                    let payload = r#"{"tool_input":{"command":"rm -rf /tmp/evil"}}"#;
                    stdin.write_all(payload.as_bytes())?;
                }
                child.wait_with_output()
            })?;
        assert_eq!(out.status.code(), Some(2));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("BLOCKED"));

        std::env::set_current_dir(&original_cwd)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }
        let _ = fs::remove_file(forbidden_file_script);
        let _ = fs::remove_file(bad_commit_script);
        let _ = fs::remove_file(stub_script);
        let _ = fs::remove_dir_all(project_root);
        Ok(())
    }
}

/// FEAT-012 (REQ-017) T-008 reality tests for the Pantheon v3.9.0 REST
/// surface (`/api/workers`, `/api/fleet`, `/api/fleet/{build_id}`).
///
/// These tests drive the real Axum router via `tower::ServiceExt::oneshot`
/// — no mocks, no hardcoded JSON stubs. They assert:
///   - auth gating on every route (missing/wrong bearer → 401)
///   - end-to-end aggregation from `state.abe_tasks` and
///     `state.fleet_v2_states` into the frozen `shared_types::api` shapes
///   - empty-array-not-null serialization for the Tauri client
///   - path-parameter routing via axum 0.8 `{build_id}` syntax
#[cfg(test)]
mod pantheon_rest_tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header::AUTHORIZATION},
        routing::get,
    };
    use daemon_core::metrics::DaemonMetrics;
    use shared_types::{FleetBuild, FleetResponse, FleetTask, WorkersResponse};
    use std::{
        collections::{HashMap, VecDeque},
        sync::Arc,
    };
    use tokio::{process::Command, sync::Mutex};
    use tower::ServiceExt;

    // Re-use the production handlers directly — no stubs, no duplicates.
    // If these symbols ever move or rename, this test module refuses to
    // compile, which is the whole point of binding the tests to the real
    // entry points rather than shadowing them.
    use super::{api_fleet, api_fleet_by_id, api_workers, DaemonRuntimeState};

    /// Shared bearer token for the pantheon REST test module. Every test
    /// builds a fresh `DaemonState` with this value, so collisions are
    /// impossible even under parallel test execution — state is not shared
    /// across tests. Cherry-picked from T-008 Athena per Gemini judge.
    const TEST_TOKEN: &str = "pantheon-rest-test-token";

    /// Build an authenticated GET request. Cherry-picked from T-008 Athena
    /// per Gemini judge to eliminate the repeated `Request::builder()...`
    /// chain across all nine tests.
    fn get_with_bearer(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request build")
    }

    /// Build a GET request with NO Authorization header, for the two
    /// missing-bearer auth-rejection tests.
    fn get_no_bearer(uri: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("request build")
    }

    fn make_state(token: &str) -> DaemonRuntimeState {
        let metrics = Arc::new(DaemonMetrics::new().expect("metrics"));
        let (ws_events, _rx) = broadcast::channel::<String>(64);
        let abe_tasks = abe::task_tracker::TaskTracker::with_observability(
            metrics.clone(),
            Some(ws_events.clone()),
        );
        DaemonState::new(
            token.to_string(),
            Arc::new(Mutex::new(HashMap::new())),
            "127.0.0.1:0".to_string(),
            Arc::new(Mutex::new(HashMap::new())),
            None,
            abe_tasks,
            Arc::new(Mutex::new(VecDeque::new())),
            Arc::new(Mutex::new(VecDeque::new())),
            metrics,
            ws_events,
        )
    }

    fn make_router(state: DaemonRuntimeState) -> Router {
        Router::new()
            .route("/api/workers", get(api_workers))
            .route("/api/fleet", get(api_fleet))
            .route("/api/fleet/{build_id}", get(api_fleet_by_id))
            .with_state(state)
    }

    async fn register_worker(
        state: &DaemonRuntimeState,
        task_id: &str,
        parent: Option<&str>,
        root: Option<&str>,
    ) {
        // A real subprocess is required because TaskTracker::register
        // takes Arc<Mutex<Child>>. `sh -c true` exits immediately but the
        // record persists in the tracker until explicitly transitioned.
        let child = Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .expect("spawn child");
        state
            .abe_tasks
            .register(
                task_id.to_string(),
                1,
                Arc::new(Mutex::new(child)),
                None,
                parent.map(ToString::to_string),
                root.map(ToString::to_string),
                None,
                None,
                std::time::Instant::now(),
            )
            .await;
    }

    async fn body_to_string(body: Body) -> String {
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .expect("read body bytes");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }

    fn sample_build(build_id: &str) -> FleetBuild {
        FleetBuild {
            build_id: build_id.to_string(),
            task_count: 2,
            completed: 1,
            failed: 0,
            in_progress: 1,
            queued: 0,
            tasks: vec![
                FleetTask {
                    task_id: "T-001".into(),
                    status: "committed".into(),
                    files: vec![],
                    worker_session_id: None,
                    elapsed_ms: 0,
                    commit_sha: None,
                },
                FleetTask {
                    task_id: "T-002".into(),
                    status: "working".into(),
                    files: vec![],
                    worker_session_id: None,
                    elapsed_ms: 0,
                    commit_sha: None,
                },
            ],
        }
    }

    // ----- /api/workers ------------------------------------------------

    #[tokio::test]
    async fn api_workers_returns_abe_workers_with_lineage() {
        let state = make_state(TEST_TOKEN);
        register_worker(&state, "T-APOLLO-A", Some("pantheon-A"), Some("root-A")).await;
        register_worker(&state, "T-APOLLO-B", Some("pantheon-B"), Some("root-B")).await;

        let router = make_router(state);
        let resp = router
            .oneshot(get_with_bearer("/api/workers", TEST_TOKEN))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_to_string(resp.into_body()).await;
        let parsed: WorkersResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.workers.len(), 2);

        let parents: std::collections::HashSet<String> = parsed
            .workers
            .iter()
            .filter_map(|w| w.parent_session_id.clone())
            .collect();
        let expected: std::collections::HashSet<String> =
            ["pantheon-A".to_string(), "pantheon-B".to_string()]
                .into_iter()
                .collect();
        assert_eq!(parents, expected, "lineage must flow through snapshot");

        let task_ids: std::collections::HashSet<String> = parsed
            .workers
            .iter()
            .filter_map(|w| w.task_id.clone())
            .collect();
        assert!(task_ids.contains("T-APOLLO-A"));
        assert!(task_ids.contains("T-APOLLO-B"));
    }

    #[tokio::test]
    async fn api_workers_empty_tracker_returns_empty_array_not_null() {
        let state = make_state(TEST_TOKEN);
        let router = make_router(state);
        let resp = router
            .oneshot(get_with_bearer("/api/workers", TEST_TOKEN))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_to_string(resp.into_body()).await;

        // Tauri client reads `workers` as Vec<WorkerInfo> — `null` would
        // deserialize as an error on the client, so this assertion is
        // load-bearing for the Pantheon reconnect flow.
        assert!(
            body.contains("\"workers\":[]"),
            "expected empty array, got body: {body}"
        );
        assert!(
            !body.contains("\"workers\":null"),
            "workers must never serialize as null"
        );

        let parsed: WorkersResponse = serde_json::from_str(&body).unwrap();
        assert!(parsed.workers.is_empty());
    }

    #[tokio::test]
    async fn api_workers_rejects_missing_bearer() {
        let state = make_state(TEST_TOKEN);
        let router = make_router(state);
        let resp = router
            .oneshot(get_no_bearer("/api/workers"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn api_workers_rejects_wrong_bearer() {
        let state = make_state(TEST_TOKEN);
        let router = make_router(state);
        let resp = router
            .oneshot(get_with_bearer("/api/workers", "wrong-token"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ----- /api/fleet --------------------------------------------------

    #[tokio::test]
    async fn api_fleet_returns_v2_builds_from_state() {
        let state = make_state(TEST_TOKEN);
        {
            let mut guard = state.fleet_v2_states.lock().await;
            guard.insert("build-001".to_string(), sample_build("build-001"));
        }
        let router = make_router(state);
        let resp = router
            .oneshot(get_with_bearer("/api/fleet", TEST_TOKEN))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_to_string(resp.into_body()).await;
        let parsed: FleetResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.builds.len(), 1);
        assert_eq!(parsed.builds[0].build_id, "build-001");
        assert_eq!(parsed.builds[0].tasks.len(), 2);
    }

    #[tokio::test]
    async fn api_fleet_empty_returns_empty_builds_array() {
        let state = make_state(TEST_TOKEN);
        let router = make_router(state);
        let resp = router
            .oneshot(get_with_bearer("/api/fleet", TEST_TOKEN))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_to_string(resp.into_body()).await;
        assert!(
            body.contains("\"builds\":[]"),
            "expected empty builds array, got: {body}"
        );
        assert!(!body.contains("\"builds\":null"));
        let parsed: FleetResponse = serde_json::from_str(&body).unwrap();
        assert!(parsed.builds.is_empty());
    }

    #[tokio::test]
    async fn api_fleet_by_id_returns_existing_build() {
        let state = make_state(TEST_TOKEN);
        {
            let mut guard = state.fleet_v2_states.lock().await;
            guard.insert("build-002".to_string(), sample_build("build-002"));
        }
        let router = make_router(state);
        let resp = router
            .oneshot(get_with_bearer("/api/fleet/build-002", TEST_TOKEN))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_to_string(resp.into_body()).await;
        let parsed: FleetBuild = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.build_id, "build-002");
        assert_eq!(parsed.tasks.len(), 2);
    }

    #[tokio::test]
    async fn api_fleet_by_id_returns_404_for_missing_build() {
        let state = make_state(TEST_TOKEN);
        let router = make_router(state);
        let resp = router
            .oneshot(get_with_bearer("/api/fleet/nonexistent", TEST_TOKEN))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn api_fleet_rejects_missing_bearer() {
        let state = make_state(TEST_TOKEN);
        let router = make_router(state);
        let resp = router
            .oneshot(get_no_bearer("/api/fleet"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

// ---------------------------------------------------------------------------
// T-009 (REQ-020, FEAT-013) reality tests — Pantheon v3.9.0 state snapshot
// + replay-aware WebSocket handshake.
//
// Every test in this module stands up a real Axum server bound to a random
// port (`TcpListener::bind("127.0.0.1:0")`) and talks to it with a real
// tokio_tungstenite WebSocket client + reqwest HTTP client. No in-process
// fakes, no mocked handlers — these exercise the exact same code path
// production `run_daemon` uses, wired with the real module-scope `api_state`
// / `ws_v2` handlers and the real `daemon_http::ws_route` for the legacy
// regression check.
//
// Bake-off credit: Apollo wrote the architecture (module-scope handlers
// imported directly into the test module via `super::api_state` /
// `super::ws_v2`). Athena's `read_text` + `ws_request_with_bearer` helpers
// are cherry-picked in from her submission per Gemini's judge verdict —
// they handle WebSocket ping/pong/close frames gracefully in tests where
// Apollo's raw `stream.next().await.expect(...).expect(...)` pattern would
// be brittle under heartbeats.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod pantheon_ws_replay_tests {
    use super::*;
    use axum::Router;
    use axum::routing::get;
    use daemon_core::{DaemonState, encode_ws_event};
    use futures_util::{SinkExt as _, StreamExt as _};
    use shared_types::{
        AgentStreamEvent, FleetBuild, ReplayResponse, SessionState, StateResponse,
    };
    use std::collections::{HashMap, VecDeque};
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, broadcast};
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::handshake::client::Request as WsRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    /// Helper: build a real DaemonRuntimeState wired to a fresh broadcast
    /// channel + a temp TokenDb for the legacy /ws route's DaemonHttpState.
    /// Returns the state plus a tempdir guard (keep it alive for the
    /// lifetime of the test — dropping it removes the sqlite file).
    async fn make_test_state(
        token: &str,
    ) -> (
        DaemonState<abe::task_tracker::TaskTracker>,
        daemon_http::DaemonHttpState,
        tempfile::TempDir,
    ) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let metrics = Arc::new(DaemonMetrics::new().expect("metrics"));
        let ws_events = broadcast::channel::<String>(256).0;
        let abe_tasks = abe::task_tracker::TaskTracker::with_observability(
            metrics.clone(),
            Some(ws_events.clone()),
        );
        let sessions: HashMap<String, SessionState> = HashMap::new();
        let state = DaemonState::new(
            token.to_string(),
            Arc::new(Mutex::new(HashMap::new())),
            "127.0.0.1:0".to_string(),
            Arc::new(Mutex::new(sessions)),
            None,
            abe_tasks,
            Arc::new(Mutex::new(VecDeque::new())),
            Arc::new(Mutex::new(VecDeque::new())),
            metrics.clone(),
            ws_events.clone(),
        );
        let token_db_path = tmp.path().join("tokens.db");
        let token_db = daemon_http::open_token_db(&token_db_path).expect("token db");
        let http_state = daemon_http::DaemonHttpState {
            token: state.token.clone(),
            queues: state.queues.clone(),
            ledger_project_lru: state.ledger_project_lru.clone(),
            marker_parse_window: state.marker_parse_window.clone(),
            metrics: state.metrics.clone(),
            ws_events: state.ws_events.clone(),
            token_db,
            ask_agent_executor: Arc::new(|_req| {
                Box::pin(async {
                    Err::<shared_types::AskAgentResponse, String>(
                        "ask_agent not used in ws replay tests".to_string(),
                    )
                })
            }),
        };
        (state, http_state, tmp)
    }

    /// Helper: construct the subset of the production Router that matters
    /// for T-009 testing. Mirrors how `run_daemon` wires the T-009 handlers
    /// plus the legacy `/ws` route for the backwards-compat test. The key
    /// architectural property: both /api/state and /ws/v2 bind to the REAL
    /// module-scope production handlers via `super::api_state` /
    /// `super::ws_v2` — no test copies, no shadow definitions.
    fn make_test_router(
        state: DaemonState<abe::task_tracker::TaskTracker>,
        http_state: daemon_http::DaemonHttpState,
    ) -> Router {
        Router::new()
            .route("/api/state", get(super::api_state))
            .route("/ws/v2", get(super::ws_v2))
            .route(
                "/ws",
                axum::routing::get_service(
                    daemon_http::ws_route.with_state(http_state),
                ),
            )
            .with_state(state)
    }

    /// Helper: start an ephemeral Axum server on a random 127.0.0.1 port
    /// and return the bound SocketAddr plus the tempdir guard and the
    /// server task handle. The caller must call `handle.abort()` at the
    /// end of the test so the runtime can shut down cleanly.
    async fn start_ephemeral_server(
        token: &str,
    ) -> (
        SocketAddr,
        DaemonState<abe::task_tracker::TaskTracker>,
        tempfile::TempDir,
        tokio::task::JoinHandle<()>,
    ) {
        let (state, http_state, tmp) = make_test_state(token).await;
        let state_for_router = state.clone();
        let app = make_test_router(state_for_router, http_state);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (addr, state, tmp, handle)
    }

    /// Helper: build a ToolCall event at the given seq.
    fn make_event(seq: u64) -> AgentStreamEvent {
        AgentStreamEvent::ToolCall {
            agent: "codex".to_string(),
            tool_name: "bash".to_string(),
            args_summary: format!("echo {seq}"),
            seq,
        }
    }

    /// Cherry-picked from T-009 Athena per Gemini judge: build a
    /// tungstenite client request carrying an Authorization header.
    /// Cleaner than inline `.into_client_request()?` + `.headers_mut()`
    /// chains at every call site.
    fn ws_request_with_bearer(url: &str, token: &str) -> WsRequest {
        let mut req = url.into_client_request().expect("ws request");
        req.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).expect("header value"),
        );
        req
    }

    /// Cherry-picked from T-009 Athena per Gemini judge: build a
    /// tungstenite client request with NO Authorization header. For the
    /// missing-bearer rejection test.
    fn ws_request_plain(url: &str) -> WsRequest {
        url.into_client_request().expect("ws request")
    }

    /// Cherry-picked from T-009 Athena per Gemini judge: read the next
    /// TEXT frame from the WebSocket, transparently swallowing any
    /// Ping/Pong/Binary/Frame control frames in between. Returns None on
    /// Close or error. Apollo's raw `stream.next().await.expect(...)` would
    /// be brittle if tungstenite ever interleaves a heartbeat; this helper
    /// is the correct way to read text payloads from a live socket.
    async fn read_text(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Option<String> {
        while let Some(msg) = ws.next().await {
            match msg {
                Ok(WsMessage::Text(text)) => return Some(text.to_string()),
                Ok(WsMessage::Ping(_)) | Ok(WsMessage::Pong(_)) => continue,
                Ok(WsMessage::Close(_)) => return None,
                Ok(WsMessage::Binary(_)) => continue,
                Ok(WsMessage::Frame(_)) => continue,
                Err(_) => return None,
            }
        }
        None
    }

    /// Helper: open a /ws/v2 WebSocket with the given bearer header (or
    /// no header, for the 401 rejection test).
    async fn connect_ws_v2(
        addr: SocketAddr,
        bearer: Option<&str>,
    ) -> tokio_tungstenite::tungstenite::Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    > {
        let url = format!("ws://{addr}/ws/v2");
        let req = match bearer {
            Some(b) => ws_request_with_bearer(&url, b),
            None => ws_request_plain(&url),
        };
        let (stream, _resp) = tokio_tungstenite::connect_async(req).await?;
        Ok(stream)
    }

    // Test 1 — GET /api/state returns a full StateResponse.
    #[tokio::test]
    async fn api_state_returns_full_snapshot_with_version_and_uptime() {
        let token = "test-token-t009-1";
        let (addr, state, _tmp, handle) = start_ephemeral_server(token).await;

        let child_a = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .expect("spawn a");
        state
            .abe_tasks
            .register(
                "T-STATE-A".to_string(),
                1,
                Arc::new(Mutex::new(child_a)),
                None,
                Some("pantheon-parent".to_string()),
                Some("pantheon-root".to_string()),
                None,
                None,
                std::time::Instant::now(),
            )
            .await;
        let child_b = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .expect("spawn b");
        state
            .abe_tasks
            .register(
                "T-STATE-B".to_string(),
                1,
                Arc::new(Mutex::new(child_b)),
                None,
                None,
                None,
                None,
                None,
                std::time::Instant::now(),
            )
            .await;

        {
            let mut guard = state.fleet_v2_states.lock().await;
            guard.insert(
                "build-t009-1".to_string(),
                FleetBuild {
                    build_id: "build-t009-1".to_string(),
                    task_count: 3,
                    completed: 1,
                    failed: 0,
                    in_progress: 1,
                    queued: 1,
                    tasks: vec![],
                },
            );
        }

        tokio::time::sleep(Duration::from_millis(5)).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{addr}/api/state"))
            .bearer_auth(token)
            .send()
            .await
            .expect("send /api/state");
        assert_eq!(resp.status().as_u16(), 200);
        let body: StateResponse = resp.json().await.expect("parse StateResponse");

        assert_eq!(body.version, daemon_core::VERSION.to_string());
        assert!(body.uptime_ms > 0);
        assert_eq!(body.workers.len(), 2);
        assert_eq!(body.fleet.len(), 1);
        assert_eq!(body.fleet[0].build_id, "build-t009-1");
        let _seq: u64 = body.last_event_seq;

        handle.abort();
        let _ = handle.await;
    }

    // Test 2 — GET /api/state with no Authorization header returns 401.
    #[tokio::test]
    async fn api_state_rejects_missing_bearer() {
        let token = "test-token-t009-2";
        let (addr, _state, _tmp, handle) = start_ephemeral_server(token).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{addr}/api/state"))
            .send()
            .await
            .expect("send /api/state");
        assert_eq!(resp.status().as_u16(), 401);

        handle.abort();
        let _ = handle.await;
    }

    // Test 3 — /ws/v2 replays events within range wrapped in the envelope.
    #[tokio::test]
    async fn ws_v2_replays_events_within_range_wrapped_in_envelope() {
        let token = "test-token-t009-3";
        let (addr, state, _tmp, handle) = start_ephemeral_server(token).await;

        for i in 1..=5u64 {
            state.replay_buffer.push(make_event(i));
        }

        let mut stream = connect_ws_v2(addr, Some(token)).await.expect("connect");
        stream
            .send(WsMessage::Text(
                serde_json::to_string(&shared_types::ReplayRequest {
                    action: "subscribe".to_string(),
                    last_seq: 0,
                })
                .unwrap()
                .into(),
            ))
            .await
            .expect("send subscribe");

        let ack_text = read_text(&mut stream).await.expect("ack frame");
        let ack: ReplayResponse = serde_json::from_str(&ack_text).expect("parse ack");
        assert_eq!(ack.replay, "ok");
        assert!(ack.oldest_seq.is_none());

        for expected_seq in 1..=5u64 {
            let text = read_text(&mut stream)
                .await
                .unwrap_or_else(|| panic!("expected frame for seq {expected_seq}"));
            let value: serde_json::Value =
                serde_json::from_str(&text).expect("parse envelope");
            assert_eq!(
                value.get("type").and_then(|v| v.as_str()),
                Some("agent_stream")
            );
            let payload = value.get("payload").expect("payload").clone();
            let event: AgentStreamEvent =
                serde_json::from_value(payload).expect("parse payload");
            assert_eq!(event.seq(), expected_seq);
        }

        let _ = stream.close(None).await;
        handle.abort();
        let _ = handle.await;
    }

    // Test 4 — /ws/v2 returns out_of_range when client is too far behind.
    #[tokio::test]
    async fn ws_v2_returns_out_of_range_when_client_too_far_behind() {
        let token = "test-token-t009-4";
        let (addr, state, _tmp, handle) = start_ephemeral_server(token).await;

        // 1500 events into a 1000-capacity buffer → oldest evicts to 501.
        for i in 1..=1500u64 {
            state.replay_buffer.push(make_event(i));
        }

        let mut stream = connect_ws_v2(addr, Some(token)).await.expect("connect");
        stream
            .send(WsMessage::Text(
                serde_json::to_string(&shared_types::ReplayRequest {
                    action: "subscribe".to_string(),
                    last_seq: 200,
                })
                .unwrap()
                .into(),
            ))
            .await
            .expect("send subscribe");

        let text = read_text(&mut stream).await.expect("out_of_range frame");
        let value: serde_json::Value =
            serde_json::from_str(&text).expect("parse out_of_range");
        assert!(value.get("type").is_none(), "must not be envelope-wrapped");
        assert!(value.get("payload").is_none());
        assert_eq!(
            value.get("replay").and_then(|v| v.as_str()),
            Some("out_of_range")
        );
        let resp: ReplayResponse =
            serde_json::from_str(&text).expect("ReplayResponse");
        assert_eq!(resp.replay, "out_of_range");
        assert_eq!(resp.oldest_seq, Some(501));

        // Socket should close after the out_of_range frame.
        assert!(read_text(&mut stream).await.is_none());

        handle.abort();
        let _ = handle.await;
    }

    // Test 5 — boundary replay.
    #[tokio::test]
    async fn ws_v2_at_boundary_replays_correctly_with_envelope() {
        let token = "test-token-t009-5";
        let (addr, state, _tmp, handle) = start_ephemeral_server(token).await;

        for i in 50..=60u64 {
            state.replay_buffer.push(make_event(i));
        }

        let mut stream = connect_ws_v2(addr, Some(token)).await.expect("connect");
        stream
            .send(WsMessage::Text(
                serde_json::to_string(&shared_types::ReplayRequest {
                    action: "subscribe".to_string(),
                    last_seq: 50,
                })
                .unwrap()
                .into(),
            ))
            .await
            .expect("send subscribe");

        let ack_text = read_text(&mut stream).await.expect("ack");
        let ack: ReplayResponse = serde_json::from_str(&ack_text).expect("parse ack");
        assert_eq!(ack.replay, "ok");

        for expected_seq in 51..=60u64 {
            let text = read_text(&mut stream).await.expect("frame");
            let value: serde_json::Value =
                serde_json::from_str(&text).expect("parse envelope");
            assert_eq!(
                value.get("type").and_then(|v| v.as_str()),
                Some("agent_stream")
            );
            let event: AgentStreamEvent =
                serde_json::from_value(value.get("payload").expect("payload").clone())
                    .expect("parse payload");
            assert_eq!(event.seq(), expected_seq);
            assert_ne!(event.seq(), 50);
        }

        let _ = stream.close(None).await;
        handle.abort();
        let _ = handle.await;
    }

    // Test 6 — live tail after historical replay preserves the envelope.
    #[tokio::test]
    async fn ws_v2_live_tail_after_historical_replay_preserves_envelope() {
        let token = "test-token-t009-6";
        let (addr, state, _tmp, handle) = start_ephemeral_server(token).await;

        for i in 1..=3u64 {
            state.replay_buffer.push(make_event(i));
        }

        let mut stream = connect_ws_v2(addr, Some(token)).await.expect("connect");
        stream
            .send(WsMessage::Text(
                serde_json::to_string(&shared_types::ReplayRequest {
                    action: "subscribe".to_string(),
                    last_seq: 0,
                })
                .unwrap()
                .into(),
            ))
            .await
            .expect("send subscribe");

        let _ = read_text(&mut stream).await.expect("ack");
        for _ in 0..3 {
            let text = read_text(&mut stream).await.expect("historical frame");
            let value: serde_json::Value = serde_json::from_str(&text).expect("json");
            assert_eq!(
                value.get("type").and_then(|v| v.as_str()),
                Some("agent_stream")
            );
        }

        let new_event = make_event(4);
        let new_envelope = encode_ws_event(
            "agent_stream",
            serde_json::to_value(&new_event).unwrap(),
        );
        let _ = state.ws_events.send(new_envelope);

        let text = tokio::time::timeout(Duration::from_millis(500), read_text(&mut stream))
            .await
            .expect("timeout waiting for live tail")
            .expect("live frame");
        let value: serde_json::Value = serde_json::from_str(&text).expect("parse live");
        assert_eq!(
            value.get("type").and_then(|v| v.as_str()),
            Some("agent_stream"),
            "live frame must use the SAME envelope as replay"
        );
        let event: AgentStreamEvent =
            serde_json::from_value(value.get("payload").expect("payload").clone())
                .expect("parse payload");
        assert_eq!(event.seq(), 4);

        let _ = stream.close(None).await;
        handle.abort();
        let _ = handle.await;
    }

    // Test 7 — dedup between historical replay and live tail.
    #[tokio::test]
    async fn ws_v2_dedups_overlap_between_historical_and_live() {
        let token = "test-token-t009-7";
        let (addr, state, _tmp, handle) = start_ephemeral_server(token).await;

        for i in 1..=5u64 {
            state.replay_buffer.push(make_event(i));
        }

        let mut stream = connect_ws_v2(addr, Some(token)).await.expect("connect");
        stream
            .send(WsMessage::Text(
                serde_json::to_string(&shared_types::ReplayRequest {
                    action: "subscribe".to_string(),
                    last_seq: 0,
                })
                .unwrap()
                .into(),
            ))
            .await
            .expect("send subscribe");

        let _ = read_text(&mut stream).await.expect("ack");
        for _ in 0..5 {
            let _ = read_text(&mut stream).await.expect("historical frame");
        }

        let duplicate = encode_ws_event(
            "agent_stream",
            serde_json::to_value(make_event(3)).unwrap(),
        );
        let _ = state.ws_events.send(duplicate);

        let timed = tokio::time::timeout(Duration::from_millis(200), read_text(&mut stream)).await;
        match timed {
            Err(_) => {} // timeout — dedup suppressed the frame
            Ok(Some(frame)) => panic!("expected no frame (dedup), got {frame:?}"),
            Ok(None) => panic!("stream ended unexpectedly during dedup test"),
        }

        let _ = stream.close(None).await;
        handle.abort();
        let _ = handle.await;
    }

    // Test 8 — /ws/v2 rejects missing bearer BEFORE the protocol switch.
    #[tokio::test]
    async fn ws_v2_rejects_missing_bearer_on_upgrade() {
        let token = "test-token-t009-8";
        let (addr, _state, _tmp, handle) = start_ephemeral_server(token).await;

        let result = connect_ws_v2(addr, None).await;
        assert!(
            result.is_err(),
            "expected connect_ws_v2 without bearer to fail with 401"
        );

        handle.abort();
        let _ = handle.await;
    }

    // Test 9 — legacy /ws route unchanged (backwards-compat for `triumvirate watch`).
    #[tokio::test]
    async fn legacy_ws_route_unchanged() {
        let token = "test-token-t009-9";
        let (addr, _state, _tmp, handle) = start_ephemeral_server(token).await;

        let url = format!("ws://{addr}/ws");
        let (mut stream, _resp) =
            tokio_tungstenite::connect_async(url).await.expect("connect legacy ws");

        let expected = [
            "agent_state",
            "fleet_progress",
            "ledger_health",
            "review_completed",
        ];
        for expected_type in expected {
            let text = read_text(&mut stream).await.expect("bootstrap frame");
            let value: serde_json::Value =
                serde_json::from_str(&text).expect("parse bootstrap");
            assert_eq!(
                value.get("type").and_then(|v| v.as_str()),
                Some(expected_type),
                "legacy /ws bootstrap out of order"
            );
        }

        let _ = stream.close(None).await;
        handle.abort();
        let _ = handle.await;
    }
}
