use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use triumvirate_proto::{FabricMessage, Topic};

/// In-process message bus backed by tokio broadcast channels.
///
/// Each topic gets its own broadcast channel. Subscribers receive all messages
/// on topics they subscribe to. When we add NATS, this becomes a thin wrapper
/// around async-nats publish/subscribe — the Topic enum already maps 1:1.
///
/// Capacity per channel: 256 messages. If a slow consumer falls behind,
/// it receives a `RecvError::Lagged` and can catch up from the next message.
const CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct MessageBus {
    channels: Arc<RwLock<HashMap<TopicKey, broadcast::Sender<FabricMessage>>>>,
}

/// Flattened topic key for HashMap lookup.
/// Topic::AgentOutput(Claude) becomes "agent_output:claude", etc.
type TopicKey = String;

fn topic_key(topic: &Topic) -> TopicKey {
    match topic {
        Topic::AgentInput(agent) => format!("agent_input:{agent}"),
        Topic::AgentOutput(agent) => format!("agent_output:{agent}"),
        other => serde_json::to_string(other)
            .unwrap_or_else(|_| format!("{other:?}")),
    }
}

impl MessageBus {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Publish a message to its topic. Creates the channel on first publish.
    pub async fn publish(&self, msg: FabricMessage) {
        let key = topic_key(&msg.topic);
        let channels = self.channels.read().await;

        if let Some(tx) = channels.get(&key) {
            // Ignore send errors — means no active subscribers
            let _ = tx.send(msg);
        } else {
            drop(channels);
            let mut channels = self.channels.write().await;
            let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            let _ = tx.send(msg);
            channels.insert(key, tx);
        }
    }

    /// Subscribe to a topic. Returns a receiver that yields all future messages.
    #[allow(dead_code)]
    pub async fn subscribe(
        &self,
        topic: &Topic,
    ) -> broadcast::Receiver<FabricMessage> {
        let key = topic_key(topic);
        let channels = self.channels.read().await;

        if let Some(tx) = channels.get(&key) {
            tx.subscribe()
        } else {
            drop(channels);
            let mut channels = self.channels.write().await;
            let (tx, rx) = broadcast::channel(CHANNEL_CAPACITY);
            channels.insert(key, tx);
            rx
        }
    }

    /// Subscribe to ALL messages across all topics. Useful for the stenographer
    /// and the web dashboard event stream.
    pub async fn subscribe_all(&self) -> broadcast::Receiver<FabricMessage> {
        // The "firehose" is a special internal topic
        let key = "__firehose__".to_string();
        let channels = self.channels.read().await;

        if let Some(tx) = channels.get(&key) {
            tx.subscribe()
        } else {
            drop(channels);
            let mut channels = self.channels.write().await;
            let (tx, rx) = broadcast::channel(CHANNEL_CAPACITY * 4);
            channels.insert(key, tx);
            rx
        }
    }

    /// Publish to both the topic channel AND the firehose.
    /// This is the primary publish method for production use.
    pub async fn emit(&self, msg: FabricMessage) {
        // Publish to specific topic
        self.publish(msg.clone()).await;

        // Also publish to firehose for global subscribers
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get("__firehose__") {
            let _ = tx.send(msg);
        }
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new()
    }
}
