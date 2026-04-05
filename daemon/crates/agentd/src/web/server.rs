use std::sync::Arc;
use std::path::PathBuf;

use axum::Json;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use rust_embed::Embed;
use rusqlite::Connection;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use triumvirate_workflow::WorkflowStore;
use triumvirate_proto::{AgentId, FabricMessage, HealthStatus, Payload, Topic};

use crate::agent::SharedHealthRegistry;
use crate::fabric::MessageBus;
use crate::fleet::merge::merge_branches_sequentially;
use crate::fleet::worktree::{git_repo_root, parse_fleet_members, provision_worktree, remove_worktree};
use crate::quota::SharedQuotaRegistry;
use crate::routing::{RoutingDecision, decide_route};
use crate::shutdown::wait_for_shutdown_signal;
use crate::web::ws_handler;

/// Static assets embedded in the binary via rust-embed.
/// In production, the Svelte build output goes here.
/// For POC 1, it's a single index.html.
#[derive(Embed)]
#[folder = "../../static/"]
struct Assets;

/// Shared state available to all HTTP handlers.
#[derive(Clone)]
pub struct AppState {
    pub bus: Arc<MessageBus>,
    pub health: SharedHealthRegistry,
    pub quota: SharedQuotaRegistry,
    pub memory_db_path: PathBuf,
    pub workflow_db_path: PathBuf,
}

/// Start the web dashboard server on the given port.
///
/// Per GR1-D1: Web-Only UI — this is the exclusive conversation interface.
/// Temporal UI at :8233 is accessible via "Developer Tools" link (GR1-D6).
pub async fn start_web_server(
    bus: Arc<MessageBus>,
    health: SharedHealthRegistry,
    quota: SharedQuotaRegistry,
    memory_db_path: PathBuf,
    workflow_db_path: PathBuf,
    port: u16,
) -> anyhow::Result<()> {
    let state = AppState {
        bus,
        health,
        quota,
        memory_db_path,
        workflow_db_path,
    };

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/health", get(health_handler))
        .route("/api/agents", get(agents_handler))
        .route("/api/quota", get(quota_handler))
        .route("/api/workflows", get(workflows_handler))
        .route("/api/decisions", get(decisions_handler))
        .route("/api/fleet/tasks", get(fleet_tasks_handler))
        .route("/api/fleet/worktrees", get(fleet_worktrees_handler))
        .route("/api/fleet/spawn", post(fleet_spawn_handler))
        .route("/api/fleet/merge", post(fleet_merge_handler))
        .route("/api/fleet/worktrees/teardown", post(fleet_teardown_handler))
        .route("/api/fleet/tasks/claim", post(fleet_claim_handler))
        .route("/api/fleet/tasks/complete", post(fleet_complete_handler))
        .route("/api/message", post(message_handler))
        .route("/ws", get(ws_handler))
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    info!(port, "web dashboard listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown_signal())
        .await?;
    Ok(())
}

async fn index_handler() -> impl IntoResponse {
    match Assets::get("index.html") {
        Some(content) => Html(
            String::from_utf8_lossy(&content.data).to_string(),
        )
        .into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "index.html not found").into_response(),
    }
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let statuses = state.health.snapshot().await;
    let claude = status_label(statuses.get(&AgentId::Claude).copied().unwrap_or(HealthStatus::Starting));
    let gemini = status_label(statuses.get(&AgentId::Gemini).copied().unwrap_or(HealthStatus::Starting));
    let codex = status_label(statuses.get(&AgentId::Codex).copied().unwrap_or(HealthStatus::Starting));

    let degraded = [claude, gemini, codex]
        .iter()
        .any(|status| *status == "dead" || *status == "unresponsive");

    axum::Json(serde_json::json!({
        "status": if degraded { "degraded" } else { "ok" },
        "version": env!("CARGO_PKG_VERSION"),
        "agents": {
            "claude": claude,
            "gemini": gemini,
            "codex": codex
        }
    }))
}

