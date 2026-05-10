// Pre-existing lint debt acknowledged in PR #29. Each allow has a tracking
// issue for follow-up cleanup; remove the allow once the underlying lint is fixed.
#![allow(clippy::large_enum_variant)]

pub mod codex;
pub mod codex_app_server;
pub mod gemini;
pub mod markers;
pub mod stuck;
pub mod types;

pub use codex::CodexExecParser;
pub use codex_app_server::{
    ApprovalChannelMode, ApprovalRequestEvent, CodexAppServerEvent, CodexAppServerParser,
    probe_approval_response_channel,
};
pub use gemini::GeminiStreamParser;
pub use markers::{ToolCallRequest, parse_tool_call_marker};
pub use stuck::{StuckDetector, StuckReason};
pub use types::{
    AgentVerbosity, ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState,
    WorkingStateEvent, should_display,
};

pub fn format_working_state(event: &WorkingStateEvent) -> String {
    match event.state {
        WorkingState::ToolCallStarted | WorkingState::ToolCallCompleted => {
            let tool = event.tool_name.as_deref().unwrap_or("tool");
            let path = extract_param(&event.tool_args_json, &["file_path", "path"]);
            let cmd = extract_param(&event.tool_args_json, &["command"]);
            let pattern = extract_param(&event.tool_args_json, &["pattern", "query"]);
            if let Some(p) = path {
                format!("{}: {} ({})", title_agent(&event.agent), event.detail, p)
            } else if let Some(c) = cmd {
                format!("{}: {} ({})", title_agent(&event.agent), event.detail, c)
            } else if let Some(p) = pattern {
                format!("{}: {} ({})", title_agent(&event.agent), event.detail, p)
            } else {
                format!("{}: {} ({})", title_agent(&event.agent), event.detail, tool)
            }
        }
        WorkingState::TurnCompleted => {
            if let Some(t) = &event.token_usage {
                let i = t.input.unwrap_or(0);
                let o = t.output.unwrap_or(0);
                let c = t.cached.unwrap_or(0);
                format!(
                    "{}: responded ({} in / {} out / {} cached tokens)",
                    title_agent(&event.agent),
                    i,
                    o,
                    c
                )
            } else {
                format!("{}: {}", title_agent(&event.agent), event.detail)
            }
        }
        _ => format!("{}: {}", title_agent(&event.agent), event.detail),
    }
}

fn title_agent(agent: &str) -> String {
    let mut chars = agent.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => "Agent".to_string(),
    }
}

fn extract_param(raw_json: &Option<String>, keys: &[&str]) -> Option<String> {
    let raw = raw_json.as_ref()?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    for key in keys {
        if let Some(v) = value.get(*key).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_state_roundtrip() {
        let event = WorkingStateEvent {
            agent: "gemini".to_string(),
            state: WorkingState::ToolCallStarted,
            detail: "calling ReadFile".to_string(),
            tool_name: Some("ReadFile".to_string()),
            tool_args_json: Some("{\"file_path\":\"src/main.rs\"}".to_string()),
            token_usage: None,
            ts_ms: Some(1),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: WorkingStateEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.agent, "gemini");
        assert_eq!(decoded.state, WorkingState::ToolCallStarted);
    }

    #[test]
    fn format_working_state_shows_file_path() {
        let event = WorkingStateEvent {
            agent: "codex".to_string(),
            state: WorkingState::ToolCallStarted,
            detail: "calling EditFile".to_string(),
            tool_name: Some("EditFile".to_string()),
            tool_args_json: Some("{\"file_path\":\"src/app.rs\"}".to_string()),
            token_usage: None,
            ts_ms: None,
        };
        assert!(format_working_state(&event).contains("src/app.rs"));
    }

    #[test]
    fn verbosity_matrix() {
        assert!(should_display(&WorkingState::TurnStarted, AgentVerbosity::Quiet));
        assert!(!should_display(&WorkingState::MessageDelta, AgentVerbosity::Quiet));
        assert!(should_display(&WorkingState::ToolCallStarted, AgentVerbosity::Standard));
        assert!(!should_display(&WorkingState::Unknown, AgentVerbosity::Detailed));
        assert!(should_display(&WorkingState::Unknown, AgentVerbosity::Raw));
    }
}
