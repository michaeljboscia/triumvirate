use std::sync::Arc;

use tokio::sync::watch;
use tracing::{info, warn};
use triumvirate_proto::{AgentId, HealthStatus};

use super::connector::AgentConnector;
use crate::fabric::MessageBus;

/// Codex CLI connector.
///
/// Integration paths (from research/031-codex-cli-deep-dive.md):
///
/// 1. `codex exec --json` — Primary headless path
///    - JSONL events on stdout (thread.started, turn.started, item.*, turn.completed)
///    - Single-shot per invocation, multi-turn via `codex exec resume <session_id>`
///    - stdout is sacred: only JSONL in --json mode
///    - Approval requests auto-rejected in exec mode
///
/// 2. `codex mcp-server` — Persistent subprocess (JSON-RPC over stdio)
///    - Multi-turn via `codex-reply` tool
///    - MCP protocol, not raw JSONL
///
/// 3. `codex app-server` — WebSocket/stdio (experimental)
///    - Full thread API, richest integration surface
///    - Maturity: experimental
///
/// POC 1 targets `codex exec --json`. POC 2 evaluates mcp-server for persistence.
pub struct CodexConnector {
    #[allow(dead_code)]
    session_id: Option<String>,
    health_status: HealthStatus,
    health_tx: watch::Sender<HealthStatus>,
    health_rx: watch::Receiver<HealthStatus>,
}

impl CodexConnector {
    pub fn new() -> Self {
        let (health_tx, health_rx) = watch::channel(HealthStatus::Starting);
        Self {
            session_id: None,
            health_status: HealthStatus::Starting,
            health_tx,
            health_rx,
        }
    }

    fn set_health(&mut self, status: HealthStatus) {
        self.health_status = status;
        let _ = self.health_tx.send(status);
    }
}

#[async_trait::async_trait]
impl AgentConnector for CodexConnector {
    fn agent_id(&self) -> AgentId {
        AgentId::Codex
    }

    async fn spawn(&mut self, _bus: Arc<MessageBus>) -> anyhow::Result<()> {
        match tokio::process::Command::new("which")
            .arg("codex")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout);
                info!(agent = "codex", path = %path.trim(), "CLI found");
                self.set_health(HealthStatus::Ready);
            }
            _ => {
                warn!(agent = "codex", "CLI not found in PATH");
                self.set_health(HealthStatus::Dead);
            }
        }
        Ok(())
    }

    async fn send(&self, message: &str) -> anyhow::Result<()> {
        info!(agent = "codex", session = ?self.session_id, %message, "would send to Codex exec");
        Ok(())
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        info!(agent = "codex", "shutting down");
        self.set_health(HealthStatus::Dead);
        Ok(())
    }

    fn health(&self) -> HealthStatus {
        self.health_status
    }

    fn health_watch(&self) -> watch::Receiver<HealthStatus> {
        self.health_rx.clone()
    }
}