async fn agents_handler(State(state): State<AppState>) -> impl IntoResponse {
    let statuses = state.health.snapshot().await;
    let claude = status_label(statuses.get(&AgentId::Claude).copied().unwrap_or(HealthStatus::Starting));
    let gemini = status_label(statuses.get(&AgentId::Gemini).copied().unwrap_or(HealthStatus::Starting));
    let codex = status_label(statuses.get(&AgentId::Codex).copied().unwrap_or(HealthStatus::Starting));

    axum::Json(serde_json::json!([
        { "id": "claude", "name": "Claude", "model": "Opus 4.6", "status": claude },
        { "id": "gemini", "name": "Gemini", "model": "Pro 2M", "status": gemini },
        { "id": "codex", "name": "Codex", "model": "GPT-5.2", "status": codex }
    ]))
}

async fn quota_handler(State(state): State<AppState>) -> impl IntoResponse {
    let snapshots = state.quota.snapshot_all().await;
    axum::Json(serde_json::json!({
        "agents": {
            "claude": snapshots.get(&AgentId::Claude),
            "gemini": snapshots.get(&AgentId::Gemini),
            "codex": snapshots.get(&AgentId::Codex),
        }
    }))
}

async fn workflows_handler(State(state): State<AppState>) -> impl IntoResponse {
    match WorkflowStore::open(&state.workflow_db_path)
        .and_then(|store| store.resumable_workflows())
    {
        Ok(workflows) => {
            let workflows_json: Vec<_> = workflows
                .into_iter()
                .map(|w| {
                    serde_json::json!({
                        "workflow_id": w.workflow_id,
                        "workflow_type": w.workflow_type,
                        "state": w.state,
                        "current_step": w.current_step,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "workflows": workflows_json }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to read workflows: {e}") })),
        )
            .into_response(),
    }
}

async fn decisions_handler(State(state): State<AppState>) -> impl IntoResponse {
    match Connection::open(&state.memory_db_path) {
        Ok(conn) => {
            let mut stmt = match conn.prepare(
                "SELECT id, session_id, decision_text, proposed_by, validated_by, created_at, evidence
                 FROM decisions
                 ORDER BY id DESC
                 LIMIT 200",
            ) {
                Ok(stmt) => stmt,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("failed to prepare decisions query: {e}") })),
                    )
                        .into_response();
                }
            };

            let rows = match stmt.query_map([], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, i64>(0)?,
                    "session_id": row.get::<_, String>(1)?,
                    "decision_text": row.get::<_, String>(2)?,
                    "proposed_by": row.get::<_, String>(3)?,
                    "validated_by": row.get::<_, Option<String>>(4)?,
                    "created_at": row.get::<_, String>(5)?,
                    "evidence": row.get::<_, Option<String>>(6)?,
                }))
            }) {
                Ok(rows) => rows,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("failed to query decisions: {e}") })),
                    )
                        .into_response();
                }
            };

            let mut decisions = Vec::new();
            for row in rows {
                match row {
                    Ok(value) => decisions.push(value),
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("failed to read decisions row: {e}") })),
                        )
                            .into_response();
                    }
                }
            }

            (StatusCode::OK, Json(serde_json::json!({ "decisions": decisions }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to open memory db: {e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct FleetSpawnRequest {
    spec: String,
}

async fn fleet_spawn_handler(
    State(state): State<AppState>,
    Json(req): Json<FleetSpawnRequest>,
) -> impl IntoResponse {
    let spec = req.spec.trim();
    if spec.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "spec must not be empty" })),
        )
            .into_response();
    }

    let fleet_id = format!("fleet-{}", uuid::Uuid::new_v4());

    match Connection::open(&state.memory_db_path) {
        Ok(conn) => {
            let members = parse_fleet_members(spec);
            let mut created = Vec::new();
            for member in &members {
                match provision_worktree(&fleet_id, member, "HEAD") {
                    Ok(worktree) => {
                        if let Err(e) = conn.execute(
                            "INSERT INTO fleet_worktrees (fleet_id, member_key, agent_type, branch_name, worktree_path, status)
                             VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
                            rusqlite::params![
                                fleet_id,
                                worktree.member_key,
                                worktree.agent_type,
                                worktree.branch_name,
                                worktree.worktree_path.display().to_string()
                            ],
                        ) {
                            let _ = remove_worktree(&worktree.worktree_path.display().to_string());
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({ "error": format!("failed to persist worktree: {e}") })),
                            )
                                .into_response();
                        }
                        created.push(worktree);
                    }
                    Err(e) => {
                        for existing in created {
                            let _ = remove_worktree(&existing.worktree_path.display().to_string());
                        }
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("failed to provision worktree: {e}") })),
                        )
                            .into_response();
                    }
                }
            }

            let tasks = [
                ("contracts", "Define contracts", "pending", Option::<&str>::None),
                ("implementation", "Parallel implementation", "blocked", Some("contracts")),
                ("merge", "Sequential merge", "blocked", Some("implementation")),
            ];

            for (task_key, title, status, depends_on) in tasks {
                if let Err(e) = conn.execute(
                    "INSERT INTO fleet_tasks (fleet_id, task_key, title, status, depends_on)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![fleet_id, task_key, title, status, depends_on],
                ) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("failed to create fleet tasks: {e}") })),
                    )
                        .into_response();
                }
            }

            state
                .bus
                .emit(FabricMessage::new(
                    AgentId::Human,
                    Topic::TaskCreated,
                    Payload::HumanMessage {
                        content: format!("fleet spawned: {fleet_id} ({spec})"),
                    },
                ))
                .await;

            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "accepted": true,
                    "fleet_id": fleet_id,
                    "spec": spec,
                    "members": members.into_iter().map(|m| m.member_key).collect::<Vec<_>>(),
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to open memory db: {e}") })),
        )
            .into_response(),
    }
}

