use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, WorkingState, WorkingStateEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequestEvent {
    pub id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalChannelMode {
    ProceedOnce,
    FullAutoFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexAppServerEvent {
    Working(WorkingStateEvent),
    ApprovalRequest(ApprovalRequestEvent),
}

#[derive(Debug, Default)]
pub struct CodexAppServerParser {
    thread_id: Option<String>,
    response_chunks: Vec<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    token_usage: Option<TokenUsage>,
    cli_version: Option<String>,
}

impl CodexAppServerParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse_line(&mut self, line: &str) -> anyhow::Result<Option<WorkingStateEvent>> {
        Ok(match self.parse_event_line(line)? {
            Some(CodexAppServerEvent::Working(event)) => Some(event),
            Some(CodexAppServerEvent::ApprovalRequest(_)) | None => None,
        })
    }

    pub fn parse_event_line(&mut self, line: &str) -> anyhow::Result<Option<CodexAppServerEvent>> {
        let json: serde_json::Value = serde_json::from_str(line)?;

        // Guardrail: exec JSON lines are incompatible with app-server mode.
        if json.get("type").is_some() && json.get("jsonrpc").is_none() {
            anyhow::bail!("exec-format JSON detected; expected codex app-server JSON-RPC");
        }

        if let Some(method) = json.get("method").and_then(|v| v.as_str()) {
            return Ok(self.parse_notification(method, json.get("params")));
        }

        if let Some(result) = json.get("result")
            && let Some(model) = result.get("model").and_then(|v| v.as_str())
        {
            self.cli_version = Some(model.to_string());
        }

        Ok(None)
    }

    fn parse_notification(
        &mut self,
        method: &str,
        params: Option<&serde_json::Value>,
    ) -> Option<CodexAppServerEvent> {
        match method {
            "initialized" => {
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnStarted,
                    detail: "app-server initialized".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                Some(CodexAppServerEvent::Working(event))
            }
            "thread/start" => {
                self.thread_id = params
                    .and_then(|p| p.get("thread_id"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
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
                Some(CodexAppServerEvent::Working(event))
            }
            "turn/start" => {
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
                Some(CodexAppServerEvent::Working(event))
            }
            "turn/text-delta" | "stream/text-delta" => {
                if let Some(delta) = params
                    .and_then(|p| p.get("delta").and_then(|v| v.as_str()).or_else(|| p.get("text").and_then(|v| v.as_str())))
                    && !delta.is_empty()
                {
                    self.response_chunks.push(delta.to_string());
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
                Some(CodexAppServerEvent::Working(event))
            }
            "approval/request" | "approval_request" => {
                let request = ApprovalRequestEvent {
                    id: params
                        .and_then(|p| p.get("id"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    reason: params
                        .and_then(|p| p.get("reason").and_then(|v| v.as_str()))
                        .map(ToString::to_string),
                };
                Some(CodexAppServerEvent::ApprovalRequest(request))
            }
            "turn/completed" => {
                let usage = params
                    .and_then(|p| p.get("usage"))
                    .cloned()
                    .unwrap_or_default();
                let token_usage = TokenUsage {
                    input: usage
                        .get("input_tokens")
                        .or_else(|| usage.get("input"))
                        .and_then(|v| v.as_u64()),
                    output: usage
                        .get("output_tokens")
                        .or_else(|| usage.get("output"))
                        .and_then(|v| v.as_u64()),
                    cached: usage
                        .get("cached_input_tokens")
                        .or_else(|| usage.get("cached"))
                        .and_then(|v| v.as_u64()),
                    thinking_tokens: None,
                    latency_ms: None,
                    tool_calls: None,
                    total: usage
                        .get("total_tokens")
                        .or_else(|| usage.get("total"))
                        .and_then(|v| v.as_u64()),
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
                Some(CodexAppServerEvent::Working(event))
            }
            _ => None,
        }
    }

    pub fn finish(self) -> ParsedAgentResult {
        ParsedAgentResult {
            response_text: self.response_chunks.join(""),
            session_id: self.thread_id,
            events: self.events,
            tool_calls: self.tool_calls,
            token_usage: self.token_usage,
            self_reported_cost_usd: None,
            cli_version: self.cli_version,
            parser_mode: "codex-app-server-jsonrpc".to_string(),
        }
    }
}

pub fn probe_approval_response_channel(probe_response: &str) -> anyhow::Result<ApprovalChannelMode> {
    let json: serde_json::Value = serde_json::from_str(probe_response)?;
    if json.get("error").is_none() {
        return Ok(ApprovalChannelMode::ProceedOnce);
    }

    let method_unsupported = json
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(|m| m.to_lowercase().contains("method not supported"))
        .unwrap_or(false);
    if method_unsupported {
        anyhow::bail!("approval response channel method not supported");
    }

    anyhow::bail!("approval response channel probe failed")
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalChannelMode, CodexAppServerEvent, CodexAppServerParser,
        probe_approval_response_channel,
    };

    #[test]
    fn parses_app_server_jsonrpc_trace() {
        let mut parser = CodexAppServerParser::new();
        let lines = [
            r#"{"jsonrpc":"2.0","id":1,"result":{"model":"codex-app-server"}}"#,
            r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
            r#"{"jsonrpc":"2.0","method":"thread/start","params":{"thread_id":"thread-123"}}"#,
            r#"{"jsonrpc":"2.0","method":"turn/start","params":{"turn_id":"turn-1"}}"#,
            r#"{"jsonrpc":"2.0","method":"turn/text-delta","params":{"delta":"Hello "}}"#,
            r#"{"jsonrpc":"2.0","method":"turn/text-delta","params":{"delta":"world"}}"#,
            r#"{"jsonrpc":"2.0","method":"turn/completed","params":{"usage":{"input_tokens":120,"output_tokens":80,"total_tokens":200}}}"#,
        ];

        for line in lines {
            parser.parse_line(line).expect("parse app-server line");
        }
        let out = parser.finish();
        assert_eq!(out.session_id.as_deref(), Some("thread-123"));
        assert_eq!(out.response_text, "Hello world");
        assert_eq!(out.token_usage.as_ref().and_then(|u| u.total), Some(200));
        assert_eq!(out.parser_mode, "codex-app-server-jsonrpc");
    }

    #[test]
    fn rejects_exec_format_json() {
        let mut parser = CodexAppServerParser::new();
        let err = parser
            .parse_line(r#"{"type":"turn.completed","usage":{"output_tokens":42}}"#)
            .expect_err("exec format must error");
        assert!(err.to_string().contains("exec-format"));
    }

    #[test]
    fn detects_approval_request_events() {
        let mut parser = CodexAppServerParser::new();
        let event = parser
            .parse_event_line(
                r#"{"jsonrpc":"2.0","method":"approval_request","params":{"id":"apr-1","reason":"edit file"}}"#,
            )
            .expect("parse approval event");
        match event {
            Some(CodexAppServerEvent::ApprovalRequest(req)) => {
                assert_eq!(req.id.as_deref(), Some("apr-1"));
                assert_eq!(req.reason.as_deref(), Some("edit file"));
            }
            other => panic!("expected approval request event, got {other:?}"),
        }
    }

    #[test]
    fn approval_probe_accepts_supported_channel() {
        let mode = probe_approval_response_channel(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .expect("probe should succeed");
        assert_eq!(mode, ApprovalChannelMode::ProceedOnce);
    }

    #[test]
    fn approval_probe_rejects_unsupported_channel() {
        let err = probe_approval_response_channel(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not supported"}}"#,
        )
        .expect_err("probe should fail");
        assert!(err.to_string().contains("method not supported"));
    }
}
