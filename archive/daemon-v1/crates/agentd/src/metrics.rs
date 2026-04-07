use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::RwLock;
use triumvirate_proto::{AgentId, HealthStatus};

use crate::quota::QuotaSnapshot;

const LATENCY_BUCKETS: [f64; 8] = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, f64::INFINITY];

#[derive(Default)]
struct AgentCounters {
    turns_total: AtomicU64,
    tokens_input_total: AtomicU64,
    tokens_output_total: AtomicU64,
    errors_total: AtomicU64,
    latency_sum_micros: AtomicU64,
    latency_bucket_counts: [AtomicU64; 8],
}

impl AgentCounters {
    fn observe_turn(&self, input_tokens: u64, output_tokens: u64, duration_secs: f64) {
        self.turns_total.fetch_add(1, Ordering::Relaxed);
        self.tokens_input_total
            .fetch_add(input_tokens, Ordering::Relaxed);
        self.tokens_output_total
            .fetch_add(output_tokens, Ordering::Relaxed);
        self.latency_sum_micros.fetch_add(
            (duration_secs * 1_000_000.0) as u64,
            Ordering::Relaxed,
        );

        for (idx, bound) in LATENCY_BUCKETS.iter().enumerate() {
            if duration_secs <= *bound {
                self.latency_bucket_counts[idx].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn record_error(&self) {
        self.errors_total.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Default)]
pub struct SharedMetricsRegistry {
    inner: Arc<RwLock<HashMap<AgentId, Arc<AgentCounters>>>>,
}

impl SharedMetricsRegistry {
    async fn for_agent(&self, agent: AgentId) -> Arc<AgentCounters> {
        let existing = {
            let map = self.inner.read().await;
            map.get(&agent).cloned()
        };
        if let Some(counters) = existing {
            return counters;
        }
        let mut map = self.inner.write().await;
        map.entry(agent)
            .or_insert_with(|| Arc::new(AgentCounters::default()))
            .clone()
    }

    pub async fn observe_turn(
        &self,
        agent: AgentId,
        input_tokens: u64,
        output_tokens: u64,
        duration_secs: f64,
    ) {
        let counters = self.for_agent(agent).await;
        counters.observe_turn(input_tokens, output_tokens, duration_secs);
    }

    pub async fn record_error(&self, agent: AgentId) {
        let counters = self.for_agent(agent).await;
        counters.record_error();
    }

    pub async fn render_prometheus(
        &self,
        fabric_messages_total: u64,
        health: HashMap<AgentId, HealthStatus>,
        quota: HashMap<AgentId, QuotaSnapshot>,
    ) -> String {
        let mut out = String::new();
        out.push_str("# HELP fabric_messages_total Total messages emitted on the fabric.\n");
        out.push_str("# TYPE fabric_messages_total counter\n");
        out.push_str(&format!("fabric_messages_total {}\n", fabric_messages_total));

        out.push_str("# HELP agent_active_connections Number of active agent connectors.\n");
        out.push_str("# TYPE agent_active_connections gauge\n");
        for agent in [AgentId::Claude, AgentId::Gemini, AgentId::Codex] {
            let status = health.get(&agent).copied().unwrap_or(HealthStatus::Starting);
            let active = matches!(
                status,
                HealthStatus::Ready | HealthStatus::Busy | HealthStatus::Restarting
            ) as u8;
            out.push_str(&format!(
                "agent_active_connections{{agent=\"{}\"}} {}\n",
                agent, active
            ));
        }

        out.push_str("# HELP quota_usage_percent Estimated quota utilization percent.\n");
        out.push_str("# TYPE quota_usage_percent gauge\n");
        for agent in [AgentId::Claude, AgentId::Gemini, AgentId::Codex] {
            let used_pct = quota.get(&agent).map(|q| q.used_pct).unwrap_or(0.0);
            out.push_str(&format!(
                "quota_usage_percent{{agent=\"{}\"}} {:.4}\n",
                agent, used_pct
            ));
        }

        out.push_str("# HELP agent_turns_total Completed turns observed by agent.\n");
        out.push_str("# TYPE agent_turns_total counter\n");
        out.push_str("# HELP agent_tokens_total Estimated token volume by direction.\n");
        out.push_str("# TYPE agent_tokens_total counter\n");
        out.push_str("# HELP agent_errors_total Error messages observed for an agent.\n");
        out.push_str("# TYPE agent_errors_total counter\n");
        out.push_str("# HELP agent_turn_duration_seconds Estimated turn latency histogram.\n");
        out.push_str("# TYPE agent_turn_duration_seconds histogram\n");

        let map = self.inner.read().await;
        for agent in [AgentId::Claude, AgentId::Gemini, AgentId::Codex] {
            let Some(counters) = map.get(&agent) else {
                continue;
            };

            let turns = counters.turns_total.load(Ordering::Relaxed);
            let input = counters.tokens_input_total.load(Ordering::Relaxed);
            let output = counters.tokens_output_total.load(Ordering::Relaxed);
            let errors = counters.errors_total.load(Ordering::Relaxed);
            let latency_sum = counters.latency_sum_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0;
            out.push_str(&format!("agent_turns_total{{agent=\"{}\"}} {}\n", agent, turns));
            out.push_str(&format!(
                "agent_tokens_total{{agent=\"{}\",direction=\"input\"}} {}\n",
                agent, input
            ));
            out.push_str(&format!(
                "agent_tokens_total{{agent=\"{}\",direction=\"output\"}} {}\n",
                agent, output
            ));
            out.push_str(&format!(
                "agent_errors_total{{agent=\"{}\"}} {}\n",
                agent, errors
            ));

            let mut cumulative = 0u64;
            for (idx, bound) in LATENCY_BUCKETS.iter().enumerate() {
                cumulative += counters.latency_bucket_counts[idx].load(Ordering::Relaxed);
                let bound_label = if bound.is_infinite() {
                    "+Inf".to_string()
                } else {
                    format!("{bound}")
                };
                out.push_str(&format!(
                    "agent_turn_duration_seconds_bucket{{agent=\"{}\",le=\"{}\"}} {}\n",
                    agent, bound_label, cumulative
                ));
            }
            out.push_str(&format!(
                "agent_turn_duration_seconds_sum{{agent=\"{}\"}} {}\n",
                agent, latency_sum
            ));
            out.push_str(&format!(
                "agent_turn_duration_seconds_count{{agent=\"{}\"}} {}\n",
                agent, turns
            ));
        }

        out
    }

    pub async fn snapshot_tokens(&self) -> HashMap<AgentId, (u64, u64, u64)> {
        let map = self.inner.read().await;
        let mut out = HashMap::new();
        for agent in [AgentId::Claude, AgentId::Gemini, AgentId::Codex] {
            if let Some(counters) = map.get(&agent) {
                out.insert(
                    agent,
                    (
                        counters.turns_total.load(Ordering::Relaxed),
                        counters.tokens_input_total.load(Ordering::Relaxed),
                        counters.tokens_output_total.load(Ordering::Relaxed),
                    ),
                );
            }
        }
        out
    }
}
