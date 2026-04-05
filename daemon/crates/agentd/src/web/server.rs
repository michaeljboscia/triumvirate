use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use rust_embed::Embed;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;
use triumvirate_proto::{AgentId, FabricMessage, HealthStatus, Payload, Topic};

use crate::agent::SharedHealthRegistry;
use crate::fabric::MessageBus;
use crate::routing::{RoutingDecision, decide_route};
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
}

/// Start the web dashboard server on the given port.
///
/// Per GR1-D1: Web-Only UI — this is the exclusive conversation interface.
/// Temporal UI at :8233 is accessible via "Developer Tools" link (GR1-D6).
pub async fn start_web_server(
    bus: Arc<MessageBus>,
    health: SharedHealthRegistry,
    port: u16,
) -> anyhow::Result<()> {
    let state = AppState { bus, health };

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/health", get(health_handler))
        .route("/api/agents", get(agents_handler))
        .route("/api/message", post(message_handler))
        .route("/ws", get(ws_handler))
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    info!(port, "web dashboard listening");

    axum::serve(listener, app).await?;
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

            state
                .bus
                .emit(FabricMessage::new(
                    AgentId::Human,
                    Topic::AgentInput(agent),
                    Payload::HumanMessage { content },
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
