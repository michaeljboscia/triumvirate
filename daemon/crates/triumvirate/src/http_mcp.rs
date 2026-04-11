//! Streamable HTTP MCP transport endpoint.
//!
//! Mounts at /mcp on the daemon's Axum server. Uses rmcp's StreamableHttpService
//! as a tower::Service that handles POST (JSON-RPC tool calls) and GET (SSE stream)
//! with session management via Mcp-Session-Id header.
//!
//! FEAT-002 (REQ-H01, REQ-H02, REQ-H03, REQ-H04, REQ-H07)

use std::sync::Arc;

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig,
    StreamableHttpService,
    session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

use crate::McpBridge;

/// Build the StreamableHttpService that serves MCP over HTTP/SSE.
///
/// The service_factory creates a new McpBridge instance per MCP session.
/// All instances share the same underlying Arc-wrapped state (sessions,
/// fleet, ABE tasks, metrics, WS events, token DB).
///
/// Mount the returned service on the Axum router via:
/// ```rust,ignore
/// .nest_service("/mcp", build_streamable_http_mcp_service(bridge_template))
/// ```
pub fn build_streamable_http_mcp_service(
    bridge_template: McpBridge,
    cancellation_token: CancellationToken,
) -> StreamableHttpService<
    impl rmcp::Service<rmcp::RoleServer> + Send + 'static,
    LocalSessionManager,
> {
    let config = StreamableHttpServerConfig {
        sse_keep_alive: Some(std::time::Duration::from_secs(15)),
        sse_retry: Some(std::time::Duration::from_secs(3)),
        stateful_mode: true,
        json_response: false,
        cancellation_token,
    };

    let session_manager = Arc::new(LocalSessionManager::default());

    StreamableHttpService::new(
        move || {
            let bridge = bridge_template.clone();
            Ok(rmcp::service::serve_directly(bridge))
        },
        session_manager,
        config,
    )
}
