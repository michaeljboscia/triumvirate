use clap::{Parser, Subcommand};
use daemon_core::{
    QueueRegistry, acknowledge_dead_drop_ticket,
    acquire_project_queue as core_acquire_project_queue,
    append_memory_entry as core_append_memory_entry,
    append_outbox_event as core_append_outbox_event, count_dead_drop_tickets,
    create_dead_drop_ticket, gc_dead_drop_tickets, list_dead_drop_tickets,
    list_scratchpad as core_list_scratchpad, project_queue_key as core_project_queue_key,
    read_memory_entries as core_read_memory_entries, read_outbox_events as core_read_outbox_events,
    resolve_context as core_resolve_context, write_scratchpad as core_write_scratchpad, ensure_daemon_token as core_ensure_daemon_token,
    sessions_file_path as core_sessions_file_path, load_json_file as core_load_json_file,
    persist_json_file as core_persist_json_file,
};
use mcp_bridge::{build_role_adapted_prompts, is_supported_agent, is_supported_agent_name};
use axum::{
    Json as AxumJson, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::{get, post},
};
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use shared_types::{
    AgentResult, AskAgentRequest, AskAgentResponse, AskTwinsRequest, AskTwinsResponse,
    FallbackAckRequest, FallbackGcRequest, FallbackGcResponse, FallbackListRequest,
    FallbackListResponse, LifecycleEvent, MemoryEntry, MemoryReadRequest, MemoryReadResponse,
    MemoryWriteRequest, MemoryWriteResponse, OutboxEvent, OutboxRecentRequest,
    OutboxRecentResponse, ScratchpadListRequest, ScratchpadListResponse, ScratchpadWriteRequest,
    ScratchpadWriteResponse, StatusResponse, DaemonHealthResponse, DaemonStatusSnapshot,
};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::Mutex,
    time::{Duration, sleep, timeout},
};
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

impl McpBridge {
    fn new() -> Self {
        Self::with_persistence(true)
    }

    fn new_ephemeral() -> Self {
        Self::with_persistence(false)
    }

