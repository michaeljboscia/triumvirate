use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tracing::{info, warn};
use triumvirate_proto::{
    AgentId, CodexEventKind, FabricMessage, HealthStatus, Payload, Topic, parse_codex_event,
};
use uuid::Uuid;

use super::connector::AgentConnector;
use crate::fabric::MessageBus;

/// Codex CLI connector.
///
/// Integration path: `codex mcp-server`
/// - Persistent subprocess over stdio
/// - JSON-RPC/MCP framed messages
/// - Input routed from Topic::AgentInput(Codex)
pub struct CodexConnector {
    #[allow(dead_code)]
    session_id: Option<String>,
    input_tx: Option<mpsc::Sender<String>>,
    health_status: HealthStatus,
    health_tx: watch::Sender<HealthStatus>,
    health_rx: watch::Receiver<HealthStatus>,
}

impl CodexConnector {
    pub fn new() -> Self {
        let (health_tx, health_rx) = watch::channel(HealthStatus::Starting);
        Self {
            session_id: None,
            input_tx: None,
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

fn codex_cli_bin() -> String {
    std::env::var("TRIUMVIRATE_CODEX_BIN").unwrap_or_else(|_| "codex".to_string())
}

#[async_trait::async_trait]
impl AgentConnector for CodexConnector {
    fn agent_id(&self) -> AgentId {
        AgentId::Codex
    }

    async fn spawn(&mut self, bus: Arc<MessageBus>) -> anyhow::Result<()> {
        let session_id = Uuid::new_v4().to_string();
        self.session_id = Some(session_id.clone());

        let codex_bin = codex_cli_bin();
        let mut child = Command::new(&codex_bin)
            .arg("mcp-server")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("codex stdin not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("codex stdout not piped"))?;

        let (input_tx, mut input_rx) = mpsc::channel::<String>(256);
        self.input_tx = Some(input_tx.clone());

        let mut codex_input_rx = bus.subscribe(&Topic::AgentInput(AgentId::Codex)).await;
        tokio::spawn(async move {
            while let Ok(msg) = codex_input_rx.recv().await {
                if let Payload::HumanMessage { content } = msg.payload {
                    let _ = input_tx.send(content).await;
                }
            }
        });

        tokio::spawn(async move {
            let mut writer = stdin;
            let mut req_id: u64 = 1;

            while let Some(message) = input_rx.recv().await {
                let payload = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "method": "codex-reply",
                    "params": {
                        "message": message
                    }
                });
                req_id = req_id.saturating_add(1);

                let line = format!("{payload}\n");
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if writer.flush().await.is_err() {
                    break;
                }
            }
        });

        let reader_bus = bus.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => match parse_codex_event(&line) {
                        Ok(Some(event)) => {
                            if let Some(text) = event.text_content() {
                                reader_bus
                                    .emit(FabricMessage::new(
                                        AgentId::Codex,
                                        Topic::AgentOutput(AgentId::Codex),
                                        Payload::TextChunk {
                                            content: text,
                                            is_final: matches!(
                                                event.kind,
                                                CodexEventKind::Response
                                            ),
                                        },
                                    ))
                                    .await;
                            }

                            if matches!(event.kind, CodexEventKind::Error) {
                                let message = event
                                    .error_message()
                                    .unwrap_or_else(|| "codex mcp-server error".to_string());
                                reader_bus
                                    .emit(FabricMessage::new(
                                        AgentId::System,
                                        Topic::SystemError,
                                        Payload::Error {
                                            message,
                                            source_agent: Some(AgentId::Codex),
                                        },
                                    ))
                                    .await;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            reader_bus
                                .emit(FabricMessage::new(
                                    AgentId::System,
                                    Topic::SystemError,
                                    Payload::Error {
                                        message: format!("failed to parse codex json: {e}"),
                                        source_agent: Some(AgentId::Codex),
                                    },
                                ))
                                .await;
                        }
                    },
                    Ok(None) => break,
                    Err(e) => {
                        reader_bus
                            .emit(FabricMessage::new(
                                AgentId::System,
                                Topic::SystemError,
                                Payload::Error {
                                    message: format!("error reading codex stdout: {e}"),
                                    source_agent: Some(AgentId::Codex),
                                },
                            ))
                            .await;
                        break;
                    }
                }
            }
        });

        let monitor_tx = self.health_tx.clone();
        tokio::spawn(async move {
            let status = child.wait().await;
            let _ = monitor_tx.send(HealthStatus::Dead);
            if let Err(e) = status {
                warn!(agent = "codex", error = %e, "codex process wait failed");
            }
        });

        info!(agent = "codex", session = %session_id, cli = %codex_bin, "spawned persistent mcp-server session");
        self.set_health(HealthStatus::Ready);
        bus.emit(FabricMessage::new(
            AgentId::System,
            Topic::SystemHealth,
            Payload::HealthChange {
                agent: AgentId::Codex,
                status: HealthStatus::Ready,
                detail: Some("codex connector started".to_string()),
            },
        ))
        .await;

        Ok(())
    }

    async fn send(&self, message: &str) -> anyhow::Result<()> {
        if let Some(tx) = &self.input_tx {
            tx.send(message.to_string()).await?;
            return Ok(());
        }
        Err(anyhow::anyhow!("codex connector is not running"))
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

#[cfg(test)]
mod tests {
    use super::codex_cli_bin;

    #[test]
    fn defaults_codex_bin() {
        // SAFETY: test process controls this env var lifecycle.
        unsafe { std::env::remove_var("TRIUMVIRATE_CODEX_BIN") };
        assert_eq!(codex_cli_bin(), "codex");
    }

    #[test]
    fn honors_codex_bin_override() {
        // SAFETY: test process controls this env var lifecycle.
        unsafe { std::env::set_var("TRIUMVIRATE_CODEX_BIN", "/tmp/mock-codex") };
        assert_eq!(codex_cli_bin(), "/tmp/mock-codex");
        // SAFETY: test process controls this env var lifecycle.
        unsafe { std::env::remove_var("TRIUMVIRATE_CODEX_BIN") };
    }
}
