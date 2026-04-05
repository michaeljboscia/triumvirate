use std::sync::Arc;

use tracing::info;
use triumvirate_proto::{AgentId, FabricMessage, Payload, Topic};

use crate::fabric::MessageBus;
use crate::quota::SharedQuotaRegistry;

const DIGEST_PREFIX: &str = "[DIGEST]";
const DIGEST_FALLBACK_THRESHOLD_PCT: f64 = 80.0;

/// Mechanical digest fan-out for idle agents.
///
/// Per FEAT-018 and REQ-2 constraints, this module does not summarize with an LLM.
/// It forwards structured, minimal templates derived directly from fabric payloads.
pub struct DigestEngine {
    bus: Arc<MessageBus>,
    quota: SharedQuotaRegistry,
}

impl DigestEngine {
    pub fn new(bus: Arc<MessageBus>, quota: SharedQuotaRegistry) -> Self {
        Self { bus, quota }
    }

    pub fn run(self) {
        tokio::spawn(async move {
            let mut rx = self.bus.subscribe_all().await;

            while let Ok(msg) = rx.recv().await {
                if let Some((source, content)) = digest_candidate(&msg) {
                    for target in [AgentId::Claude, AgentId::Gemini, AgentId::Codex] {
                        if target == source {
                            continue;
                        }

                        if self
                            .quota
                            .is_over_threshold(target, DIGEST_FALLBACK_THRESHOLD_PCT)
                            .await
                        {
                            self.bus
                                .emit(FabricMessage::new(
                                    AgentId::System,
                                    Topic::TaskProgress,
                                    Payload::TextChunk {
                                        content: format!(
                                            "digest skipped: quota threshold reached for {target}"
                                        ),
                                        is_final: true,
                                    },
                                ))
                                .await;
                            continue;
                        }

                        let digest = build_digest_message(source, msg.id.to_string().as_str(), &content);
                        self.bus
                            .emit(FabricMessage::new(
                                AgentId::System,
                                Topic::AgentInput(target),
                                Payload::HumanMessage { content: digest },
                            ))
                            .await;
                    }

                    info!(source = %source, message_id = %msg.id, "digest fan-out emitted");
                }
            }
        });
    }
}

fn digest_candidate(msg: &FabricMessage) -> Option<(AgentId, String)> {
    let source = match msg.topic {
        Topic::AgentOutput(agent) => agent,
        _ => return None,
    };

    let content = match &msg.payload {
        Payload::AgentResponse { content, .. } => content.clone(),
        Payload::TextChunk {
            content,
            is_final: true,
        } => content.clone(),
        _ => return None,
    };

    if content.trim().is_empty() || content.starts_with(DIGEST_PREFIX) {
        return None;
    }

    Some((source, content))
}

fn build_digest_message(source: AgentId, event_id: &str, content: &str) -> String {
    format!(
        "{DIGEST_PREFIX} source={source} event_id={event_id} raw_output={content}\nRespond only if you have a material correction.",
    )
}

#[cfg(test)]
mod tests {
    use super::{build_digest_message, digest_candidate};
    use triumvirate_proto::{AgentId, FabricMessage, Payload, Topic};

    #[test]
    fn candidate_detects_final_agent_output() {
        let msg = FabricMessage::new(
            AgentId::Codex,
            Topic::AgentOutput(AgentId::Codex),
            Payload::TextChunk {
                content: "done".to_string(),
                is_final: true,
            },
        );

        let candidate = digest_candidate(&msg);
        assert!(candidate.is_some());
    }

    #[test]
    fn candidate_ignores_non_final_chunks() {
        let msg = FabricMessage::new(
            AgentId::Claude,
            Topic::AgentOutput(AgentId::Claude),
            Payload::TextChunk {
                content: "partial".to_string(),
                is_final: false,
            },
        );

        let candidate = digest_candidate(&msg);
        assert!(candidate.is_none());
    }

    #[test]
    fn digest_includes_event_id() {
        let digest = build_digest_message(AgentId::Gemini, "abc-123", "raw text");
        assert!(digest.contains("event_id=abc-123"));
        assert!(digest.contains("raw_output=raw text"));
    }
}