    fn with_persistence(enable_persistence: bool) -> Self {
        let sessions_file = if enable_persistence {
            sessions_file_path().ok()
        } else {
            None
        };
        // Load persisted sessions on startup so sessions survive MCP bridge restarts.
        let sessions = sessions_file
            .as_ref()
            .and_then(|path| load_sessions(path).ok())
            .unwrap_or_default();
        Self {
            tool_router: Self::tool_router(),
            sessions: Arc::new(Mutex::new(sessions)),
            sessions_file,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SessionState {
    agent: String,
    history: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
struct SpawnSessionRequest {
    agent: String,
    name: String,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct SessionInfo {
    name: String,
    agent: String,
    turns: usize,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct SessionListResponse {
    sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
struct AskSessionRequest {
    name: String,
    message: String,
}

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
struct DismissSessionRequest {
    name: String,
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
    ) -> Result<Json<AskAgentResponse>, String> {
        if use_daemon_for_mcp() {
            return fetch_daemon_ask_agent(&req)
                .await
                .map(Json)
                .map_err(|e| format!("ask_agent via daemon failed: {e}"));
        }

        execute_ask_agent(&req).await.map(Json)
    }

    #[tool(description = "Fan out a request to Gemini and Codex in parallel with role-adapted prompts.")]
    async fn ask_twins(
        &self,
        Parameters(req): Parameters<AskTwinsRequest>,
    ) -> Result<Json<AskTwinsResponse>, String> {
        if use_daemon_for_mcp() {
            return fetch_daemon_ask_twins(&req)
                .await
                .map(Json)
                .map_err(|e| format!("ask_twins via daemon failed: {e}"));
        }

        execute_ask_twins(&req).await.map(Json)
    }

    #[tool(description = "Create a persistent named session for an agent.")]
    async fn spawn_session(
        &self,
        Parameters(req): Parameters<SpawnSessionRequest>,
    ) -> Result<String, String> {
        let agent = req.agent.to_lowercase();
        if !is_supported_agent_name(&agent) {
            return Err("spawn_session supports only 'gemini' or 'codex'".to_string());
        }

        let mut sessions = self.sessions.lock().await;
        sessions.insert(
            req.name.clone(),
            SessionState {
                agent: agent.clone(),
                history: Vec::new(),
            },
        );
        persist_sessions_if_enabled(&self.sessions_file, &sessions)
            .map_err(|e| format!("failed to persist sessions: {e}"))?;
        Ok(format!("session '{}' spawned for {}", req.name, agent))
    }

    #[tool(description = "Ask within a named persistent session.")]
    async fn ask_session(
        &self,
        Parameters(req): Parameters<AskSessionRequest>,
    ) -> Result<String, String> {
        let (agent, prompt) = {
            let mut sessions = self.sessions.lock().await;
            let state = sessions
                .get_mut(&req.name)
                .ok_or_else(|| format!("session '{}' not found", req.name))?;

            let context = if state.history.is_empty() {
                String::new()
            } else {
                format!("Previous turns:\n{}\n\n", state.history.join("\n"))
            };
            let prompt = format!("{context}New user message:\n{}", req.message);
            state.history.push(req.message.clone());
            (state.agent.clone(), prompt)
        };

        let response = run_named_agent(&agent, &prompt)
            .await
            .map_err(|e| format!("ask_session failed: {e}"))?;

        let mut sessions = self.sessions.lock().await;
        if let Some(state) = sessions.get_mut(&req.name) {
            state.history.push(format!("assistant: {response}"));
        }
        persist_sessions_if_enabled(&self.sessions_file, &sessions)
            .map_err(|e| format!("failed to persist sessions: {e}"))?;

        Ok(response)
    }

    #[tool(description = "Dismiss a named session.")]
    async fn dismiss_session(
        &self,
        Parameters(req): Parameters<DismissSessionRequest>,
    ) -> Result<String, String> {
        let mut sessions = self.sessions.lock().await;
        match sessions.remove(&req.name) {
            Some(_) => {
                persist_sessions_if_enabled(&self.sessions_file, &sessions)
                    .map_err(|e| format!("failed to persist sessions: {e}"))?;
                Ok(format!("session '{}' dismissed", req.name))
            }
            None => Err(format!("session '{}' not found", req.name)),
        }
    }

    #[tool(description = "List active sessions.")]
    async fn list_sessions(&self) -> Json<SessionListResponse> {
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
        if use_daemon_for_mcp()
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
        if use_daemon_for_mcp() {
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
            ts_ms: now_ms(),
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
        if use_daemon_for_mcp() {
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
        if use_daemon_for_mcp() {
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
        if use_daemon_for_mcp() {
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
        if use_daemon_for_mcp() {
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
        if use_daemon_for_mcp() {
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
        if use_daemon_for_mcp() {
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
        if use_daemon_for_mcp() {
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

fn gemini_command() -> (String, Vec<String>) {
    let bin = std::env::var("TRIUMVIRATE_GEMINI_BIN").unwrap_or_else(|_| "mock-gemini".to_string());
    let args = std::env::var("TRIUMVIRATE_GEMINI_ARGS")
        .map(|v| v.split_whitespace().map(ToString::to_string).collect())
        .unwrap_or_else(|_| Vec::new());
    (bin, args)
}

fn codex_command() -> (String, Vec<String>) {
    let bin = std::env::var("TRIUMVIRATE_CODEX_BIN").unwrap_or_else(|_| "mock-codex".to_string());
    let args = std::env::var("TRIUMVIRATE_CODEX_ARGS")
        .map(|v| v.split_whitespace().map(ToString::to_string).collect())
        .unwrap_or_else(|_| Vec::new());
    (bin, args)
}

fn use_daemon_for_mcp() -> bool {
    // Bridge can be forced to proxy tool execution through daemon HTTP so ephemeral MCP lifetimes
    // never own long-running agent work.
    std::env::var("TRIUMVIRATE_MCP_USE_DAEMON")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

async fn execute_ask_agent(req: &AskAgentRequest) -> Result<AskAgentResponse, String> {
    if !is_supported_agent(req) {
        return Err("ask_agent supports only agent='gemini' or agent='codex'".to_string());
    }
    let agent = req.agent.to_lowercase();
    let request_id = Uuid::new_v4().to_string();
    let (resolved_cwd, resolved_repo, resolved_branch) =
        core_resolve_context(req.cwd.as_ref(), req.repo.as_ref(), req.branch.as_ref());

    // Emit lifecycle states in-band so clients can render progress before native MCP progress
    // notifications are wired in a later increment.
    let mut lifecycle = vec![LifecycleEvent {
        state: "SPAWNED".to_string(),
        detail: format!(
            "Started {} connector{}{}{}",
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
                .unwrap_or_default()
        ),
    }];
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: now_ms(),
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

    lifecycle.push(LifecycleEvent {
        state: "WORKING".to_string(),
        detail: format!("{agent} is processing request"),
    });
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: now_ms(),
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

    let backoffs = [Duration::from_millis(250), Duration::from_secs(1), Duration::from_secs(2)];
    let mut last_err: Option<String> = None;

    for (idx, backoff) in backoffs.iter().enumerate() {
        match run_named_agent(&agent, &req.message).await {
            Ok(response) => {
                lifecycle.push(LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: format!("{agent} responded on attempt {}", idx + 1),
                });
                if let Err(e) = append_outbox_event(&OutboxEvent {
                    ts_ms: now_ms(),
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
                return Ok(AskAgentResponse {
                    request_id,
                    agent: agent.clone(),
                    response,
                    lifecycle,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("timed out") {
                    lifecycle.push(LifecycleEvent {
                        state: "TIMEOUT".to_string(),
                        detail: format!("{agent} timed out on attempt {}", idx + 1),
                    });
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
                    ts_ms: now_ms(),
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
        ts_ms: now_ms(),
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
            ts_ms: now_ms(),
            request_id: request_id.clone(),
            tool: "ask_agent".to_string(),
            status: "FALLBACK".to_string(),
            agent: Some(agent.clone()),
            detail: format!("dead drop launched: {}", path.display()),
            cwd: resolved_cwd.clone(),
            repo: resolved_repo.clone(),
            branch: resolved_branch.clone(),
        });
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

async fn execute_ask_twins(req: &AskTwinsRequest) -> Result<AskTwinsResponse, String> {
    let request_id = Uuid::new_v4().to_string();
    let (resolved_cwd, resolved_repo, resolved_branch) =
        core_resolve_context(req.cwd.as_ref(), req.repo.as_ref(), req.branch.as_ref());
    let (gemini_prompt, codex_prompt) = build_role_adapted_prompts(&AskTwinsRequest {
        message: req.message.clone(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
    });

    let mut lifecycle = vec![
        LifecycleEvent {
            state: "SPAWNED".to_string(),
            detail: "Gemini request sent".to_string(),
        },
        LifecycleEvent {
            state: "SPAWNED".to_string(),
            detail: "Codex request sent".to_string(),
        },
        LifecycleEvent {
            state: "WORKING".to_string(),
            detail: "Gemini and Codex processing in parallel".to_string(),
        },
    ];
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: now_ms(),
        request_id: request_id.clone(),
        tool: "ask_twins".to_string(),
        status: "WORKING".to_string(),
        agent: None,
        detail: "Gemini and Codex processing in parallel".to_string(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }

    let gemini_fut = run_named_agent("gemini", &gemini_prompt);
    let codex_fut = run_named_agent("codex", &codex_prompt);
    let (gemini_out, codex_out) = tokio::join!(gemini_fut, codex_fut);

    let mut results = Vec::new();
    let mut failures = Vec::new();

    match gemini_out {
        Ok(response) => {
            lifecycle.push(LifecycleEvent {
                state: "DONE".to_string(),
                detail: "Gemini responded".to_string(),
            });
            results.push(AgentResult {
                agent: "gemini".to_string(),
                response,
                prompt_sent: gemini_prompt,
            });
            let _ = append_outbox_event(&OutboxEvent {
                ts_ms: now_ms(),
                request_id: request_id.clone(),
                tool: "ask_twins".to_string(),
                status: "DONE".to_string(),
                agent: Some("gemini".to_string()),
                detail: "Gemini responded".to_string(),
                cwd: resolved_cwd.clone(),
                repo: resolved_repo.clone(),
                branch: resolved_branch.clone(),
            });
        }
        Err(e) => {
            let detail = format!("Gemini failed: {e}");
            lifecycle.push(LifecycleEvent {
                state: "FAILED".to_string(),
                detail: detail.clone(),
            });
            failures.push(LifecycleEvent {
                state: "FAILED".to_string(),
                detail,
            });
            let _ = append_outbox_event(&OutboxEvent {
                ts_ms: now_ms(),
                request_id: request_id.clone(),
                tool: "ask_twins".to_string(),
                status: "FAILED".to_string(),
                agent: Some("gemini".to_string()),
                detail: "Gemini failed".to_string(),
                cwd: resolved_cwd.clone(),
                repo: resolved_repo.clone(),
                branch: resolved_branch.clone(),
            });
            if let Ok(path) = spawn_dead_drop(
                "gemini",
                &req.message,
                &e.to_string(),
                &resolved_cwd,
                &resolved_repo,
                &resolved_branch,
            ) {
                let info = format!("Gemini dead drop launched: {}", path.display());
                lifecycle.push(LifecycleEvent {
                    state: "FALLBACK".to_string(),
                    detail: info.clone(),
                });
                failures.push(LifecycleEvent {
                    state: "FALLBACK".to_string(),
                    detail: info,
                });
            }
        }
    }

    match codex_out {
        Ok(response) => {
            lifecycle.push(LifecycleEvent {
                state: "DONE".to_string(),
                detail: "Codex responded".to_string(),
            });
            results.push(AgentResult {
                agent: "codex".to_string(),
                response,
                prompt_sent: codex_prompt,
            });
            let _ = append_outbox_event(&OutboxEvent {
                ts_ms: now_ms(),
                request_id: request_id.clone(),
                tool: "ask_twins".to_string(),
                status: "DONE".to_string(),
                agent: Some("codex".to_string()),
                detail: "Codex responded".to_string(),
                cwd: resolved_cwd.clone(),
                repo: resolved_repo.clone(),
                branch: resolved_branch.clone(),
            });
        }
        Err(e) => {
            let detail = format!("Codex failed: {e}");
            lifecycle.push(LifecycleEvent {
                state: "FAILED".to_string(),
                detail: detail.clone(),
            });
            failures.push(LifecycleEvent {
                state: "FAILED".to_string(),
                detail,
            });
            let _ = append_outbox_event(&OutboxEvent {
                ts_ms: now_ms(),
                request_id: request_id.clone(),
                tool: "ask_twins".to_string(),
                status: "FAILED".to_string(),
                agent: Some("codex".to_string()),
                detail: "Codex failed".to_string(),
                cwd: resolved_cwd.clone(),
                repo: resolved_repo.clone(),
                branch: resolved_branch.clone(),
            });
            if let Ok(path) = spawn_dead_drop(
                "codex",
                &req.message,
                &e.to_string(),
                &resolved_cwd,
                &resolved_repo,
                &resolved_branch,
            ) {
                let info = format!("Codex dead drop launched: {}", path.display());
                lifecycle.push(LifecycleEvent {
                    state: "FALLBACK".to_string(),
                    detail: info.clone(),
                });
                failures.push(LifecycleEvent {
                    state: "FALLBACK".to_string(),
                    detail: info,
                });
            }
        }
    }

    if results.is_empty() {
        return Err(format!(
            "ask_twins failed for both agents: {}",
            failures
                .iter()
                .map(|f| f.detail.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    Ok(AskTwinsResponse {
        request_id,
        results,
        failures,
        lifecycle,
    })
}

async fn run_named_agent(agent: &str, message: &str) -> anyhow::Result<String> {
    match agent {
        "gemini" => {
            let (bin, args) = gemini_command();
            run_agent_process(&bin, &args, message).await
        }
        "codex" => {
            let (bin, args) = codex_command();
            run_agent_process(&bin, &args, message).await
        }
        _ => anyhow::bail!("unsupported agent: {agent}"),
    }
}

async fn run_agent_process(bin: &str, args: &[String], message: &str) -> anyhow::Result<String> {
    let mut child = Command::new(&bin)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
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
    let mut lines = BufReader::new(stdout).lines();

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
            }
        }
        Err(anyhow::anyhow!("no result.text message from gemini connector"))
    })
    .await;

    let response = match read_result {
        Ok(result) => result?,
        Err(_) => anyhow::bail!("gemini connector timed out"),
    };

    let _ = child.kill().await;
    let _ = child.wait().await;

    Ok(response)
}

#[tool_handler]
impl ServerHandler for McpBridge {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Triumvirate MCP bridge. Use `ping` to verify connectivity.")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "triumvirate=info".into()),
        )
        .with_target(false)
        .init();

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
            let health = fetch_daemon_status().await?;
            let snapshot = fetch_daemon_status_snapshot().await?;
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                "health": health,
                "snapshot": snapshot
            }))?);
        }
        CliCommand::Doctor => {
            run_doctor().await?;
        }
    }

    Ok(())
}

fn launch_agent_plist(exe_path: &str, home_dir: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.triumvirate.daemon-v2</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe_path}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>EnvironmentVariables</key>
  <dict>
    <key>TRIUMVIRATE_HOME</key>
    <string>{home_dir}</string>
  </dict>
  <key>StandardOutPath</key>
  <string>{home_dir}/daemon.log</string>
  <key>StandardErrorPath</key>
  <string>{home_dir}/daemon.err.log</string>
</dict>
</plist>
"#
    )
}

fn run_install() -> anyhow::Result<()> {
    let home = triumvirate_home_dir()?;
    fs::create_dir_all(&home)?;
    let launch_agents = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?
        .join("Library/LaunchAgents");
    fs::create_dir_all(&launch_agents)?;

    let plist_path = launch_agents.join("com.triumvirate.daemon-v2.plist");
    let exe_path = std::env::current_exe()?;
    let plist = launch_agent_plist(&exe_path.display().to_string(), &home.display().to_string());
    fs::write(&plist_path, plist)?;

    println!("Installed launchd plist at {}", plist_path.display());
    println!("Load with: launchctl load {}", plist_path.display());
    println!("Start now with: launchctl start com.triumvirate.daemon-v2");
    Ok(())
}

fn launchd_plist_path() -> anyhow::Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?
        .join("Library/LaunchAgents/com.triumvirate.daemon-v2.plist"))
}

fn run_uninstall() -> anyhow::Result<()> {
    let plist_path = launchd_plist_path()?;
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
    let token_path = daemon_token_path()?;
    let plist_path = launchd_plist_path()?;
    let daemon_health = fetch_daemon_status().await.ok();
    let report = serde_json::json!({
        "token_file_exists": token_path.exists(),
        "token_file_path": token_path,
        "launchd_plist_exists": plist_path.exists(),
        "launchd_plist_path": plist_path,
        "daemon_reachable": daemon_health.is_some(),
        "daemon_health": daemon_health
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn run_daemon() -> anyhow::Result<()> {
    #[derive(Debug, Clone)]
    struct DaemonState {
        token: String,
        queues: QueueRegistry,
    }

    async fn health(
        State(state): State<DaemonState>,
        headers: HeaderMap,
    ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
        if !is_authorized(&headers, &state.token) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(AxumJson(serde_json::json!({
            "status": "ok",
            "service": "triumvirate-daemon-v2",
            "mode": "incremental-dev"
        })))
    }

    async fn status(
        State(state): State<DaemonState>,
        headers: HeaderMap,
    ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
        if !is_authorized(&headers, &state.token) {
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
            "fallback_tickets": tickets
        })))
    }

    async fn ask_agent_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<AskAgentRequest>,
    ) -> Result<AxumJson<AskAgentResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_authorized(&headers, &state.token) {
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
        execute_ask_agent(&req).await.map(AxumJson).map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                AxumJson(serde_json::json!({ "error": e })),
            )
        })
    }

    async fn ask_twins_route(
        State(state): State<DaemonState>,
        headers: HeaderMap,
        AxumJson(req): AxumJson<AskTwinsRequest>,
    ) -> Result<AxumJson<AskTwinsResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
        if !is_authorized(&headers, &state.token) {
            return Err((
                StatusCode::UNAUTHORIZED,
                AxumJson(serde_json::json!({ "error": "unauthorized" })),
            ));
        }
        let queue = core_acquire_project_queue(
            &state.queues,
            core_project_queue_key(req.cwd.as_ref(), req.repo.as_ref()),
        )
        .await;
        let _guard = queue.lock().await;
        execute_ask_twins(&req).await.map(AxumJson).map_err(|e| {
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
        if !is_authorized(&headers, &state.token) {
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
            ts_ms: now_ms(),
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
        if !is_authorized(&headers, &state.token) {
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
        if !is_authorized(&headers, &state.token) {
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
        if !is_authorized(&headers, &state.token) {
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
        if !is_authorized(&headers, &state.token) {
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
        if !is_authorized(&headers, &state.token) {
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
        if !is_authorized(&headers, &state.token) {
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
        if !is_authorized(&headers, &state.token) {
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

    let token = ensure_daemon_token()?;
    let state = DaemonState {
        token,
        queues: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/ask-agent", post(ask_agent_route))
        .route("/ask-twins", post(ask_twins_route))
        .route("/memory/write", post(memory_write_route))
        .route("/memory/read", post(memory_read_route))
        .route("/scratchpad/write", post(scratchpad_write_route))
        .route("/scratchpad/list", post(scratchpad_list_route))
        .route("/outbox/recent", post(outbox_recent_route))
        .route("/fallback/list", post(fallback_list_route))
        .route("/fallback/ack", post(fallback_ack_route))
        .route("/fallback/gc", post(fallback_gc_route))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn triumvirate_home_dir() -> anyhow::Result<PathBuf> {
    if let Ok(override_dir) = std::env::var("TRIUMVIRATE_HOME") {
        return Ok(PathBuf::from(override_dir));
    }
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?;
    Ok(home.join(".triumvirate"))
}

fn daemon_token_path() -> anyhow::Result<PathBuf> {
    Ok(triumvirate_home_dir()?.join("daemon.token"))
}

fn ensure_daemon_token() -> anyhow::Result<String> {
    core_ensure_daemon_token(&triumvirate_home_dir()?)
}

fn sessions_file_path() -> anyhow::Result<PathBuf> {
    Ok(core_sessions_file_path(&triumvirate_home_dir()?))
}

fn load_sessions(path: &PathBuf) -> anyhow::Result<HashMap<String, SessionState>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    core_load_json_file::<HashMap<String, SessionState>>(path)
}

fn persist_sessions_if_enabled(
    maybe_path: &Option<PathBuf>,
    sessions: &HashMap<String, SessionState>,
) -> anyhow::Result<()> {
    let Some(path) = maybe_path else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    core_persist_json_file(path, sessions)?;
    Ok(())
}

fn append_outbox_event(event: &OutboxEvent) -> anyhow::Result<()> {
    core_append_outbox_event(&triumvirate_home_dir()?, event)
}

fn read_outbox_events() -> anyhow::Result<Vec<OutboxEvent>> {
    core_read_outbox_events(&triumvirate_home_dir()?)
}

fn append_memory_entry(entry: &MemoryEntry) -> anyhow::Result<()> {
    core_append_memory_entry(&triumvirate_home_dir()?, entry)
}

fn read_memory_entries() -> anyhow::Result<Vec<MemoryEntry>> {
    core_read_memory_entries(&triumvirate_home_dir()?)
}

fn write_scratchpad(project: &str, topic: &str, content: &str) -> anyhow::Result<PathBuf> {
    core_write_scratchpad(&triumvirate_home_dir()?, project, topic, content, now_ms())
}

fn list_scratchpad(project: &str) -> anyhow::Result<Vec<PathBuf>> {
    core_list_scratchpad(&triumvirate_home_dir()?, project)
}

fn now_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
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
    create_dead_drop_ticket(
        &triumvirate_home_dir()?,
        agent,
        message,
        reason,
        cwd,
        repo,
        branch,
        &id,
    )
}

fn count_pending_fallbacks() -> anyhow::Result<usize> {
    count_dead_drop_tickets(&triumvirate_home_dir()?)
}

fn list_pending_fallback_paths(limit: usize) -> anyhow::Result<Vec<PathBuf>> {
    list_dead_drop_tickets(&triumvirate_home_dir()?, limit)
}

fn acknowledge_fallback_path(path: &str) -> anyhow::Result<()> {
    acknowledge_dead_drop_ticket(&triumvirate_home_dir()?, path)
}

fn gc_fallbacks(max_age_days: u64) -> anyhow::Result<usize> {
    gc_dead_drop_tickets(&triumvirate_home_dir()?, max_age_days)
}

fn is_authorized(headers: &HeaderMap, token: &str) -> bool {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    let expected = format!("Bearer {token}");
    value == expected
}

fn daemon_status_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_URL")
        .unwrap_or_else(|_| format!("{}/status", daemon_base_url()))
}

static DAEMON_AUTOSTART_ATTEMPTED: AtomicBool = AtomicBool::new(false);

fn daemon_autostart_enabled() -> bool {
    std::env::var("TRIUMVIRATE_DAEMON_AUTOSTART")
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

#[cfg(test)]
fn reset_daemon_autostart_flag_for_tests() {
    DAEMON_AUTOSTART_ATTEMPTED.store(false, Ordering::SeqCst);
}

fn attempt_daemon_autostart_once() -> anyhow::Result<bool> {
    if !daemon_autostart_enabled() {
        return Ok(false);
    }
    if DAEMON_AUTOSTART_ATTEMPTED.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }

    if std::env::var("TRIUMVIRATE_DAEMON_AUTOSTART_DRYRUN")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
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

async fn daemon_get_json<T: DeserializeOwned>(url: String) -> anyhow::Result<T> {
    let token = ensure_daemon_token()?;
    let client = reqwest::Client::new();

    let first = client.get(&url).bearer_auth(&token).send().await;
    match first {
        Ok(response) => {
            if !response.status().is_success() {
                anyhow::bail!("daemon responded with HTTP {}", response.status());
            }
            return Ok(response.json::<T>().await?);
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
        }
    }
    anyhow::bail!("daemon request failed");
}

async fn daemon_post_json<TReq: serde::Serialize, TResp: DeserializeOwned>(
    url: String,
    payload: &TReq,
) -> anyhow::Result<TResp> {
    let token = ensure_daemon_token()?;
    let client = reqwest::Client::new();

    let first = client.post(&url).bearer_auth(&token).json(payload).send().await;
    match first {
        Ok(response) => {
            if !response.status().is_success() {
                anyhow::bail!("daemon responded with HTTP {}", response.status());
            }
            return Ok(response.json::<TResp>().await?);
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
        }
    }
    anyhow::bail!("daemon request failed");
}

async fn fetch_daemon_status() -> anyhow::Result<DaemonHealthResponse> {
    daemon_get_json::<DaemonHealthResponse>(daemon_status_url()).await
}

async fn fetch_daemon_status_snapshot() -> anyhow::Result<DaemonStatusSnapshot> {
    daemon_get_json::<DaemonStatusSnapshot>(daemon_status_url()).await
}

fn daemon_base_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
}

fn daemon_ask_agent_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL")
        .unwrap_or_else(|_| format!("{}/ask-agent", daemon_base_url()))
}

fn daemon_ask_twins_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_ASK_TWINS_URL")
        .unwrap_or_else(|_| format!("{}/ask-twins", daemon_base_url()))
}

fn daemon_memory_write_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_MEMORY_WRITE_URL")
        .unwrap_or_else(|_| format!("{}/memory/write", daemon_base_url()))
}

fn daemon_memory_read_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_MEMORY_READ_URL")
        .unwrap_or_else(|_| format!("{}/memory/read", daemon_base_url()))
}

fn daemon_scratchpad_write_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SCRATCHPAD_WRITE_URL")
        .unwrap_or_else(|_| format!("{}/scratchpad/write", daemon_base_url()))
}

fn daemon_scratchpad_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SCRATCHPAD_LIST_URL")
        .unwrap_or_else(|_| format!("{}/scratchpad/list", daemon_base_url()))
}

fn daemon_outbox_recent_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_OUTBOX_RECENT_URL")
        .unwrap_or_else(|_| format!("{}/outbox/recent", daemon_base_url()))
}

fn daemon_fallback_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_LIST_URL")
        .unwrap_or_else(|_| format!("{}/fallback/list", daemon_base_url()))
}

fn daemon_fallback_ack_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_ACK_URL")
        .unwrap_or_else(|_| format!("{}/fallback/ack", daemon_base_url()))
}

fn daemon_fallback_gc_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_GC_URL")
        .unwrap_or_else(|_| format!("{}/fallback/gc", daemon_base_url()))
}

async fn fetch_daemon_ask_agent(req: &AskAgentRequest) -> anyhow::Result<AskAgentResponse> {
    daemon_post_json::<AskAgentRequest, AskAgentResponse>(daemon_ask_agent_url(), req).await
}

async fn fetch_daemon_ask_twins(req: &AskTwinsRequest) -> anyhow::Result<AskTwinsResponse> {
    daemon_post_json::<AskTwinsRequest, AskTwinsResponse>(daemon_ask_twins_url(), req).await
}

async fn fetch_daemon_memory_write(req: &MemoryWriteRequest) -> anyhow::Result<MemoryWriteResponse> {
    daemon_post_json::<MemoryWriteRequest, MemoryWriteResponse>(daemon_memory_write_url(), req).await
}

async fn fetch_daemon_memory_read(req: &MemoryReadRequest) -> anyhow::Result<MemoryReadResponse> {
    daemon_post_json::<MemoryReadRequest, MemoryReadResponse>(daemon_memory_read_url(), req).await
}

async fn fetch_daemon_scratchpad_write(
    req: &ScratchpadWriteRequest,
) -> anyhow::Result<ScratchpadWriteResponse> {
    daemon_post_json::<ScratchpadWriteRequest, ScratchpadWriteResponse>(
        daemon_scratchpad_write_url(),
        req,
    )
    .await
}

async fn fetch_daemon_scratchpad_list(
    req: &ScratchpadListRequest,
) -> anyhow::Result<ScratchpadListResponse> {
    daemon_post_json::<ScratchpadListRequest, ScratchpadListResponse>(
        daemon_scratchpad_list_url(),
        req,
    )
    .await
}

async fn fetch_daemon_outbox_recent(
    req: &OutboxRecentRequest,
) -> anyhow::Result<OutboxRecentResponse> {
    daemon_post_json::<OutboxRecentRequest, OutboxRecentResponse>(daemon_outbox_recent_url(), req)
        .await
}

async fn fetch_daemon_fallback_list(
    req: &FallbackListRequest,
) -> anyhow::Result<FallbackListResponse> {
    daemon_post_json::<FallbackListRequest, FallbackListResponse>(daemon_fallback_list_url(), req)
        .await
}

async fn fetch_daemon_fallback_ack(req: &FallbackAckRequest) -> anyhow::Result<String> {
    let json = daemon_post_json::<FallbackAckRequest, serde_json::Value>(
        daemon_fallback_ack_url(),
        req,
    )
    .await?;
    Ok(json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("acknowledged")
        .to_string())
}

async fn fetch_daemon_fallback_gc(req: &FallbackGcRequest) -> anyhow::Result<FallbackGcResponse> {
    daemon_post_json::<FallbackGcRequest, FallbackGcResponse>(daemon_fallback_gc_url(), req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{ClientHandler, model::ClientInfo};
    use rmcp::model::CallToolRequestParams;
    use std::{
        fs,
        net::SocketAddr,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::{Mutex, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Debug, Clone, Default)]
    struct NoopClient;

    impl ClientHandler for NoopClient {
        fn get_info(&self) -> ClientInfo {
            ClientInfo::default()
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
    async fn ask_twins_parallel_and_role_adapted() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let gemini_script = write_mock_agent_script("gemini", 1.0)?;
        let codex_script = write_mock_agent_script("codex", 0.2)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", gemini_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", codex_script.as_os_str());
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
            "message": "Add auth module",
            "cwd": "/tmp/project",
            "repo": "triumvirate",
            "branch": "feat/mcp-first"
        });

        let start = std::time::Instant::now();
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_twins")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;
        let elapsed = start.elapsed();

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();

        assert!(elapsed < Duration::from_secs(2));
        assert!(raw_text.contains("gemini done"));
        assert!(raw_text.contains("codex done"));
        assert!(raw_text.contains("[Gemini role: research/analysis]"));
        assert!(raw_text.contains("[Codex role: implementation/testing]"));

        client.cancel().await?;
        server_handle.await??;

        let _ = fs::remove_file(gemini_script);
        let _ = fs::remove_file(codex_script);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn ask_twins_returns_partial_when_one_agent_fails() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let gemini_script = write_mock_agent_script("gemini", 0.0)?;
        let codex_fail_script = write_failing_agent_script("codex")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", gemini_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", codex_fail_script.as_os_str());
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
            "message": "partial success test"
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_twins")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(raw_text.contains("gemini done"));
        assert!(raw_text.contains("\"failures\""));
        assert!(raw_text.contains("Codex failed"));

        client.cancel().await?;
        server_handle.await??;

        let _ = fs::remove_file(gemini_script);
        let _ = fs::remove_file(codex_fail_script);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
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

        client.cancel().await?;
        server_handle.await??;
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

        async fn status_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
        ) -> Result<AxumJson<serde_json::Value>, StatusCode> {
            if !is_authorized(&headers, &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(serde_json::json!({
                "daemon_mode": "daemon-snapshot",
                "supported_agents": ["gemini", "codex", "claude"],
                "pending_fallbacks": 7,
                "fallback_tickets": ["x.md", "y.md"]
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
            std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "true");
            std::env::set_var("TRIUMVIRATE_DAEMON_URL", format!("http://{addr}/status"));
        }

        let bridge = McpBridge::new_ephemeral();
        let status = bridge.get_status().await;
        assert_eq!(status.0.daemon_mode, "daemon-snapshot");
        assert_eq!(status.0.pending_fallbacks, 7);
        assert_eq!(status.0.fallback_tickets.len(), 2);
        assert!(status.0.supported_agents.contains(&"claude".to_string()));

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

        let token_one = ensure_daemon_token()?;
        let token_two = ensure_daemon_token()?;
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
        assert!(is_authorized(&headers, token));
        assert!(!is_authorized(&headers, "wrong"));
    }

    #[test]
    fn launch_agent_plist_contains_expected_values() {
        let plist = launch_agent_plist("/usr/local/bin/triumvirate", "/tmp/tri-home");
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
            if !is_authorized(&headers, &state.token) {
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
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
        }
        let _ = fs::remove_dir_all(test_home);
        Ok(())
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
            if !is_authorized(&headers, &state.token) {
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
    async fn fetch_daemon_ask_twins_uses_bearer_token() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-ask-twins-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "twins-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
        }

        async fn ask_twins_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(_req): AxumJson<AskTwinsRequest>,
        ) -> Result<AxumJson<AskTwinsResponse>, StatusCode> {
            if !is_authorized(&headers, &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(AskTwinsResponse {
                request_id: "daemon-req-2".to_string(),
                results: vec![AgentResult {
                    agent: "gemini".to_string(),
                    response: "daemon twins result".to_string(),
                    prompt_sent: "prompt".to_string(),
                }],
                failures: vec![],
                lifecycle: vec![LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: "served by daemon".to_string(),
                }],
            }))
        }

        let app = Router::new()
            .route("/ask-twins", post(ask_twins_handler))
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
                "TRIUMVIRATE_DAEMON_ASK_TWINS_URL",
                format!("http://{addr}/ask-twins"),
            );
        }

        let out = fetch_daemon_ask_twins(&AskTwinsRequest {
            message: "fan out".to_string(),
            cwd: None,
            repo: None,
            branch: None,
        })
        .await?;
        assert_eq!(out.results.len(), 1);
        assert_eq!(out.results[0].response, "daemon twins result");
        assert_eq!(out.lifecycle.first().map(|e| e.state.as_str()), Some("DONE"));

        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_TWINS_URL");
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
            if !is_authorized(&headers, &state.token) {
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
    async fn mcp_ask_twins_uses_daemon_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let test_home = std::env::temp_dir().join(format!("triumvirate-mcp-twins-daemon-{now}"));
        fs::create_dir_all(&test_home)?;
        let token = "mcp-twins-daemon-token-123";
        fs::write(test_home.join("daemon.token"), format!("{token}\n"))?;

        #[derive(Clone)]
        struct TestState {
            token: String,
        }

        async fn ask_twins_handler(
            State(state): State<TestState>,
            headers: HeaderMap,
            AxumJson(_req): AxumJson<AskTwinsRequest>,
        ) -> Result<AxumJson<AskTwinsResponse>, StatusCode> {
            if !is_authorized(&headers, &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(AxumJson(AskTwinsResponse {
                request_id: "daemon-req-twins".to_string(),
                results: vec![AgentResult {
                    agent: "codex".to_string(),
                    response: "daemon twins path used".to_string(),
                    prompt_sent: "daemon prompt".to_string(),
                }],
                failures: vec![],
                lifecycle: vec![LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: "daemon twins served".to_string(),
                }],
            }))
        }

        let app = Router::new()
            .route("/ask-twins", post(ask_twins_handler))
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
                "TRIUMVIRATE_DAEMON_ASK_TWINS_URL",
                format!("http://{addr}/ask-twins"),
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
            "message": "should proxy twins"
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_twins")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(raw_text.contains("daemon twins path used"));
        assert!(raw_text.contains("daemon twins served"));

        client.cancel().await?;
        server_handle.await??;
        server.abort();
        let _ = server.await;

        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_HOME");
            std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_TWINS_URL");
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
            if !is_authorized(&headers, &state.token) {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let id = "daemon-memory-1".to_string();
            let mut entries = state.entries.lock().expect("entries lock poisoned");
            entries.push(MemoryEntry {
                id: id.clone(),
                namespace: req.namespace,
                key: req.key,
                value: req.value,
                ts_ms: now_ms(),
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
            if !is_authorized(&headers, &state.token) {
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
            if !is_authorized(&headers, &state.token) {
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
            if !is_authorized(&headers, &state.token) {
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
            if !is_authorized(&headers, &state.token) {
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
            if !is_authorized(&headers, &state.token) {
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
            if !is_authorized(&headers, &state.token) {
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
            if !is_authorized(&headers, &state.token) {
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
        })
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
        })
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
                    history: vec!["hello".to_string()],
                },
            );
            persist_sessions_if_enabled(&first.sessions_file, &sessions)?;
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
