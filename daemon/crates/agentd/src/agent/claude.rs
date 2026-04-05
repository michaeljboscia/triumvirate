use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tracing::{info, warn};
use triumvirate_proto::{
    parse_claude_event, AgentId, ClaudeEventKind, FabricMessage, HealthStatus, Payload, Topic,
};
use uuid::Uuid;

use super::connector::AgentConnector;
use crate::fabric::MessageBus;

/// Claude CLI connector.
///
/// Integration path: `claude -p --output-format stream-json`
/// - Single-shot JSONL streaming (each invocation is one turn)
/// - Session continuity is preserved via `--session-id`
pub struct ClaudeConnector {
    #[allow(dead_code)]
    session_id: Option<String>,
    input_tx: Option<mpsc::Sender<String>>,
    health_status: HealthStatus,
    health_tx: watch::Sender<HealthStatus>,
    health_rx: watch::Receiver<HealthStatus>,
}

impl ClaudeConnector {
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

fn claude_cli_bin() -> String {
    std::env::var("TRIUMVIRATE_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string())
}

#[async_trait::async_trait]
impl AgentConnector for ClaudeConnector {
    fn agent_id(&self) -> AgentId {
        AgentId::Claude
    }

    async fn spawn(&mut self, bus: Arc<MessageBus>) -> anyhow::Result<()> {
        let session_id = Uuid::new_v4().to_string();
        self.session_id = Some(session_id.clone());
        let claude_bin = claude_cli_bin();

        let (input_tx, mut input_rx) = mpsc::channel::<String>(64);
        self.input_tx = Some(input_tx.clone());

        let mut human_rx = bus.subscribe(&Topic::AgentInput(AgentId::Claude)).await;
        tokio::spawn(async move {
            while let Ok(msg) = human_rx.recv().await {
                if let Payload::HumanMessage { content } = msg.payload {
                    let _ = input_tx.send(content).await;
                }
            }
        });

        let worker_bus = bus.clone();
        let worker_claude_bin = claude_bin.clone();
        let worker_session_id = session_id.clone();
        tokio::spawn(async move {
            while let Some(message) = input_rx.recv().await {
                let mut child = match Command::new(&worker_claude_bin)
                    .arg("-p")
                    .arg("--output-format")
                    .arg("stream-json")
                    .arg("--bare")
                    .arg("--dangerously-skip-permissions")
                    .arg("--session-id")
                    .arg(&worker_session_id)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                {
                    Ok(child) => child,
                    Err(e) => {
                        worker_bus
                            .emit(FabricMessage::new(
                                AgentId::System,
                                Topic::SystemError,
                                Payload::Error {
                                    message: format!("failed to spawn claude process: {e}"),
                                    source_agent: Some(AgentId::Claude),
                                },
                            ))
                            .await;
                        continue;
                    }
                };

                let Some(mut stdin) = child.stdin.take() else {
                    worker_bus
                        .emit(FabricMessage::new(
                            AgentId::System,
                            Topic::SystemError,
                            Payload::Error {
                                message: "claude stdin not piped".to_string(),
                                source_agent: Some(AgentId::Claude),
                            },
                        ))
                        .await;
                    continue;
                };
                let Some(stdout) = child.stdout.take() else {
                    worker_bus
                        .emit(FabricMessage::new(
                            AgentId::System,
                            Topic::SystemError,
                            Payload::Error {
                                message: "claude stdout not piped".to_string(),
                                source_agent: Some(AgentId::Claude),
                            },
                        ))
                        .await;
                    continue;
                };
                let Some(stderr) = child.stderr.take() else {
                    worker_bus
                        .emit(FabricMessage::new(
                            AgentId::System,
                            Topic::SystemError,
                            Payload::Error {
                                message: "claude stderr not piped".to_string(),
                                source_agent: Some(AgentId::Claude),
                            },
                        ))
                        .await;
                    continue;
                };

                let prompt = format!("{message}\n");
                if let Err(e) = stdin.write_all(prompt.as_bytes()).await {
                    worker_bus
                        .emit(FabricMessage::new(
                            AgentId::System,
                            Topic::SystemError,
                            Payload::Error {
                                message: format!("failed writing claude stdin: {e}"),
                                source_agent: Some(AgentId::Claude),
                            },
                        ))
                        .await;
                    continue;
                }
                let _ = stdin.flush().await;
                drop(stdin);

                let stderr_bus = worker_bus.clone();
                tokio::spawn(async move {
                    let mut stderr_lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = stderr_lines.next_line().await {
                        warn!(agent = "claude", stderr = %line, "claude stderr");
                        stderr_bus
                            .emit(FabricMessage::new(
                                AgentId::System,
                                Topic::SystemError,
                                Payload::Error {
                                    message: format!("claude stderr: {line}"),
                                    source_agent: Some(AgentId::Claude),
                                },
                            ))
                            .await;
                    }
                });

                let mut lines = BufReader::new(stdout).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => match parse_claude_event(&line) {
                            Ok(Some(event)) => {
                                if let Some(text) = event.text_content() {
                                    let payload = if matches!(event.kind, ClaudeEventKind::Result)
                                    {
                                        Payload::AgentResponse {
                                            content: text,
                                            tokens_used: None,
                                        }
                                    } else {
                                        Payload::TextChunk {
                                            content: text,
                                            is_final: false,
                                        }
                                    };
                                    worker_bus
                                        .emit(FabricMessage::new(
                                            AgentId::Claude,
                                            Topic::AgentOutput(AgentId::Claude),
                                            payload,
                                        ))
                                        .await;
                                }

                                if matches!(event.kind, ClaudeEventKind::Error) {
                                    let message = event
                                        .error_message()
                                        .unwrap_or_else(|| "claude stream error".to_string());
                                    worker_bus
                                        .emit(FabricMessage::new(
                                            AgentId::System,
                                            Topic::SystemError,
                                            Payload::Error {
                                                message,
                                                source_agent: Some(AgentId::Claude),
                                            },
                                        ))
                                        .await;
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                worker_bus
                                    .emit(FabricMessage::new(
                                        AgentId::System,
                                        Topic::SystemError,
                                        Payload::Error {
                                            message: format!("failed to parse claude jsonl: {e}"),
                                            source_agent: Some(AgentId::Claude),
                                        },
                                    ))
                                    .await;
                            }
                        },
                        Ok(None) => break,
                        Err(e) => {
                            worker_bus
                                .emit(FabricMessage::new(
                                    AgentId::System,
                                    Topic::SystemError,
                                    Payload::Error {
                                        message: format!("error reading claude stdout: {e}"),
                                        source_agent: Some(AgentId::Claude),
                                    },
                                ))
                                .await;
                            break;
                        }
                    }
                }

                match child.wait().await {
                    Ok(status) => {
                        if !status.success() {
                            worker_bus
                                .emit(FabricMessage::new(
                                    AgentId::System,
                                    Topic::SystemError,
                                    Payload::Error {
                                        message: format!(
                                            "claude process exited with status: {}",
                                            status
                                        ),
                                        source_agent: Some(AgentId::Claude),
                                    },
                                ))
                                .await;
                        }
                    }
                    Err(e) => {
                        worker_bus
                            .emit(FabricMessage::new(
                                AgentId::System,
                                Topic::SystemError,
                                Payload::Error {
                                    message: format!("claude process wait failed: {e}"),
                                    source_agent: Some(AgentId::Claude),
                                },
                            ))
                            .await;
                    }
                }
            }
        });

        info!(agent = "claude", session = %session_id, cli = %claude_bin, "ready for per-turn claude invocations");
        self.set_health(HealthStatus::Ready);
        bus.emit(FabricMessage::new(
            AgentId::System,
            Topic::SystemHealth,
            Payload::HealthChange {
                agent: AgentId::Claude,
                status: HealthStatus::Ready,
                detail: Some("claude connector started".to_string()),
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
        Err(anyhow::anyhow!("claude connector is not running"))
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

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::claude_cli_bin;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn defaults_claude_bin() {
        let _guard = env_lock().lock().expect("lock poisoned");
        // SAFETY: test process controls this env var lifecycle.
        unsafe { std::env::remove_var("TRIUMVIRATE_CLAUDE_BIN") };
        assert_eq!(claude_cli_bin(), "claude");
    }

    #[test]
    fn honors_claude_bin_override() {
        let _guard = env_lock().lock().expect("lock poisoned");
        // SAFETY: test process controls this env var lifecycle.
        unsafe { std::env::set_var("TRIUMVIRATE_CLAUDE_BIN", "/tmp/mock-claude") };
        assert_eq!(claude_cli_bin(), "/tmp/mock-claude");
        // SAFETY: test process controls this env var lifecycle.
        unsafe { std::env::remove_var("TRIUMVIRATE_CLAUDE_BIN") };
    }
}
