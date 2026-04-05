use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::info;
use triumvirate_proto::{AgentId, Payload, Topic};

use crate::fabric::MessageBus;

const DEFAULT_AGENT_BUDGET: u64 = 100_000;

#[derive(Debug, Clone, serde::Serialize)]
pub struct QuotaSnapshot {
    pub used_tokens: u64,
    pub budget_tokens: u64,
    pub used_pct: f64,
}

#[derive(Clone, Default)]
pub struct SharedQuotaRegistry {
    inner: Arc<RwLock<HashMap<AgentId, u64>>>,
}

impl SharedQuotaRegistry {
    pub async fn add_usage(&self, agent: AgentId, tokens: u64) {
        let mut map = self.inner.write().await;
        let entry = map.entry(agent).or_insert(0);
        *entry = entry.saturating_add(tokens);
    }

    pub async fn used_tokens(&self, agent: AgentId) -> u64 {
        self.inner.read().await.get(&agent).copied().unwrap_or(0)
    }

    pub async fn is_over_threshold(&self, agent: AgentId, threshold_pct: f64) -> bool {
        let used = self.used_tokens(agent).await;
        let pct = (used as f64 / DEFAULT_AGENT_BUDGET as f64) * 100.0;
        pct >= threshold_pct
    }

    pub async fn snapshot_all(&self) -> HashMap<AgentId, QuotaSnapshot> {
        let map = self.inner.read().await;
        [AgentId::Claude, AgentId::Gemini, AgentId::Codex]
            .iter()
            .map(|agent| {
                let used = map.get(agent).copied().unwrap_or(0);
                let pct = (used as f64 / DEFAULT_AGENT_BUDGET as f64) * 100.0;
                (
                    *agent,
                    QuotaSnapshot {
                        used_tokens: used,
                        budget_tokens: DEFAULT_AGENT_BUDGET,
                        used_pct: pct,
                    },
                )
            })
            .collect()
    }
}

pub struct QuotaTracker {
    bus: Arc<MessageBus>,
    registry: SharedQuotaRegistry,
}

impl QuotaTracker {
    pub fn new(bus: Arc<MessageBus>, registry: SharedQuotaRegistry) -> Self {
        Self { bus, registry }
    }

    pub fn run(self) {
        tokio::spawn(async move {
            let mut rx = self.bus.subscribe_all().await;
            while let Ok(msg) = rx.recv().await {
                let source = match msg.topic {
                    Topic::AgentOutput(agent) => agent,
                    _ => continue,
                };

                let content = match msg.payload {
                    Payload::TextChunk {
                        ref content,
                        is_final: true,
                    } => Some(content.as_str()),
                    Payload::AgentResponse { ref content, .. } => Some(content.as_str()),
                    _ => None,
                };

                if let Some(content) = content {
                    let estimated_tokens = estimate_tokens(content);
                    self.registry.add_usage(source, estimated_tokens).await;
                    info!(agent = %source, estimated_tokens, "quota usage updated");
                }
            }
        });
    }
}

fn estimate_tokens(content: &str) -> u64 {
    // Cheap approximation to avoid per-turn model tokenization overhead.
    let chars = content.chars().count() as u64;
    (chars / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::estimate_tokens;

    #[test]
    fn estimate_tokens_has_minimum_one() {
        assert_eq!(estimate_tokens("a"), 1);
    }

    #[test]
    fn estimate_tokens_scales_with_content() {
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }
}
