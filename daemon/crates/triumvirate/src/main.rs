use clap::{Parser, Subcommand};
use agent_worker::{
    WorkerAcquireMode, acquire_worker, dismiss_worker,
};
#[cfg(test)]
use agent_worker::{reset_worker_registry_for_tests, update_worker_session};
use daemon_core::{
    DaemonState,
    daemon_bind_addr as core_daemon_bind_addr,
    publish_ws_event,
    metrics::DaemonMetrics,
    triumvirate_home_dir as core_triumvirate_home_dir,
    ensure_daemon_token as core_ensure_daemon_token,
    sessions_file_path as core_sessions_file_path,
    load_json_file_if_exists as core_load_json_file_if_exists,
    persist_json_file_if_enabled as core_persist_json_file_if_enabled,
};
#[cfg(test)]
use daemon_core::unix_time_ms as core_unix_time_ms;
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
    codex_command, is_bearer_authorized, is_supported_agent_name,
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
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
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
#[cfg(test)]
use shared_types::GeminiReviewVerdict;
#[cfg(test)]
use shared_types::{DaemonStatusSnapshot, LifecycleEvent, OutboxEvent};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
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
mod abe;
mod cli_ops;
mod git_ops_impl;
mod tracing_setup;

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
    token_db: Arc<TokenDb>,
}

static PROCESS_METRICS: OnceLock<Arc<DaemonMetrics>> = OnceLock::new();
static PROCESS_TOKEN_DB: OnceLock<Arc<TokenDb>> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct TokenDb {
    db_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct TokenRecord {
    pub agent: String,
    pub session_id: String,
    pub timestamp: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub thinking_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
    pub latency_ms: Option<i64>,
    pub tool_calls: Option<i64>,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    pub rate_limit_pct: Option<f64>,
    pub context_window: Option<i64>,
    pub build_id: Option<String>,
    pub task_id: Option<String>,
    pub wave: Option<i64>,
}

pub(crate) fn process_metrics() -> Option<&'static Arc<DaemonMetrics>> {
    PROCESS_METRICS.get()
}

pub(crate) fn process_token_db() -> Option<&'static Arc<TokenDb>> {
    PROCESS_TOKEN_DB.get()
}

fn set_process_metrics(metrics: Arc<DaemonMetrics>) {
    let _ = PROCESS_METRICS.set(metrics);
}

fn init_process_token_db() {
    if PROCESS_TOKEN_DB.get().is_some() {
        return;
    }

    let home = match core_triumvirate_home_dir() {
        Ok(home) => home,
        Err(err) => {
            warn!("failed to resolve triumvirate home for token DB: {err}");
            return;
        }
    };
    let db_path = home.join("token-economics.db");
    if let Some(parent) = db_path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        warn!(
            "failed to initialize token economics DB directory at {}: {err}",
            parent.display()
        );
        return;
    }
    let _ = PROCESS_TOKEN_DB.set(Arc::new(TokenDb { db_path }));
}

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_opt_text(value: Option<&str>) -> String {
    value.map(sql_quote).unwrap_or_else(|| "NULL".to_string())
}

