use std::sync::Arc;

use tokio::sync::watch;
use tracing::{info, warn};
use triumvirate_proto::{AgentId, HealthStatus};

use super::connector::AgentConnector;
use crate::fabric::MessageBus;

/// Gemini CLI connector.
///
/// Integration path: `gemini --acp` (Agent Communication Protocol)
/// - Persistent subprocess over stdio
/// - JSON-RPC protocol: send requests, receive responses
/// - Multi-turn sessions with session save/load
/// - Cancellation via JSON-RPC cancel method
/// - Model switching mid-session
///
/// ACP is the clear winner over headless mode (-p) which is single-shot only.
/// See research/030-gemini-cli-deep-dive.md for full protocol details.
pub struct GeminiConnector {
    health_status: HealthStatus,
    health_tx: watch::Sender<HealthStatus>,
    health_rx: watch::Receiver<HealthStatus>,
}

impl GeminiConnector {
    pub fn new() -> Self {
        let (health_tx, health_rx) = watch::channel(HealthStatus::Starting);
        Self {
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
impl AgentConnector for GeminiConnector {
    fn agent_id(&self) -> AgentId {
        AgentId::Gemini
    }

    async fn spawn(&mut self, _bus: Arc<MessageBus>) -> anyhow::Result<()> {
        match tokio::process::Command::new("which")
            .arg("gemini")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout);
                info!(agent = "gemini", path = %path.trim(), "CLI found");
                self.set_health(HealthStatus::Ready);
            }
            _ => {
                warn!(agent = "gemini", "CLI not found in PATH");
                self.set_health(HealthStatus::Dead);
            }
        }
        Ok(())
    }

    async fn send(&self, message: &str) -> anyhow::Result<()> {
        info!(agent = "gemini", %message, "would send to Gemini ACP");
        Ok(())
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        info!(agent = "gemini", "shutting down");
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
