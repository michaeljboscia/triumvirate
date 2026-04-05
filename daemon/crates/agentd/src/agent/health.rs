use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::watch;
use tokio::sync::RwLock;
use tracing::{info, warn};
use triumvirate_proto::{AgentId, FabricMessage, HealthStatus, Payload, Topic};

use crate::fabric::MessageBus;

/// Monitors agent health and publishes status changes to the fabric.
///
/// Each agent connector provides a watch::Receiver<HealthStatus>.
/// The monitor polls these and emits HealthChange events on the bus,
/// which the web dashboard and stenographer consume.
pub struct HealthMonitor {
    watchers: HashMap<AgentId, watch::Receiver<HealthStatus>>,
    bus: Arc<MessageBus>,
    registry: SharedHealthRegistry,
}

impl HealthMonitor {
    pub fn new(bus: Arc<MessageBus>, registry: SharedHealthRegistry) -> Self {
        Self {
            watchers: HashMap::new(),
            bus,
            registry,
        }
    }

    pub fn register(&mut self, agent: AgentId, rx: watch::Receiver<HealthStatus>) {
        self.watchers.insert(agent, rx);
    }

    /// Run the health monitor loop. Spawns a task per agent that watches
    /// for health transitions and emits fabric messages.
    pub fn run(self) {
        for (agent, mut rx) in self.watchers {
            let bus = self.bus.clone();
            let registry = self.registry.clone();
            tokio::spawn(async move {
                let mut prev = *rx.borrow();
                while rx.changed().await.is_ok() {
                    let current = *rx.borrow();
                    if current != prev {
                        match current {
                            HealthStatus::Ready => info!(%agent, "agent ready"),
                            HealthStatus::Dead => warn!(%agent, "agent dead"),
                            _ => info!(%agent, ?current, "health changed"),
                        }
                        registry.set(agent, current).await;

                        let msg = FabricMessage::new(
                            AgentId::System,
                            Topic::SystemHealth,
                            Payload::HealthChange {
                                agent,
                                status: current,
                                detail: None,
                            },
                        );
                        bus.emit(msg).await;
                        prev = current;
                    }
                }
            });
        }
    }
}

#[derive(Clone, Default)]
pub struct SharedHealthRegistry {
    inner: Arc<RwLock<HashMap<AgentId, HealthStatus>>>,
}

impl SharedHealthRegistry {
    pub async fn set(&self, agent: AgentId, status: HealthStatus) {
        let mut map = self.inner.write().await;
        map.insert(agent, status);
    }

    pub async fn snapshot(&self) -> HashMap<AgentId, HealthStatus> {
        self.inner.read().await.clone()
    }
}