async fn fleet_worktrees_handler(State(state): State<AppState>) -> impl IntoResponse {
    match Connection::open(&state.memory_db_path) {
        Ok(conn) => {
            let mut stmt = match conn.prepare(
                "SELECT fleet_id, member_key, agent_type, branch_name, worktree_path, status, created_at, updated_at
                 FROM fleet_worktrees
                 ORDER BY id DESC
                 LIMIT 500",
            ) {
                Ok(stmt) => stmt,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("failed to prepare fleet_worktrees query: {e}") })),
                    )
                        .into_response();
                }
            };

            let rows = match stmt.query_map([], |row| {
                Ok(serde_json::json!({
                    "fleet_id": row.get::<_, String>(0)?,
                    "member_key": row.get::<_, String>(1)?,
                    "agent_type": row.get::<_, String>(2)?,
                    "branch_name": row.get::<_, String>(3)?,
                    "worktree_path": row.get::<_, String>(4)?,
                    "status": row.get::<_, String>(5)?,
                    "created_at": row.get::<_, String>(6)?,
                    "updated_at": row.get::<_, String>(7)?,
                }))
            }) {
                Ok(rows) => rows,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("failed to query fleet_worktrees: {e}") })),
                    )
                        .into_response();
                }
            };

            let mut worktrees = Vec::new();
            for row in rows {
                match row {
                    Ok(value) => worktrees.push(value),
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("failed to read fleet_worktrees row: {e}") })),
                        )
                            .into_response();
                    }
                }
            }

            (StatusCode::OK, Json(serde_json::json!({ "worktrees": worktrees }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to open memory db: {e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct FleetTeardownRequest {
    fleet_id: String,
}

async fn fleet_teardown_handler(
    State(state): State<AppState>,
    Json(req): Json<FleetTeardownRequest>,
) -> impl IntoResponse {
    let fleet_id = req.fleet_id.trim();
    if fleet_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "fleet_id is required" })),
        )
            .into_response();
    }

    let conn = match Connection::open(&state.memory_db_path) {
        Ok(conn) => conn,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to open memory db: {e}") })),
            )
                .into_response();
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT worktree_path FROM fleet_worktrees WHERE fleet_id = ?1 AND status = 'active'",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to query fleet_worktrees: {e}") })),
            )
                .into_response();
        }
    };

    let paths = match stmt.query_map(rusqlite::params![fleet_id], |row| row.get::<_, String>(0)) {
        Ok(rows) => {
            let mut out = Vec::new();
            for row in rows {
                match row {
                    Ok(path) => out.push(path),
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("failed to read worktree row: {e}") })),
                        )
                            .into_response();
                    }
                }
            }
            out
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to map worktree rows: {e}") })),
            )
                .into_response();
        }
    };

    let mut removed = Vec::new();
    for path in &paths {
        if let Err(e) = remove_worktree(path) {
            let _ = conn.execute(
                "UPDATE fleet_worktrees
                 SET status = 'failed', updated_at = datetime('now')
                 WHERE fleet_id = ?1 AND worktree_path = ?2",
                rusqlite::params![fleet_id, path],
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to remove worktree {path}: {e}") })),
            )
                .into_response();
        }
        let _ = conn.execute(
            "UPDATE fleet_worktrees
             SET status = 'removed', updated_at = datetime('now')
             WHERE fleet_id = ?1 AND worktree_path = ?2",
            rusqlite::params![fleet_id, path],
        );
        removed.push(path.clone());
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "fleet_id": fleet_id,
            "removed_worktrees": removed,
        })),
    )
        .into_response()
}