fn sql_opt_i64(value: Option<i64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

fn sql_opt_f64(value: Option<f64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

pub(crate) fn record_daemon_tokens(db: &TokenDb, record: &TokenRecord) -> Result<(), String> {
    if record.agent.trim().is_empty() {
        return Err("token record agent must be non-empty".to_string());
    }
    if record.session_id.trim().is_empty() {
        return Err("token record session_id must be non-empty".to_string());
    }

    let sql = format!(
        "CREATE TABLE IF NOT EXISTS token_records (
            id INTEGER PRIMARY KEY,
            agent TEXT NOT NULL,
            session_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            model TEXT,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            cached_tokens INTEGER DEFAULT 0,
            thinking_tokens INTEGER DEFAULT 0,
            total_tokens INTEGER NOT NULL,
            cost_usd REAL,
            latency_ms INTEGER,
            tool_calls INTEGER,
            lines_added INTEGER,
            lines_removed INTEGER,
            rate_limit_pct REAL,
            context_window INTEGER,
            build_id TEXT,
            task_id TEXT,
            wave INTEGER
        );
        INSERT INTO token_records (
            agent, session_id, timestamp, model, input_tokens, output_tokens, cached_tokens,
            thinking_tokens, total_tokens, cost_usd, latency_ms, tool_calls, lines_added,
            lines_removed, rate_limit_pct, context_window, build_id, task_id, wave
        ) VALUES (
            {agent}, {session_id}, {timestamp}, {model}, {input_tokens}, {output_tokens},
            {cached_tokens}, {thinking_tokens}, {total_tokens}, {cost_usd}, {latency_ms},
            {tool_calls}, {lines_added}, {lines_removed}, {rate_limit_pct}, {context_window},
            {build_id}, {task_id}, {wave}
        );",
        agent = sql_quote(&record.agent),
        session_id = sql_quote(&record.session_id),
        timestamp = sql_quote(&record.timestamp),
        model = sql_opt_text(record.model.as_deref()),
        input_tokens = record.input_tokens,
        output_tokens = record.output_tokens,
        cached_tokens = record.cached_tokens,
        thinking_tokens = record.thinking_tokens,
        total_tokens = record.total_tokens,
        cost_usd = sql_opt_f64(record.cost_usd),
        latency_ms = sql_opt_i64(record.latency_ms),
        tool_calls = sql_opt_i64(record.tool_calls),
        lines_added = sql_opt_i64(record.lines_added),
        lines_removed = sql_opt_i64(record.lines_removed),
        rate_limit_pct = sql_opt_f64(record.rate_limit_pct),
        context_window = sql_opt_i64(record.context_window),
        build_id = sql_opt_text(record.build_id.as_deref()),
        task_id = sql_opt_text(record.task_id.as_deref()),
        wave = sql_opt_i64(record.wave),
    );

    let output = Command::new("sqlite3")
        .arg(&db.db_path)
        .arg(sql)
        .output()
        .map_err(|err| format!("failed to execute sqlite3 for token record insert: {err}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        "sqlite3 token record insert failed".to_string()
    } else {
        format!("sqlite3 token record insert failed: {stderr}")
    })
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
        init_process_token_db();
        let token_db = process_token_db()
            .expect("token DB should be initialized")
            .clone();
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
    ) -> mcp_tools::abe::BoxFuture<()> {
        let tracker = self.clone();
        Box::pin(async move { tracker.register(task_id, wave, child, worktree_path).await })
    }

    fn mark_completed(
        &self,
        task_id: String,
        commit_sha: String,
        modified_files: Vec<String>,
        stdout: String,
        validation_log: Option<String>,
        test_output: Option<String>,
    ) -> mcp_tools::abe::BoxFuture<()> {
        let tracker = self.clone();
        Box::pin(async move {
            let _ = tracker
                .mark_completed(
                    &task_id,
                    commit_sha,
                    modified_files,
                    stdout,
                    validation_log,
                    test_output,
                )
                .await;
        })
    }

    fn mark_failed(
        &self,
        task_id: String,
        exit_code: Option<i32>,
        error_message: String,
    ) -> mcp_tools::abe::BoxFuture<()> {
        let tracker = self.clone();
        Box::pin(async move {
            let _ = tracker.mark_failed(&task_id, exit_code, error_message).await;
        })
    }

    fn mark_timeout(&self, task_id: String) -> mcp_tools::abe::BoxFuture<()> {
        let tracker = self.clone();
        Box::pin(async move {
            let _ = tracker.mark_timeout(&task_id).await;
        })
    }

    fn mark_stuck(&self, task_id: String, error_message: String) -> mcp_tools::abe::BoxFuture<()> {
        let tracker = self.clone();
        Box::pin(async move {
            let _ = tracker.mark_stuck(&task_id, error_message).await;
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

    #[tool(description = "Send a task to a specific agent (Increment 1b supports gemini mock path).")]
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
        mcp_token_tools::get_token_summary(self.token_db.db_path.as_path(), req).map(Json)
    }

    #[tool(description = "Return per-task token and cost breakdown for a build_id.")]
    async fn get_build_cost(
        &self,
        Parameters(req): Parameters<mcp_token_tools::GetBuildCostRequest>,
    ) -> Result<Json<mcp_token_tools::GetBuildCostResponse>, String> {
        mcp_token_tools::get_build_cost(self.token_db.db_path.as_path(), req).map(Json)
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

    #[tool(description = "Query Gemini synchronously and return response text.")]
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

    #[tool(description = "Query Gemini for code review verdicts on pass/failure contexts.")]
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
    init_process_token_db();
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

#[tool_handler]
impl ServerHandler for McpBridge {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(format!(
                "Triumvirate MCP bridge v{}. Use `ping` to verify connectivity.",
                daemon_core::VERSION
            ))
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
    type DaemonRuntimeState = DaemonState<abe::task_tracker::TaskTracker>;

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
        Ok(AxumJson(serde_json::json!({
            "status": "ok",
            "service": "triumvirate-daemon-v2",
            "mode": "incremental-dev",
            "daemon_bind_addr": state.bind_addr,
            "version": daemon_core::VERSION
        })))
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
            "supported_agents": ["gemini", "codex"],
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
                }
                Err(err) => {
                    tracing::warn!(
                        "ledger background sweep join failure for {}: {err}",
                        project_root.display()
                    );
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
        State(state): State<DaemonRuntimeState>,
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
    let metrics = Arc::new(DaemonMetrics::new()?);
    let ws_events = broadcast::channel(256).0;
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
    init_process_token_db();
    let token_db_path = core_triumvirate_home_dir()?.join("token-economics.db");
    let token_db = daemon_http::open_token_db(&token_db_path)?;
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
        .route("/{*path}", get(daemon_http::dashboard_spa_fallback_route))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state.clone(), metrics_middleware));
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(%bind_addr, "daemon listener bound");
    tokio::spawn(async {
        prewarm_daemon_workers().await;
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
    async fn ask_agent_gemini_injects_tool_marker_instructions() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
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
    async fn ask_agent_codex_adds_full_auto_only_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
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
            std::env::remove_var("TRIUMVIRATE_REQUIRE_PEER_REVIEW");
        }

        let req = AskAgentRequest {
            agent: "codex".to_string(),
            message: "capture args".to_string(),
            cwd: Some(test_home.display().to_string()),
            repo: Some("triumvirate".to_string()),
            branch: Some("feat/mcp-first".to_string()),
        };
        let _ = execute_ask_agent(&req, None).await.map_err(anyhow::Error::msg)?;
        let captured = fs::read_to_string(&args_file)?;
        assert!(captured.lines().any(|line| line == "--full-auto"));

        // SAFETY: test controls env var lifecycle under lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_CODEX_AUTO_APPROVE") };
        let _ = execute_ask_agent(&req, None).await.map_err(anyhow::Error::msg)?;
        let captured_without = fs::read_to_string(&args_file)?;
        assert!(!captured_without.lines().any(|line| line == "--full-auto"));

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
    async fn ask_agent_codex_auto_approve_writes_ledger_record() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
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
    async fn ask_agent_requires_peer_review_when_env_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
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
    async fn mcp_ask_agent_returns_daemon_recovery_error_when_unreachable() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
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

    #[tokio::test]
    async fn ask_agent_writes_outbox_events() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
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
        let _guard = env_lock().lock().expect("env lock poisoned");
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
        let _guard = env_lock().lock().expect("env lock poisoned");
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
        let _guard = env_lock().lock().expect("env lock poisoned");
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
        let _guard = env_lock().lock().expect("env lock poisoned");
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
        let _guard = env_lock().lock().expect("env lock poisoned");
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
        let _guard = env_lock().lock().expect("env lock poisoned");
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
            task_timeout_sec: 1,
            done_when: "phase 1 e2e verified".to_string(),
            reality_test: "dispatch->status->output->review->cancel".to_string(),
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
        let _guard = env_lock().lock().expect("env lock poisoned");
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

        let command_hook = PathBuf::from(
            std::env::var("HOME")
                .map(PathBuf::from)?
                .join(".claude")
                .join("hooks")
                .join("enforce-command-scope.sh"),
        );
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
