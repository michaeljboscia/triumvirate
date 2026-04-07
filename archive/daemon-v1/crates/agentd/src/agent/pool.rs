#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use triumvirate_proto::AgentId;

#[derive(Debug, Clone)]
pub struct AgentInstance {
    pub instance_id: String,
    pub agent: AgentId,
}

#[derive(Clone, Default)]
pub struct AgentPool {
    inner: Arc<Mutex<PoolState>>,
}

#[derive(Default)]
struct PoolState {
    by_agent: HashMap<AgentId, Vec<AgentInstance>>,
    cursors: HashMap<AgentId, usize>,
}

impl AgentPool {
    pub async fn register_instance(&self, instance: AgentInstance) {
        let mut state = self.inner.lock().await;
        state
            .by_agent
            .entry(instance.agent)
            .or_default()
            .push(instance);
    }

    pub async fn list_instances(&self, agent: AgentId) -> Vec<AgentInstance> {
        let state = self.inner.lock().await;
        state.by_agent.get(&agent).cloned().unwrap_or_default()
    }

    /// Pick the next instance of an agent in round-robin order.
    pub async fn next_instance(&self, agent: AgentId) -> Option<AgentInstance> {
        let mut state = self.inner.lock().await;
        let len = state.by_agent.get(&agent)?.len();
        if len == 0 {
            return None;
        }
        let idx = *state.cursors.get(&agent).unwrap_or(&0);
        let selected = {
            let instances = state.by_agent.get(&agent)?;
            instances[idx % len].clone()
        };
        state.cursors.insert(agent, (idx + 1) % len);
        Some(selected)
    }

    pub async fn find_instance(&self, agent: AgentId, instance_id: &str) -> Option<AgentInstance> {
        let state = self.inner.lock().await;
        state
            .by_agent
            .get(&agent)?
            .iter()
            .find(|i| i.instance_id == instance_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentInstance, AgentPool};
    use triumvirate_proto::AgentId;

    #[tokio::test]
    async fn round_robin_cycles_instances() {
        let pool = AgentPool::default();
        pool.register_instance(AgentInstance {
            instance_id: "codex-1".to_string(),
            agent: AgentId::Codex,
        })
        .await;
        pool.register_instance(AgentInstance {
            instance_id: "codex-2".to_string(),
            agent: AgentId::Codex,
        })
        .await;

        let a = pool.next_instance(AgentId::Codex).await.expect("first");
        let b = pool.next_instance(AgentId::Codex).await.expect("second");
        let c = pool.next_instance(AgentId::Codex).await.expect("third");

        assert_eq!(a.instance_id, "codex-1");
        assert_eq!(b.instance_id, "codex-2");
        assert_eq!(c.instance_id, "codex-1");
    }
}
