//! Structured events emitted during agent execution.
//!
//! Consumed by: WebSocket broadcast, watch CLI, future dashboard, SSE streaming.
//! Defined here in shared-types so all consumers use the same type.
//! FEAT-001 (REQ-E01, REQ-E02)

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Structured event emitted during agent execution.
///
/// Each variant carries a monotonic `seq` field for ordering and gap detection.
/// Serialized with `event_type` serde tag discriminator.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "event_type")]
pub enum AgentStreamEvent {
    /// Agent turn has started processing.
    TurnStarted {
        agent: String,
        session_name: String,
        seq: u64,
    },
    /// Agent invoked a tool.
    ToolCall {
        agent: String,
        tool_name: String,
        args_summary: String,
        seq: u64,
    },
    /// Agent read a file (convenience variant — subset of ToolCall).
    FileRead {
        agent: String,
        file_path: String,
        seq: u64,
    },
    /// Agent is generating a response (periodic during long generation).
    ResponseChunk {
        agent: String,
        text_preview: String,
        seq: u64,
    },
    /// Agent turn completed with token statistics.
    TurnCompleted {
        agent: String,
        tokens_in: i64,
        tokens_out: i64,
        cached_tokens: Option<i64>,
        tool_count: i64,
        duration_ms: u64,
        seq: u64,
    },
    /// Agent encountered an error.
    Error {
        agent: String,
        message: String,
        seq: u64,
    },
    /// Worker lifecycle event — spawned, completed, or failed.
    /// Carries lineage fields for hierarchical sidebar display.
    /// FEAT-014 (REQ-010)
    WorkerLifecycle {
        lifecycle: WorkerLifecycleType,
        agent: String,
        session_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        root_session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        commit_sha: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u64>,
        seq: u64,
    },
}

/// Sub-type for WorkerLifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerLifecycleType {
    /// Worker has been dispatched and is starting.
    Spawned,
    /// Worker completed successfully.
    Completed,
    /// Worker encountered a fatal error.
    Failed,
}

impl AgentStreamEvent {
    /// Format this event as a human-readable line for the watch CLI.
    /// Example: "→ Gemini: calling read_file (src/auth.rs)"
    pub fn display_text(&self) -> String {
        match self {
            Self::TurnStarted { agent, session_name, .. } => {
                format!("→ {agent}: turn started [{session_name}]")
            }
            Self::ToolCall { agent, tool_name, args_summary, .. } => {
                format!("→ {agent}: calling {tool_name} ({args_summary})")
            }
            Self::FileRead { agent, file_path, .. } => {
                format!("→ {agent}: reading {file_path}")
            }
            Self::ResponseChunk { agent, text_preview, .. } => {
                format!("→ {agent}: {text_preview}")
            }
            Self::TurnCompleted {
                agent,
                tokens_in,
                tokens_out,
                cached_tokens,
                tool_count,
                duration_ms,
                ..
            } => {
                let cached = cached_tokens
                    .map(|c| format!(" / {c} cached"))
                    .unwrap_or_default();
                let dur = *duration_ms as f64 / 1000.0;
                format!(
                    "→ {agent}: responded ({tokens_in} in / {tokens_out} out{cached}, {tool_count} tools, {dur:.1}s)"
                )
            }
            Self::Error { agent, message, .. } => {
                format!("→ {agent}: error ({message})")
            }
        }
    }

    /// Extract the sequence number from any variant.
    pub fn seq(&self) -> u64 {
        match self {
            Self::TurnStarted { seq, .. }
            | Self::ToolCall { seq, .. }
            | Self::FileRead { seq, .. }
            | Self::ResponseChunk { seq, .. }
            | Self::TurnCompleted { seq, .. }
            | Self::Error { seq, .. } => *seq,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_round_trip_json() {
        let events = vec![
            AgentStreamEvent::TurnStarted {
                agent: "gemini".into(),
                session_name: "research".into(),
                seq: 1,
            },
            AgentStreamEvent::ToolCall {
                agent: "gemini".into(),
                tool_name: "read_file".into(),
                args_summary: "src/auth.rs".into(),
                seq: 2,
            },
            AgentStreamEvent::FileRead {
                agent: "gemini".into(),
                file_path: "src/auth.rs".into(),
                seq: 3,
            },
            AgentStreamEvent::ResponseChunk {
                agent: "gemini".into(),
                text_preview: "generating response".into(),
                seq: 4,
            },
            AgentStreamEvent::TurnCompleted {
                agent: "gemini".into(),
                tokens_in: 12847,
                tokens_out: 1203,
                cached_tokens: Some(8400),
                tool_count: 2,
                duration_ms: 4100,
                seq: 5,
            },
            AgentStreamEvent::Error {
                agent: "codex".into(),
                message: "process exited with code 1".into(),
                seq: 6,
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).expect("serialize");
            let parsed: AgentStreamEvent = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(*event, parsed);

            // Verify serde tag
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(value.get("event_type").is_some(), "missing event_type tag");

            // Verify seq field
            assert!(value.get("seq").is_some(), "missing seq field");
        }
    }

    #[test]
    fn seq_extraction_works() {
        let event = AgentStreamEvent::ToolCall {
            agent: "codex".into(),
            tool_name: "bash".into(),
            args_summary: "cargo check".into(),
            seq: 42,
        };
        assert_eq!(event.seq(), 42);
    }

    #[test]
    fn display_text_formats_correctly() {
        let event = AgentStreamEvent::TurnCompleted {
            agent: "Gemini".into(),
            tokens_in: 12847,
            tokens_out: 1203,
            cached_tokens: Some(8400),
            tool_count: 2,
            duration_ms: 4100,
            seq: 5,
        };
        let text = event.display_text();
        assert!(text.contains("Gemini"));
        assert!(text.contains("12847 in"));
        assert!(text.contains("1203 out"));
        assert!(text.contains("8400 cached"));
        assert!(text.contains("2 tools"));
        assert!(text.contains("4.1s"));
    }
}