#[derive(Debug, serde::Deserialize)]
struct FleetMergeRequest {
    fleet_id: String,
}

async fn fleet_merge_handler(
    State(state): State<AppState>,
    Json(req): Json<FleetMergeRequest>,
) -> impl IntoResponse {
    let fleet_id = req.fleet_id.trim();
    if fleet_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "fleet_id is required" })),
        )
            .into_response();
    }

    let conn = match Connection::open(&state.memory_db_path) {
        Ok(conn) => conn,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to open memory db: {e}") })),
            )
                .into_response();
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT branch_name
         FROM fleet_worktrees
         WHERE fleet_id = ?1
         ORDER BY id ASC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to prepare branch query: {e}") })),
            )
                .into_response();
        }
    };

    let branches = match stmt.query_map(rusqlite::params![fleet_id], |row| row.get::<_, String>(0))
    {
        Ok(rows) => {
            let mut out = Vec::new();
            for row in rows {
                match row {
                    Ok(branch) => out.push(branch),
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("failed to read branch row: {e}") })),
                        )
                            .into_response();
                    }
                }
            }
            out
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to map branch rows: {e}") })),
            )
                .into_response();
        }
    };

    if branches.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no branches found for fleet_id" })),
        )
            .into_response();
    }

    let repo_root = match git_repo_root() {
        Ok(path) => path,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to detect git repo: {e}") })),
            )
                .into_response();
        }
    };

    match merge_branches_sequentially(&repo_root, &branches) {
        Ok((merged, failed_branch)) => {
            let conflict = failed_branch.is_some();
            if !conflict {
                let _ = conn.execute(
                    "UPDATE fleet_tasks
                     SET status = 'completed', updated_at = datetime('now')
                     WHERE fleet_id = ?1 AND task_key = 'merge'",
                    rusqlite::params![fleet_id],
                );
            }

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "fleet_id": fleet_id,
                    "repo_root": repo_root.display().to_string(),
                    "merged_branches": merged,
                    "failed_branch": failed_branch,
                    "conflict": conflict,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": format!("merge failed: {e}") })),
        )
            .into_response(),
    }
}

