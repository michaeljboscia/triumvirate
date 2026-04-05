use std::sync::Arc;

use tokio::sync::watch;
use triumvirate_proto::{AgentId, HealthStatus};

use crate::fabric::MessageBus;

/// Trait for all agent CLI connectors.
///
/// Each implementation manages a persistent subprocess (PTY in POC 2+),
/// parses its output into FabricMessages, and routes input from the bus.
///
/// The lifecycle: spawn() → ready → send()/recv loop → shutdown()
#[async_trait::async_trait]
pub trait AgentConnector: Send + Sync {
    /// Which agent this connector manages.
    fn agent_id(&self) -> AgentId;

    /// Spawn the CLI subprocess and begin reading output.
    /// Returns once the process is alive (not necessarily ready).
    async fn spawn(&mut self, bus: Arc<MessageBus>) -> anyhow::Result<()>;

    /// Send a message to the agent's stdin.
    async fn send(&self, message: &str) -> anyhow::Result<()>;

    /// Gracefully shut down the subprocess.
    async fn shutdown(&mut self) -> anyhow::Result<()>;

    /// Current health status.
    fn health(&self) -> HealthStatus;

    /// Subscribe to health status changes.
    fn health_watch(&self) -> watch::Receiver<HealthStatus>;
}

/// Handle returned after spawning an agent — allows the daemon to interact
/// with the running connector without holding a mutable reference.
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub health_rx: watch::Receiver<HealthStatus>,
}
