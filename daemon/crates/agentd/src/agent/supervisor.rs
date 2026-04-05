use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;
use tracing::{info, warn};
use triumvirate_proto::{AgentId, FabricMessage, HealthStatus, Payload, Topic};
use triumvirate_workflow::next_backoff_ms;

use super::{AgentConnector, ClaudeConnector, CodexConnector, GeminiConnector, SharedHealthRegistry};
use crate::fabric::MessageBus;

pub fn spawn_claude_supervisor(bus: Arc<MessageBus>, registry: SharedHealthRegistry) {
    tokio::spawn(run_claude_supervisor(bus, registry));
}

pub fn spawn_gemini_supervisor(bus: Arc<MessageBus>, registry: SharedHealthRegistry) {
    tokio::spawn(run_gemini_supervisor(bus, registry));
}

pub fn spawn_codex_supervisor(bus: Arc<MessageBus>, registry: SharedHealthRegistry) {
    tokio::spawn(run_codex_supervisor(bus, registry));
}

async fn run_claude_supervisor(bus: Arc<MessageBus>, registry: SharedHealthRegistry) {
    run_supervisor_loop(
        bus,
        registry,
        AgentId::Claude,
        "claude",
        ClaudeConnector::new,
    )
    .await;
}

async fn run_gemini_supervisor(bus: Arc<MessageBus>, registry: SharedHealthRegistry) {
    run_supervisor_loop(
        bus,
        registry,
        AgentId::Gemini,
        "gemini",
        GeminiConnector::new,
    )
    .await;
}

async fn run_codex_supervisor(bus: Arc<MessageBus>, registry: SharedHealthRegistry) {
    run_supervisor_loop(
        bus,
        registry,
        AgentId::Codex,
        "codex",
        CodexConnector::new,
    )
    .await;
}

async fn run_supervisor_loop<C, F>(
    bus: Arc<MessageBus>,
    registry: SharedHealthRegistry,
    agent_id: AgentId,
    agent_name: &'static str,
    mut make_connector: F,
) where
    C: AgentConnector + 'static,
    F: FnMut() -> C,
{
    let mut attempt: u32 = 0;

    loop {
        emit_health(
            &bus,
            &registry,
            agent_id,
            if attempt == 0 {
                HealthStatus::Starting
            } else {
                HealthStatus::Restarting
            },
            Some(format!("{agent_name} supervisor attempt {}", attempt + 1)),
        )
        .await;

        let mut connector = make_connector();
        match connector.spawn(bus.clone()).await {
            Ok(()) => {
                attempt = 0;
                let mut health_rx = connector.health_watch();
                let mut prev = *health_rx.borrow();

                emit_health(
                    &bus,
                    &registry,
                    agent_id,
                    prev,
                    Some(format!("{agent_name} connector online")),
                )
                .await;

                while health_rx.changed().await.is_ok() {
                    let current = *health_rx.borrow();
                    if current != prev {
                        emit_health(&bus, &registry, agent_id, current, None).await;
                        prev = current;
                    }
                    if current == HealthStatus::Dead {
                        break;
                    }
                }

                warn!(agent = agent_name, "connector exited; scheduling restart");
            }
            Err(e) => {
                warn!(agent = agent_name, error = %e, "failed to spawn connector");
                emit_health(
                    &bus,
                    &registry,
                    agent_id,
                    HealthStatus::Dead,
                    Some(format!("{agent_name} spawn failed: {e}")),
                )
                .await;
            }
        }

        let backoff_ms = next_backoff_ms(1_000, attempt, 60_000);
        info!(
            agent = agent_name,
            attempt = attempt + 1,
            backoff_ms,
            "supervisor backoff before restart"
        );
        sleep(Duration::from_millis(backoff_ms)).await;
        attempt = attempt.saturating_add(1);
    }
}

async fn emit_health(
    bus: &Arc<MessageBus>,
    registry: &SharedHealthRegistry,
    agent: AgentId,
    status: HealthStatus,
    detail: Option<String>,
) {
    registry.set(agent, status).await;
    bus.emit(FabricMessage::new(
        AgentId::System,
        Topic::SystemHealth,
        Payload::HealthChange {
            agent,
            status,
            detail,
        },
    ))
    .await;
}