async fn fleet_tasks_handler(State(state): State<AppState>) -> impl IntoResponse {
    match Connection::open(&state.memory_db_path) {
        Ok(conn) => {
            let mut stmt = match conn.prepare(
                "SELECT fleet_id, task_key, title, status, assigned_agent, depends_on, created_at, updated_at
                 FROM fleet_tasks
                 ORDER BY id DESC
                 LIMIT 500",
            ) {
                Ok(stmt) => stmt,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("failed to prepare fleet_tasks query: {e}") })),
                    )
                        .into_response();
                }
            };

            let rows = match stmt.query_map([], |row| {
                Ok(serde_json::json!({
                    "fleet_id": row.get::<_, String>(0)?,
                    "task_key": row.get::<_, String>(1)?,
                    "title": row.get::<_, String>(2)?,
                    "status": row.get::<_, String>(3)?,
                    "assigned_agent": row.get::<_, Option<String>>(4)?,
                    "depends_on": row.get::<_, Option<String>>(5)?,
                    "created_at": row.get::<_, String>(6)?,
                    "updated_at": row.get::<_, String>(7)?,
                }))
            }) {
                Ok(rows) => rows,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("failed to query fleet_tasks: {e}") })),
                    )
                        .into_response();
                }
            };

            let mut tasks = Vec::new();
            for row in rows {
                match row {
                    Ok(value) => tasks.push(value),
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("failed to read fleet_tasks row: {e}") })),
                        )
                            .into_response();
                    }
                }
            }

            (StatusCode::OK, Json(serde_json::json!({ "tasks": tasks }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to open memory db: {e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct FleetTaskClaimRequest {
    fleet_id: String,
    task_key: String,
    agent: String,
}

async fn fleet_claim_handler(
    State(state): State<AppState>,
    Json(req): Json<FleetTaskClaimRequest>,
) -> impl IntoResponse {
    let fleet_id = req.fleet_id.trim();
    let task_key = req.task_key.trim();
    let agent = req.agent.trim();
    if fleet_id.is_empty() || task_key.is_empty() || agent.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "fleet_id, task_key, and agent are required" })),
        )
            .into_response();
    }

    let conn = match Connection::open(&state.memory_db_path) {
        Ok(conn) => conn,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to open memory db: {e}") })),
            )
                .into_response();
        }
    };

    let mut depends_on: Option<String> = None;
    let mut current_status: Option<String> = None;
    let lookup = conn.query_row(
        "SELECT status, depends_on
         FROM fleet_tasks
         WHERE fleet_id = ?1 AND task_key = ?2",
        rusqlite::params![fleet_id, task_key],
        |row| {
            current_status = Some(row.get::<_, String>(0)?);
            depends_on = row.get::<_, Option<String>>(1)?;
            Ok(())
        },
    );
    if let Err(e) = lookup {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("task not found: {e}") })),
        )
            .into_response();
    }

    if current_status.as_deref() != Some("pending") && current_status.as_deref() != Some("blocked")
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "task is not claimable in current state" })),
        )
            .into_response();
    }

    if let Some(dep_key) = depends_on {
        let dep_status = conn.query_row(
            "SELECT status FROM fleet_tasks WHERE fleet_id = ?1 AND task_key = ?2",
            rusqlite::params![fleet_id, dep_key],
            |row| row.get::<_, String>(0),
        );
        match dep_status {
            Ok(status) if status != "completed" => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": "dependency not completed" })),
                )
                    .into_response();
            }
            Ok(_) => {}
            Err(e) => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": format!("failed dependency lookup: {e}") })),
                )
                    .into_response();
            }
        }
    }

    if let Err(e) = conn.execute(
        "UPDATE fleet_tasks
         SET status = 'in_progress',
             assigned_agent = ?3,
             updated_at = datetime('now')
         WHERE fleet_id = ?1 AND task_key = ?2",
        rusqlite::params![fleet_id, task_key, agent],
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to claim task: {e}") })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "claimed": true,
            "fleet_id": fleet_id,
            "task_key": task_key,
            "agent": agent
        })),
    )
        .into_response()
}

#[derive(Debug, serde::Deserialize)]
struct FleetTaskCompleteRequest {
    fleet_id: String,
    task_key: String,
}

async fn fleet_complete_handler(
    State(state): State<AppState>,
    Json(req): Json<FleetTaskCompleteRequest>,
) -> impl IntoResponse {
    let fleet_id = req.fleet_id.trim();
    let task_key = req.task_key.trim();
    if fleet_id.is_empty() || task_key.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "fleet_id and task_key are required" })),
        )
            .into_response();
    }

    let conn = match Connection::open(&state.memory_db_path) {
        Ok(conn) => conn,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to open memory db: {e}") })),
            )
                .into_response();
        }
    };

    if let Err(e) = conn.execute(
        "UPDATE fleet_tasks
         SET status = 'completed',
             updated_at = datetime('now')
         WHERE fleet_id = ?1 AND task_key = ?2",
        rusqlite::params![fleet_id, task_key],
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to complete task: {e}") })),
        )
            .into_response();
    }

    if let Err(e) = conn.execute(
        "UPDATE fleet_tasks
         SET status = 'pending',
             updated_at = datetime('now')
         WHERE fleet_id = ?1 AND depends_on = ?2 AND status = 'blocked'",
        rusqlite::params![fleet_id, task_key],
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to unblock dependents: {e}") })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "completed": true,
            "fleet_id": fleet_id,
            "task_key": task_key
        })),
    )
        .into_response()
}

