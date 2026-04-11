use std::any::Any;
use crate::metrics::DaemonMetrics;
use shared_types::AgentStreamEvent;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct ObservabilityBus {
    pub metrics: Arc<DaemonMetrics>,
    pub ws_events: broadcast::Sender<String>,
    pub token_db: Arc<dyn Any + Send + Sync>,
}

impl ObservabilityBus {
    pub fn new(
        metrics: Arc<DaemonMetrics>,
        ws_events: broadcast::Sender<String>,
        token_db: Arc<dyn Any + Send + Sync>,
    ) -> Self {
        Self {
            metrics,
            ws_events,
            token_db,
        }
    }

    pub fn publish_event(&self, event_type: &str, payload: serde_json::Value) {
        let msg = serde_json::json!({
            "type": event_type,
            "ts_ms": crate::unix_time_ms(),
            "payload": payload,
        })
        .to_string();
        let _ = self.ws_events.send(msg);
    }

    /// Publish an AgentStreamEvent to the WebSocket broadcast.
    /// Emitted as type "agent_stream" alongside existing event types.
    /// REQ-E04
    pub fn publish_agent_stream_event(&self, event: &AgentStreamEvent) {
        let payload = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to serialize AgentStreamEvent: {e}");
                return;
            }
        };
        self.publish_event("agent_stream", payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn observability_bus_clone_send_sync_round_trip() {
        let metrics = Arc::new(crate::metrics::DaemonMetrics::new().unwrap());
        let (tx, mut rx) = broadcast::channel::<String>(16);
        let bus = ObservabilityBus::new(metrics.clone(), tx, Arc::new(()));

        let bus_a = bus.clone();
        let bus_b = bus.clone();

        let h1 = tokio::spawn(async move {
            bus_a.publish_event("test", serde_json::json!({ "n": 1 }));
            bus_a.metrics.agent_requests_total.inc();
        });
        let h2 = tokio::spawn(async move {
            bus_b.publish_event("test", serde_json::json!({ "n": 2 }));
            bus_b.metrics.agent_requests_total.inc();
        });

        h1.await.unwrap();
        h2.await.unwrap();

        let msg_a = rx.recv().await.unwrap();
        let msg_b = rx.recv().await.unwrap();

        let parsed_a: serde_json::Value = serde_json::from_str(&msg_a).unwrap();
        let parsed_b: serde_json::Value = serde_json::from_str(&msg_b).unwrap();

        let n_a = parsed_a["payload"]["n"].as_i64().unwrap();
        let n_b = parsed_b["payload"]["n"].as_i64().unwrap();

        let mut ns = [n_a, n_b];
        ns.sort();
        assert_eq!(ns, [1, 2]);

        assert_eq!(metrics.agent_requests_total.get(), 2);
    }
}
