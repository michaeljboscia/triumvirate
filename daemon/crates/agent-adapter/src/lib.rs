// Pre-existing lint debt acknowledged in PR #29. Each allow has a tracking
// issue for follow-up cleanup; remove the allow once the underlying lint is fixed.
#![allow(clippy::large_enum_variant)]

pub mod codex;
pub mod agy_stream;
pub mod codex_app_server;
pub mod gemini;
pub mod grok;
pub mod markers;
pub mod stuck;
pub mod types;

pub use codex::CodexExecParser;
pub use agy_stream::{AgyStreamParser, PARSER_MODE_STREAM as AGY_PARSER_MODE_STREAM};
pub use codex_app_server::{
    ApprovalChannelMode, ApprovalRequestEvent, CodexAppServerEvent, CodexAppServerParser,
    probe_approval_response_channel,
};
pub use gemini::GeminiStreamParser;
pub use grok::{GrokStreamParser, Termination as GrokTermination};
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
            // `target_file` FIRST, because that is what grok actually sends. The vendor guide's
            // example showed `path`, the live capture in
            // tests/fixtures/grok-streaming-tools-20260830.jsonl shows
            // `"rawInput":{"target_file":"target.txt"}`, and checking only the guide's names
            // left every grok tool line in the watch CLI showing the tool name instead of the
            // file. The grok parser was fixed for its own FileRead event; this generic
            // formatter was not, so the same bug survived on the other surface.
            let path = extract_param(&event.tool_args_json, &["target_file", "file_path", "path"]);
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

#[cfg(test)]
mod format_working_state_tests {
    use super::*;
    use crate::types::{TokenUsage, WorkingState, WorkingStateEvent};

    fn ev(args: &str) -> WorkingStateEvent {
        WorkingStateEvent {
            agent: "grok".to_string(),
            state: WorkingState::ToolCallStarted,
            detail: "read_file".to_string(),
            tool_name: Some("read_file".to_string()),
            tool_args_json: Some(args.to_string()),
            token_usage: None::<TokenUsage>,
            ts_ms: None,
        }
    }

    /// The watch CLI must show the FILE, and grok names it `target_file`.
    ///
    /// FIND-GROK-01. The vendor guide's example used `path`, so only `path` and `file_path`
    /// were checked, and every grok tool line rendered the tool name instead of the file.
    /// The grok parser had already been fixed for its own FileRead event; this generic
    /// formatter had not, which is the two-surface split again.
    ///
    /// RED IF: `target_file` is dropped from the lookup list.
    #[test]
    fn format_shows_the_path_grok_actually_sends() {
        let line = format_working_state(&ev(r#"{"target_file":"target.txt"}"#));
        assert!(
            line.contains("target.txt"),
            "grok sends target_file; got: {line}"
        );
    }

    /// The guide's names must keep working, since other agents use them.
    /// RED IF: adding target_file displaced the existing lookups.
    #[test]
    fn format_still_shows_the_documented_names() {
        assert!(format_working_state(&ev(r#"{"path":"a.rs"}"#)).contains("a.rs"));
        assert!(format_working_state(&ev(r#"{"file_path":"b.rs"}"#)).contains("b.rs"));
    }

    /// Locked to the REAL capture, not to my belief about it. If the fixture is ever recaptured
    /// with a different key, this fails rather than silently rendering blank lines again.
    ///
    /// RED IF: the fixture stops carrying a tool call whose rawInput names the file.
    #[test]
    fn the_live_fixture_uses_target_file() {
        const TOOLS: &str =
            include_str!("../tests/fixtures/grok-streaming-tools-20260830.jsonl");
        assert!(
            TOOLS.contains(r#""target_file""#),
            "the committed live capture must still exercise target_file; if grok changed its \
             key, update the lookup list rather than this assertion"
        );
    }
}
