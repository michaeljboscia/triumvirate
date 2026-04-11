use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState, WorkingStateEvent,
};
use shared_types::AgentStreamEvent;
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct GeminiStreamParser {
    session_id: Option<String>,
    response_chunks: Vec<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    token_usage: Option<TokenUsage>,
    cli_version: Option<String>,
    /// Optional channel for emitting AgentStreamEvent during parsing.
    /// When set, each meaningful parse event is forwarded to this sender.
    /// REQ-E03
    stream_tx: Option<mpsc::Sender<AgentStreamEvent>>,
    stream_seq: u64,
}

impl GeminiStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a parser that also emits AgentStreamEvent to the given channel.
    pub fn with_stream_channel(tx: mpsc::Sender<AgentStreamEvent>) -> Self {
        Self {
            stream_tx: Some(tx),
            ..Self::default()
        }
    }

    /// Try to send an AgentStreamEvent to the stream channel (if configured).
    /// Non-blocking best-effort — if the receiver is dropped, events are silently lost.
    fn emit_stream_event(&mut self, event: AgentStreamEvent) {
        if let Some(tx) = &self.stream_tx {
            // try_send to avoid blocking the parser — drop events on backpressure
            let _ = tx.try_send(event);
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.stream_seq += 1;
        self.stream_seq
    }

    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent> {
        let json: serde_json::Value = serde_json::from_str(line).ok()?;
        let event_type = json.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        match event_type {
            "init" => {
                self.session_id = json
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                self.cli_version = json.get("model").and_then(|v| v.as_str()).map(ToString::to_string);
                let event = WorkingStateEvent {
                    agent: "gemini".to_string(),
                    state: WorkingState::TurnStarted,
                    detail: "turn started".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnStarted {
                    agent: "gemini".into(),
                    session_name: self.session_id.clone().unwrap_or_default(),
                    seq,
                });
                Some(event)
            }
            "message" => {
                let role = json.get("role").and_then(|v| v.as_str()).unwrap_or_default();
                if role == "assistant" {
                    let content = json
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if !content.is_empty() {
                        self.response_chunks.push(content.clone());
                    }
                    let event = WorkingStateEvent {
                        agent: "gemini".to_string(),
                        state: WorkingState::MessageDelta,
                        detail: "assistant response chunk".to_string(),
                        tool_name: None,
                        tool_args_json: None,
                        token_usage: None,
                        ts_ms: None,
                    };
                    self.events.push(event.clone());
                    return Some(event);
                }
                None
            }
            "tool_use" => {
                let tool_name = json
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let args_json = json.get("parameters").map(|v| v.to_string());
                self.tool_calls.push(ToolCallRecord {
                    id: json.get("tool_id").and_then(|v| v.as_str()).map(ToString::to_string),
                    tool: tool_name.to_string(),
                    kind: map_tool_kind(tool_name),
                    success: None,
                    duration_ms: None,
                    args_json: args_json.clone(),
                });
                let event = WorkingStateEvent {
                    agent: "gemini".to_string(),
                    state: WorkingState::ToolCallStarted,
                    detail: format!("calling {tool_name}"),
                    tool_name: Some(tool_name.to_string()),
                    tool_args_json: args_json.clone(),
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                if tool_name == "read_file" {
                    let path = args_json
                        .as_deref()
                        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                        .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(String::from))
                        .unwrap_or_default();
                    self.emit_stream_event(AgentStreamEvent::FileRead {
                        agent: "gemini".into(),
                        file_path: path,
                        seq,
                    });
                } else {
                    self.emit_stream_event(AgentStreamEvent::ToolCall {
                        agent: "gemini".into(),
                        tool_name: tool_name.to_string(),
                        args_summary: args_json.clone().unwrap_or_default(),
                        seq,
                    });
                }
                Some(event)
            }
            "tool_result" => {
                let status = json.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
                let tool_id = json.get("tool_id").and_then(|v| v.as_str());
                if let Some(id) = tool_id
                    && let Some(existing) = self.tool_calls.iter_mut().find(|r| r.id.as_deref() == Some(id))
                {
                    existing.success = Some(status.eq_ignore_ascii_case("success"));
                }
                let event = WorkingStateEvent {
                    agent: "gemini".to_string(),
                    state: WorkingState::ToolCallCompleted,
                    detail: format!("tool result: {status}"),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                Some(event)
            }
            "error" => {
                let msg = json
                    .get("message")
                    .or_else(|| json.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("gemini error")
                    .to_string();
                let is_stuck = msg.to_lowercase().contains("loopdetected");
                let event = WorkingStateEvent {
                    agent: "gemini".to_string(),
                    state: if is_stuck {
                        WorkingState::Stuck
                    } else {
                        WorkingState::Error
                    },
                    detail: msg.clone(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::Error {
                    agent: "gemini".into(),
                    message: msg,
                    seq,
                });
                Some(event)
            }
            "result" => {
                let stats = json.get("stats").cloned().unwrap_or_default();
                let usage = TokenUsage {
                    input: stats
                        .get("input_tokens")
                        .or_else(|| stats.get("input"))
                        .and_then(|v| v.as_u64()),
                    output: stats.get("output_tokens").and_then(|v| v.as_u64()),
                    cached: stats.get("cached").and_then(|v| v.as_u64()),
                    thinking_tokens: stats
                        .get("thoughtsTokenCount")
                        .and_then(|v| v.as_u64()),
                    latency_ms: stats
                        .get("totalLatencyMs")
                        .or_else(|| stats.get("duration_ms"))
                        .and_then(|v| v.as_u64()),
                    tool_calls: stats
                        .get("tools")
                        .and_then(|v| v.get("totalCalls"))
                        .or_else(|| stats.get("tool_calls"))
                        .and_then(|v| v.as_u64()),
                    total: stats.get("total_tokens").and_then(|v| v.as_u64()),
                };
                self.token_usage = Some(usage.clone());
                let event = WorkingStateEvent {
                    agent: "gemini".to_string(),
                    state: WorkingState::TurnCompleted,
                    detail: "turn completed".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: Some(usage.clone()),
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnCompleted {
                    agent: "gemini".into(),
                    tokens_in: usage.input.unwrap_or(0) as i64,
                    tokens_out: usage.output.unwrap_or(0) as i64,
                    cached_tokens: usage.cached.map(|c| c as i64),
                    tool_count: usage.tool_calls.unwrap_or(0) as i64,
                    duration_ms: usage.latency_ms.unwrap_or(0),
                    seq,
                });
                Some(event)
            }
            _ => {
                tracing::debug!("unknown gemini event type: {event_type}");
                None
            }
        }
    }

    pub fn finish(self) -> ParsedAgentResult {
        ParsedAgentResult {
            response_text: self.response_chunks.join(""),
            session_id: self.session_id,
            events: self.events,
            tool_calls: self.tool_calls,
            token_usage: self.token_usage,
            cli_version: self.cli_version,
            parser_mode: "gemini-stream-json".to_string(),
        }
    }
}

fn map_tool_kind(name: &str) -> ToolKind {
    match name.to_lowercase().as_str() {
        "read_file" => ToolKind::ReadFile,
        "write_file" => ToolKind::WriteFile,
        "edit_file" => ToolKind::EditFile,
        "bash" => ToolKind::Bash,
        "grep" => ToolKind::Grep,
        "glob" => ToolKind::Glob,
        "request_user_input" => ToolKind::RequestUserInput,
        _ => ToolKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_golden_trace() {
        let mut parser = GeminiStreamParser::new();
        let raw = include_str!("../../../tests/fixtures/gemini-stream-trace.jsonl");
        let mut count = 0;
        for line in raw.lines() {
            if parser.parse_line(line).is_some() {
                count += 1;
            }
        }
        let result = parser.finish();
        assert!(count >= 5);
        assert_eq!(result.session_id.as_deref(), Some("9396020a-f4d8-43e9-82e0-386da5df7cb1"));
        assert!(result.response_text.contains("8 crates"));
        assert_eq!(result.token_usage.as_ref().and_then(|t| t.total), Some(43893));
        assert_eq!(result.token_usage.as_ref().and_then(|t| t.thinking_tokens), Some(121));
        assert_eq!(result.token_usage.as_ref().and_then(|t| t.latency_ms), Some(19516));
        assert_eq!(result.token_usage.as_ref().and_then(|t| t.tool_calls), Some(1));
    }
}
