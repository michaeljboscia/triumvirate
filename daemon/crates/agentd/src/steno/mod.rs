use std::sync::Arc;

use tracing::info;
use triumvirate_proto::FabricMessage;

use crate::fabric::MessageBus;

/// Stenographer — mechanical extraction of session facts from the fabric.
///
/// Per REQ-2: NO LLM summarization. Subscribes to all NATS/fabric topics
/// and builds a structured log from:
/// - Agent messages (NATS streams)
/// - Git diffs (session start vs end)
/// - Tool invocations and results
/// - Syntax-gated decisions (# DECISION: + # VALIDATE:)
///
/// POC 1: Just logs messages to tracing. Week 2 adds structured JSON output.
pub struct Stenographer {
    bus: Arc<MessageBus>,
}

impl Stenographer {
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self { bus }
    }

    /// Start consuming fabric messages in a background task.
    pub fn run(self) {
        tokio::spawn(async move {
            let mut rx = self.bus.subscribe_all().await;
            info!("stenographer started — listening to all fabric topics");

            loop {
                match rx.recv().await {
                    Ok(msg) => self.handle_message(&msg),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "stenographer lagged — missed messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("stenographer shutting down — fabric closed");
                        break;
                    }
                }
            }
        });
    }

    fn handle_message(&self, msg: &FabricMessage) {
        info!(
            id = %msg.id,
            source = %msg.source,
            topic = ?msg.topic,
            "steno: fabric event"
        );
    }
}
