use clap::{Parser, Subcommand};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::router::tool::ToolRouter,
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};

#[derive(Debug, Parser)]
#[command(name = "triumvirate")]
#[command(about = "Triumvirate v2 daemon + MCP bridge binary")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the MCP stdio bridge.
    Mcp,
    /// Run the long-lived daemon (stub in Increment 1a).
    Daemon,
}

#[derive(Debug, Clone)]
struct McpBridge {
    tool_router: ToolRouter<Self>,
}

impl McpBridge {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl McpBridge {
    #[tool(description = "Health check tool for MCP connectivity")]
    async fn ping(&self) -> String {
        "pong".to_string()
    }
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
        Command::Mcp => {
            McpBridge::new().serve(stdio()).await?.waiting().await?;
        }
        Command::Daemon => {
            println!("daemon mode is not implemented in Increment 1a yet");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{ClientHandler, model::ClientInfo};
    use rmcp::model::CallToolRequestParams;

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
            McpBridge::new().serve(server_transport).await?.waiting().await?;
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
}
