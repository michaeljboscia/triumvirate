use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tracing::{info, warn};
use triumvirate_proto::{
    AgentId, FabricMessage, GeminiEventKind, HealthStatus, Payload, Topic, parse_gemini_event,
};
use uuid::Uuid;

use super::connector::AgentConnector;
use crate::fabric::MessageBus;

/// Gemini CLI connector.
///
/// Integration path: `gemini --acp`
/// - Persistent subprocess over stdio
/// - JSON-RPC protocol (request/response + notifications)
/// - Input routed from Topic::AgentInput(Gemini)
pub struct GeminiConnector {
    #[allow(dead_code)]
    session_id: Option<String>,
    input_tx: Option<mpsc::Sender<String>>,
    health_status: HealthStatus,
    health_tx: watch::Sender<HealthStatus>,
    health_rx: watch::Receiver<HealthStatus>,
}

impl GeminiConnector {
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

#[async_trait::async_trait]
impl AgentConnector for GeminiConnector {
    fn agent_id(&self) -> AgentId {
        AgentId::Gemini
    }

    async fn spawn(&mut self, bus: Arc<MessageBus>) -> anyhow::Result<()> {
        let session_id = Uuid::new_v4().to_string();
        self.session_id = Some(session_id.clone());

        let gemini_bin = std::env::var("TRIUMVIRATE_GEMINI_BIN")
            .unwrap_or_else(|_| "gemini".to_string());
        let mut child = Command::new(&gemini_bin)
            .arg("--acp")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("gemini stdin not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("gemini stdout not piped"))?;

        let (input_tx, mut input_rx) = mpsc::channel::<String>(256);
        self.input_tx = Some(input_tx.clone());

        let mut gemini_input_rx = bus.subscribe(&Topic::AgentInput(AgentId::Gemini)).await;
        tokio::spawn(async move {
            while let Ok(msg) = gemini_input_rx.recv().await {
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
                    "method": "message/send",
                    "params": {
                        "content": message
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
                    Ok(Some(line)) => match parse_gemini_event(&line) {
                        Ok(Some(event)) => {
                            if let Some(text) = event.text_content() {
                                reader_bus
                                    .emit(FabricMessage::new(
                                        AgentId::Gemini,
                                        Topic::AgentOutput(AgentId::Gemini),
                                        Payload::TextChunk {
                                            content: text,
                                            is_final: matches!(
                                                event.kind,
                                                GeminiEventKind::Response
                                            ),
                                        },
                                    ))
                                    .await;
                            }

                            if matches!(event.kind, GeminiEventKind::Error) {
                                let message = event
                                    .error_message()
                                    .unwrap_or_else(|| "gemini acp error".to_string());
                                reader_bus
                                    .emit(FabricMessage::new(
                                        AgentId::System,
                                        Topic::SystemError,
                                        Payload::Error {
                                            message,
                                            source_agent: Some(AgentId::Gemini),
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
                                        message: format!("failed to parse gemini json: {e}"),
                                        source_agent: Some(AgentId::Gemini),
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
                                    message: format!("error reading gemini stdout: {e}"),
                                    source_agent: Some(AgentId::Gemini),
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
                warn!(agent = "gemini", error = %e, "gemini process wait failed");
            }
        });

        info!(agent = "gemini", session = %session_id, cli = %gemini_bin, "spawned persistent ACP session");
        self.set_health(HealthStatus::Ready);
        bus.emit(FabricMessage::new(
            AgentId::System,
            Topic::SystemHealth,
            Payload::HealthChange {
                agent: AgentId::Gemini,
                status: HealthStatus::Ready,
                detail: Some("gemini connector started".to_string()),
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
        Err(anyhow::anyhow!("gemini connector is not running"))
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
