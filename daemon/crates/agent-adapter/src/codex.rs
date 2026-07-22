use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState, WorkingStateEvent,
};
use shared_types::AgentStreamEvent;
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct CodexExecParser {
    thread_id: Option<String>,
    response_chunks: Vec<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    token_usage: Option<TokenUsage>,
    stream_tx: Option<mpsc::Sender<AgentStreamEvent>>,
    stream_seq: u64,
}

impl CodexExecParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_stream_channel(tx: mpsc::Sender<AgentStreamEvent>) -> Self {
        Self {
            stream_tx: Some(tx),
            ..Self::default()
        }
    }

    fn emit_stream_event(&mut self, event: AgentStreamEvent) {
        if let Some(tx) = &self.stream_tx {
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
            "thread.started" => {
                self.thread_id = json.get("thread_id").and_then(|v| v.as_str()).map(ToString::to_string);
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnStarted,
                    detail: "thread started".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnStarted {
                    agent: "codex".into(),
                    session_name: self.thread_id.clone().unwrap_or_default(),
                    seq,
                });
                Some(event)
            }
            "turn.started" => {
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
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
                    agent: "codex".into(),
                    session_name: self.thread_id.clone().unwrap_or_default(),
                    seq,
                });
                Some(event)
            }
            "item.started" => self.parse_item_event(&json, true),
            "item.completed" => self.parse_item_event(&json, false),
            "turn.completed" => {
                let usage = json.get("usage").cloned().unwrap_or_default();
                let token_usage = TokenUsage {
                    input: usage.get("input_tokens").and_then(|v| v.as_u64()),
                    output: usage.get("output_tokens").and_then(|v| v.as_u64()),
                    cached: usage.get("cached_input_tokens").and_then(|v| v.as_u64()),
                    // 0.145 reports reasoning tokens separately as `reasoning_output_tokens`;
                    // map to thinking_tokens (already emitted as tv_thinking_tokens). Previously
                    // dropped, so codex reasoning volume went uncounted.
                    thinking_tokens: usage.get("reasoning_output_tokens").and_then(|v| v.as_u64()),
                    latency_ms: None,
                    tool_calls: None,
                    total: None,
                };
                self.token_usage = Some(token_usage.clone());
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnCompleted,
                    detail: "turn completed".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: Some(token_usage.clone()),
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnCompleted {
                    agent: "codex".into(),
                    tokens_in: token_usage.input.unwrap_or(0) as i64,
                    tokens_out: token_usage.output.unwrap_or(0) as i64,
                    cached_tokens: token_usage.cached.map(|c| c as i64),
                    tool_count: self.tool_calls.len() as i64,
                    duration_ms: token_usage.latency_ms.unwrap_or(0),
                    seq,
                });
                Some(event)
            }
            "error" => {
                let detail = json
                    .get("message")
                    .or_else(|| json.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("codex error")
                    .to_string();
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::Error,
                    detail: detail.clone(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::Error {
                    agent: "codex".into(),
                    message: detail,
                    seq,
                });
                Some(event)
            }
            _ => {
                tracing::debug!("unknown codex event type: {event_type}");
                None
            }
        }
    }

    fn parse_item_event(&mut self, json: &serde_json::Value, started: bool) -> Option<WorkingStateEvent> {
        let item = json.get("item")?;
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        match item_type {
            "agent_message" => {
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                if !text.is_empty() {
                    self.response_chunks.push(text.to_string());
                }
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::MessageDelta,
                    detail: "assistant response chunk".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                Some(event)
            }
            "command_execution" => {
                let command = item.get("command").and_then(|v| v.as_str()).unwrap_or_default();
                if started {
                    self.tool_calls.push(ToolCallRecord {
                        id: item.get("id").and_then(|v| v.as_str()).map(ToString::to_string),
                        tool: "command_execution".to_string(),
                        kind: ToolKind::Bash,
                        success: None,
                        duration_ms: None,
                        args_json: Some(serde_json::json!({"command": command}).to_string()),
                    });
                } else {
                    let id = item.get("id").and_then(|v| v.as_str());
                    let exit_code = item.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
                    if let Some(id) = id
                        && let Some(existing) = self.tool_calls.iter_mut().find(|r| r.id.as_deref() == Some(id))
                    {
                        existing.success = Some(exit_code == 0);
                    }
                }
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: if started {
                        WorkingState::CommandStarted
                    } else {
                        WorkingState::CommandCompleted
                    },
                    detail: if started {
                        "running command".to_string()
                    } else {
                        "command completed".to_string()
                    },
                    tool_name: Some("command_execution".to_string()),
                    tool_args_json: Some(serde_json::json!({"command": command}).to_string()),
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                if started {
                    let seq = self.next_seq();
                    self.emit_stream_event(AgentStreamEvent::ToolCall {
                        agent: "codex".into(),
                        tool_name: "bash".into(),
                        args_summary: command.to_string(),
                        seq,
                    });
                }
                Some(event)
            }
            _ => {
                let state = if started {
                    WorkingState::ToolCallStarted
                } else {
                    WorkingState::ToolCallCompleted
                };
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state,
                    detail: format!("{} {}", if started { "started" } else { "completed" }, item_type),
                    tool_name: Some(item_type.to_string()),
                    tool_args_json: Some(item.to_string()),
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                Some(event)
            }
        }
    }

    pub fn finish(self) -> ParsedAgentResult {
        ParsedAgentResult {
            response_text: self.response_chunks.join("\n"),
            session_id: self.thread_id,
            events: self.events,
            tool_calls: self.tool_calls,
            token_usage: self.token_usage,
            cli_version: None,
            parser_mode: "codex-exec-json".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_golden_trace() {
        let mut parser = CodexExecParser::new();
        let raw = include_str!("../../../tests/fixtures/codex-exec-trace.jsonl");
        let mut parsed = 0;
        for line in raw.lines() {
            if parser.parse_line(line).is_some() {
                parsed += 1;
            }
        }
        let result = parser.finish();
        assert!(parsed >= 5);
        assert_eq!(
            result.session_id.as_deref(),
            Some("019d626f-c562-7a43-b388-f48c1d9b8dc8")
        );
        assert!(result.response_text.contains("8 crates"));
        assert_eq!(result.token_usage.as_ref().and_then(|t| t.output), Some(157));
    }
}
