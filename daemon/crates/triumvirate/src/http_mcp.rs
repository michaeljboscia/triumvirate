//! Streamable HTTP MCP transport endpoint.
//!
//! Mounts at /mcp on the daemon's Axum server. Uses rmcp's StreamableHttpService
//! as a tower::Service that handles POST (JSON-RPC tool calls) and GET (SSE stream)
//! with session management via Mcp-Session-Id header.
//!
//! Bearer token auth is enforced via a tower middleware layer on the /mcp route,
//! matching the existing HTTP API auth pattern.
//!
//! FEAT-002 (REQ-H01, REQ-H02, REQ-H03, REQ-H04, REQ-H07, REQ-H09)

use std::sync::Arc;

use axum::{Router, middleware, response::Response};
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig,
    StreamableHttpService,
    session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

use crate::McpBridge;

/// Build an Axum Router for the /mcp endpoint with auth + StreamableHttpService.
///
/// The returned router includes bearer token auth middleware and the rmcp
/// StreamableHttpService as a nested service. Mount via:
/// ```rust,ignore
/// .nest("/mcp", http_mcp::build_mcp_router(bridge, token, cancel))
/// ```
pub fn build_mcp_router(
    bridge_template: McpBridge,
    bearer_token: String,
    cancellation_token: CancellationToken,
) -> Router {
    let mcp_service = build_streamable_http_service(bridge_template, cancellation_token);

    Router::new()
        .nest_service("/", mcp_service)
        .layer(middleware::from_fn(move |req, next| {
            let token = bearer_token.clone();
            bearer_auth_middleware(token, req, next)
        }))
}

/// Bearer token auth middleware for the /mcp endpoint.
/// Returns 401 Unauthorized if the token is missing or wrong.
/// REQ-H09
async fn bearer_auth_middleware(
    expected_token: String,
    req: Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let is_authorized = auth_header
        .as_deref()
        .and_then(|h: &str| h.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected_token);

    if !is_authorized {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}

fn build_streamable_http_service(
    bridge_template: McpBridge,
    cancellation_token: CancellationToken,
) -> StreamableHttpService<McpBridge, LocalSessionManager> {
    let config = StreamableHttpServerConfig::default()
        .with_sse_keep_alive(Some(std::time::Duration::from_secs(15)))
        .with_sse_retry(Some(std::time::Duration::from_secs(3)))
        .with_stateful_mode(true)
        .with_json_response(false)
        .with_cancellation_token(cancellation_token);

    let session_manager = Arc::new(LocalSessionManager::default());

    StreamableHttpService::new(
        move || Ok(bridge_template.clone()),
        session_manager,
        config,
    )
}
