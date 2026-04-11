//! Minimal MCP server that sends progress notifications over Streamable HTTP.
//! Used to test whether Claude Code renders intermediate SSE frames.

use std::sync::Arc;

use axum::Router;
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, Content, NumberOrString, ProgressNotificationParam, ProgressToken,
        ServerCapabilities, ServerInfo,
    },
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpService, StreamableHttpServerConfig,
        session::local::LocalSessionManager,
    },
};
use serde::Deserialize;
use tokio::time::{Duration, sleep};

#[derive(Clone)]
struct SpikeServer;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SlowTestParams {
    /// Optional label to include in progress messages.
    #[allow(dead_code)]
    label: Option<String>,
}

#[tool_router]
impl SpikeServer {
    /// Sleeps 5 times (1 second each), sending a progress notification after each step.
    #[tool(description = "Slow operation that sends progress notifications over 5 seconds")]
    async fn slow_test(
        &self,
        #[allow(unused_variables)] Parameters(_params): Parameters<SlowTestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, String> {
        for step in 1u32..=5 {
            sleep(Duration::from_secs(1)).await;

            let percentage = f64::from(step) * 20.0;
            let token = ProgressToken(NumberOrString::Number(step as i64));
            let mut params = ProgressNotificationParam::new(token, percentage);
            params.total = Some(100.0);
            params.message = Some(format!("Step {step}/5"));

            context.peer.notify_progress(params).await.ok();
        }

        Ok(CallToolResult::success(vec![Content::text(
            "Done after 5 seconds",
        )]))
    }
}

#[tool_handler]
impl ServerHandler for SpikeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("SSE spike test server. Call slow_test to observe progress frames.")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = StreamableHttpServerConfig::default()
        .with_sse_keep_alive(Some(Duration::from_secs(15)))
        .with_stateful_mode(true)
        .with_json_response(false);

    let session_manager = Arc::new(LocalSessionManager::default());
    let mcp_service = StreamableHttpService::new(|| Ok(SpikeServer), session_manager, config);

    let app = Router::new().nest_service("/mcp", mcp_service);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:9999").await?;
    println!("SSE spike server listening on http://127.0.0.1:9999/mcp");

    axum::serve(listener, app).await?;

    Ok(())
}
