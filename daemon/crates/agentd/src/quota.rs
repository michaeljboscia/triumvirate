use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;
use tracing::info;
use triumvirate_proto::{AgentId, Payload, Topic};
use uuid::Uuid;

use crate::cost::{PricingConfig, estimate_cost_usd};
use crate::fabric::MessageBus;
use crate::langfuse::LangfuseClient;
use crate::metrics::SharedMetricsRegistry;

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
    metrics: SharedMetricsRegistry,
    langfuse: LangfuseClient,
    session_id: Uuid,
    pricing: PricingConfig,
}

impl QuotaTracker {
    pub fn new(
        bus: Arc<MessageBus>,
        registry: SharedQuotaRegistry,
        metrics: SharedMetricsRegistry,
        langfuse: LangfuseClient,
        session_id: Uuid,
        pricing: PricingConfig,
    ) -> Self {
        Self {
            bus,
            registry,
            metrics,
            langfuse,
            session_id,
            pricing,
        }
    }

    pub fn run(self) {
        tokio::spawn(async move {
            let mut rx = self.bus.subscribe_all().await;
            let mut in_flight: HashMap<AgentId, (Instant, u64)> = HashMap::new();
            while let Ok(msg) = rx.recv().await {
                match msg.topic {
                    Topic::AgentInput(agent) => {
                        if let Payload::HumanMessage { content } = msg.payload {
                            in_flight.insert(agent, (Instant::now(), estimate_tokens(&content)));
                        }
                    }
                    Topic::AgentOutput(agent) => {
                        let content = match msg.payload {
                            Payload::TextChunk {
                                ref content,
                                is_final: true,
                            } => Some(content.as_str()),
                            Payload::AgentResponse { ref content, .. } => Some(content.as_str()),
                            Payload::Error { .. } => {
                                self.metrics.record_error(agent).await;
                                None
                            }
                            _ => None,
                        };

                        if let Some(content) = content {
                            let output_tokens = estimate_tokens(content);
                            self.registry.add_usage(agent, output_tokens).await;

                            let (started_at, input_tokens) = in_flight
                                .remove(&agent)
                                .unwrap_or((Instant::now(), 0));
                            let duration_secs = started_at.elapsed().as_secs_f64();
                            self.metrics
                                .observe_turn(agent, input_tokens, output_tokens, duration_secs)
                                .await;
                            let estimated_cost_usd =
                                estimate_cost_usd(agent, input_tokens, output_tokens, &self.pricing);
                            self.langfuse.record_turn(
                                &self.session_id.to_string(),
                                agent,
                                input_tokens,
                                output_tokens,
                                estimated_cost_usd,
                                (duration_secs * 1_000.0) as u64,
                            );
                            info!(agent = %agent, estimated_tokens = output_tokens, "quota usage updated");
                        }
                    }
                    _ => {}
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
