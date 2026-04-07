use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use triumvirate_proto::{AgentId, HealthStatus};

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
