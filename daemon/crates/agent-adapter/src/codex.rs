use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState, WorkingStateEvent,
};

#[derive(Debug, Default)]
pub struct CodexExecParser {
    thread_id: Option<String>,
    response_chunks: Vec<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    token_usage: Option<TokenUsage>,
}

impl CodexExecParser {
    pub fn new() -> Self {
        Self::default()
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
                    thinking_tokens: None,
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
                    token_usage: Some(token_usage),
                    ts_ms: None,
                };
                self.events.push(event.clone());
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
                    detail,
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
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
