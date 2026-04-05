use std::sync::Arc;

use tokio::sync::watch;
use tracing::{info, warn};
use triumvirate_proto::{AgentId, HealthStatus};

use super::connector::AgentConnector;
use crate::fabric::MessageBus;

/// Claude CLI connector.
///
/// Integration path: `claude -p --output-format stream-json`
/// - Single-shot JSONL streaming (each invocation is one turn)
/// - For multi-turn: `claude --resume <session_id> -p --output-format stream-json`
/// - Session ID preserved across turns for context continuity
///
/// POC 2 will add PTY mode for full interactive sessions.
pub struct ClaudeConnector {
    session_id: Option<String>,
    health_status: HealthStatus,
    health_tx: watch::Sender<HealthStatus>,
    health_rx: watch::Receiver<HealthStatus>,
}

impl ClaudeConnector {
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
impl AgentConnector for ClaudeConnector {
    fn agent_id(&self) -> AgentId {
        AgentId::Claude
    }

    async fn spawn(&mut self, _bus: Arc<MessageBus>) -> anyhow::Result<()> {
        // POC 1: Verify claude CLI exists, mark ready
        match tokio::process::Command::new("which")
            .arg("claude")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout);
                info!(agent = "claude", path = %path.trim(), "CLI found");
                self.set_health(HealthStatus::Ready);
            }
            _ => {
                warn!(agent = "claude", "CLI not found in PATH");
                self.set_health(HealthStatus::Dead);
            }
        }
        Ok(())
    }

    async fn send(&self, message: &str) -> anyhow::Result<()> {
        // POC 1: Log the message. POC 2: Write to subprocess stdin.
        info!(agent = "claude", session = ?self.session_id, %message, "would send to Claude CLI");
        Ok(())
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        info!(agent = "claude", "shutting down");
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