#[derive(Debug, serde::Deserialize)]
struct MessageRequest {
    content: String,
}

#[derive(Debug, serde::Serialize)]
struct MessageResponse {
    accepted: bool,
}

async fn message_handler(
    State(state): State<AppState>,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    let content = req.content.trim().to_string();
    if content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "content must not be empty" })),
        )
            .into_response();
    }

    state
        .bus
        .emit(FabricMessage::new(
            AgentId::Human,
            Topic::HumanInput,
            Payload::HumanMessage {
                content: content.clone(),
            },
        ))
        .await;

    let is_direct_mention = content.starts_with('@');
    let decision = decide_route(&content);
    let (reason, target_agent) = match decision {
        RoutingDecision::Agent { agent, content } => {
            if content.is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "message content must not be empty after target prefix" })),
                )
                    .into_response();
            }

            let enriched_content =
                match inject_memory_context(&state.memory_db_path, agent, &content) {
                    Ok(value) => value,
                    Err(e) => {
                        warn!(agent = %agent, error = %e, "failed to inject memory context");
                        content.clone()
                    }
                };

            state
                .bus
                .emit(FabricMessage::new(
                    AgentId::Human,
                    Topic::AgentInput(agent),
                    Payload::HumanMessage {
                        content: enriched_content,
                    },
                ))
                .await;
            let reason = if is_direct_mention {
                "direct_mention".to_string()
            } else {
                "lead_default".to_string()
            };
            (reason, Some(agent))
        }
        RoutingDecision::Debate { topic } => {
            state
                .bus
                .emit(FabricMessage::new(
                    AgentId::Human,
                    Topic::DebateProposal,
                    Payload::HumanMessage { content: topic },
                ))
                .await;
            ("debate_command".to_string(), None)
        }
        RoutingDecision::Fleet { spec } => {
            state
                .bus
                .emit(FabricMessage::new(
                    AgentId::Human,
                    Topic::TaskCreated,
                    Payload::HumanMessage { content: spec },
                ))
                .await;
            ("fleet_command".to_string(), None)
        }
        RoutingDecision::Status => {
            state
                .bus
                .emit(FabricMessage::new(
                    AgentId::System,
                    Topic::SystemHealth,
                    Payload::HealthChange {
                        agent: AgentId::System,
                        status: triumvirate_proto::HealthStatus::Ready,
                        detail: Some("status command requested".to_string()),
                    },
                ))
                .await;
            ("status_command".to_string(), None)
        }
    };

    if let Some(target_agent) = target_agent {
        state
            .bus
            .emit(FabricMessage::new(
                AgentId::System,
                Topic::TaskProgress,
                Payload::RoutingDecision {
                    target_agent,
                    reason,
                    content,
                },
            ))
            .await;
    }

    (StatusCode::ACCEPTED, Json(MessageResponse { accepted: true })).into_response()
}

fn inject_memory_context(
    memory_db_path: &std::path::Path,
    target_agent: AgentId,
    content: &str,
) -> anyhow::Result<String> {
    let conn = Connection::open(memory_db_path)?;
    let mut stmt = conn.prepare(
        "SELECT decision_text, created_at
         FROM decisions
         ORDER BY id DESC
         LIMIT 5",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut decisions = Vec::new();
    for row in rows {
        let (decision_text, created_at) = row?;
        decisions.push(format!("- [{created_at}] {decision_text}"));
    }

    if decisions.is_empty() {
        return Ok(content.to_string());
    }

    let context_block = decisions.join("\n");
    Ok(format!(
        "[TRIUMVIRATE CONTEXT]\nTarget agent: {target_agent}\nRecent decisions:\n{context_block}\n\n[USER REQUEST]\n{content}"
    ))
}

fn status_label(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Starting => "starting",
        HealthStatus::Ready => "ready",
        HealthStatus::Busy => "busy",
        HealthStatus::Unresponsive => "unresponsive",
        HealthStatus::Restarting => "restarting",
        HealthStatus::Dead => "dead",
    }
}

/// Serve embedded static files (CSS, JS, images).
async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    match Assets::get(path) {
        Some(content) => {
            let mime = content.metadata.mimetype();
            (
                [(header::CONTENT_TYPE, mime)],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
