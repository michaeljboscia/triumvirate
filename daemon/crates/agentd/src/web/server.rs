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
use triumvirate_proto::{AgentId, FabricMessage, Payload, Topic};

use crate::fabric::MessageBus;
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
}

/// Start the web dashboard server on the given port.
///
/// Per GR1-D1: Web-Only UI — this is the exclusive conversation interface.
/// Temporal UI at :8233 is accessible via "Developer Tools" link (GR1-D6).
pub async fn start_web_server(bus: Arc<MessageBus>, port: u16) -> anyhow::Result<()> {
    let state = AppState { bus };

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

async fn health_handler(State(_state): State<AppState>) -> impl IntoResponse {
    // POC 1: Return basic health. POC 2+: Include per-agent health from bus.
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "agents": {
            "claude": "ready",
            "gemini": "ready",
            "codex": "ready"
        }
    }))
}

async fn agents_handler(State(_state): State<AppState>) -> impl IntoResponse {
    axum::Json(serde_json::json!([
        { "id": "claude", "name": "Claude", "model": "Opus 4.6", "status": "ready" },
        { "id": "gemini", "name": "Gemini", "model": "Pro 2M", "status": "ready" },
        { "id": "codex", "name": "Codex", "model": "GPT-5.2", "status": "ready" }
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

    let (topic, routed_content) = route_message(&content);
    if routed_content.is_empty() {
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
            Topic::HumanInput,
            Payload::HumanMessage {
                content: content.clone(),
            },
        ))
        .await;

    state
        .bus
        .emit(FabricMessage::new(
            AgentId::Human,
            topic,
            Payload::HumanMessage {
                content: routed_content,
            },
        ))
        .await;

    (StatusCode::ACCEPTED, Json(MessageResponse { accepted: true })).into_response()
}

fn route_message(content: &str) -> (Topic, String) {
    let trimmed = content.trim();

    let routes = [
        ("@claude", AgentId::Claude),
        ("@gemini", AgentId::Gemini),
        ("@codex", AgentId::Codex),
    ];

    for (prefix, agent) in routes {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return (Topic::AgentInput(agent), rest.trim().to_string());
        }
    }

    (Topic::AgentInput(AgentId::Claude), trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::route_message;
    use triumvirate_proto::{AgentId, Topic};

    #[test]
    fn routes_to_claude_by_default() {
        let (topic, content) = route_message("hello world");
        assert!(matches!(topic, Topic::AgentInput(AgentId::Claude)));
        assert_eq!(content, "hello world");
    }

    #[test]
    fn routes_to_gemini_when_prefixed() {
        let (topic, content) = route_message("@gemini summarize this");
        assert!(matches!(topic, Topic::AgentInput(AgentId::Gemini)));
        assert_eq!(content, "summarize this");
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
