use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentVerbosity {
    Quiet,
    Standard,
    Detailed,
    Raw,
}

impl AgentVerbosity {
    pub fn from_env(raw: Option<&str>) -> Self {
        match raw.unwrap_or("normal").to_lowercase().as_str() {
            "quiet" | "minimal" => Self::Quiet,
            "standard" | "normal" => Self::Standard,
            "detailed" | "verbose" => Self::Detailed,
            "raw" | "debug" => Self::Raw,
            _ => Self::Standard,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    ReadFile,
    WriteFile,
    EditFile,
    Bash,
    Grep,
    Glob,
    RequestUserInput,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached: Option<u64>,
    pub thinking_tokens: Option<u64>,
    pub latency_ms: Option<u64>,
    pub tool_calls: Option<u64>,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub id: Option<String>,
    pub tool: String,
    pub kind: ToolKind,
    pub success: Option<bool>,
    pub duration_ms: Option<u64>,
    pub args_json: Option<String>,
    /// First and last line numbers the tool's RESULT showed, when the result numbers its lines
    /// (grok `N→text`, claude `N\ttext`). D-032: the sight gate used to credit the window a read
    /// ASKED for; this is the window it RECEIVED, truncation and offset convention included.
    /// `None` when the result does not number its lines (agy `view_file` reports only a count).
    #[serde(default)]
    pub returned_lines: Option<(u64, u64)>,
}

/// The first and last line numbers in a read tool's line-numbered output. A line counts when,
/// after leading spaces, it starts with digits followed by `→` (grok) or a tab (claude). `None`
/// when no line is numbered, so an unnumbered result never invents a range.
pub fn numbered_line_span(text: &str) -> Option<(u64, u64)> {
    let mut span: Option<(u64, u64)> = None;
    for line in text.lines() {
        let t = line.trim_start_matches(' ');
        let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            continue;
        }
        let rest = &t[digits..];
        if !(rest.starts_with('→') || rest.starts_with('\t')) {
            continue;
        }
        let Ok(n) = t[..digits].parse::<u64>() else { continue };
        span = Some(match span {
            None => (n, n),
            Some((a, b)) => (a.min(n), b.max(n)),
        });
    }
    span
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "data")]
pub enum WorkingState {
    TurnStarted,
    MessageDelta,
    ToolCallStarted,
    ToolCallCompleted,
    CommandStarted,
    CommandCompleted,
    FileEditStarted,
    FileEditCompleted,
    InputRequested,
    Stuck,
    TurnCompleted,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkingStateEvent {
    pub agent: String,
    pub state: WorkingState,
    pub detail: String,
    pub tool_name: Option<String>,
    pub tool_args_json: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub ts_ms: Option<u128>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]  // `Eq` dropped: f64 has no total order.
pub struct ParsedAgentResult {
    pub response_text: String,
    pub session_id: Option<String>,
    pub events: Vec<WorkingStateEvent>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub token_usage: Option<TokenUsage>,
    /// Cost the AGENT reported for this turn, in USD, when it reports one.
    ///
    /// Only grok does today: `end.total_cost_usd`. It runs on a flat SuperGrok plan, so this is
    /// a USAGE signal rather than a bill, and it is the only per-turn quota number available
    /// for a subscription agent.
    ///
    /// It was being dropped: the runner persisted `cost_usd: None` while the parser had the
    /// value in hand, so grok quota burn was under-recorded. Codex found it reviewing slice J.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_reported_cost_usd: Option<f64>,
    pub cli_version: Option<String>,
    pub parser_mode: String,
}

pub fn should_display(state: &WorkingState, verbosity: AgentVerbosity) -> bool {
    match verbosity {
        AgentVerbosity::Quiet => matches!(
            state,
            WorkingState::TurnStarted
                | WorkingState::TurnCompleted
                | WorkingState::Stuck
                | WorkingState::Error
                | WorkingState::InputRequested
        ),
        AgentVerbosity::Standard => matches!(
            state,
            WorkingState::TurnStarted
                | WorkingState::TurnCompleted
                | WorkingState::ToolCallStarted
                | WorkingState::ToolCallCompleted
                | WorkingState::CommandStarted
                | WorkingState::CommandCompleted
                | WorkingState::InputRequested
                | WorkingState::Stuck
                | WorkingState::Error
        ),
        AgentVerbosity::Detailed => !matches!(state, WorkingState::Unknown),
        AgentVerbosity::Raw => true,
    }
}

#[cfg(test)]
mod numbered_line_span_tests {
    use super::numbered_line_span;

    /// RED IF a numbered result loses its span, or an unnumbered one invents a range.
    #[test]
    fn spans_grok_and_claude_numbering_and_nothing_else() {
        assert_eq!(numbered_line_span("1→a\n2→b\n3→c\n"), Some((1, 3)), "grok");
        assert_eq!(numbered_line_span("    41\tfn x() {\n    42\t}\n"), Some((41, 42)), "claude cat -n");
        assert_eq!(numbered_line_span("403 lines, 42837 bytes"), None, "agy's summary is not a span");
        assert_eq!(numbered_line_span("plain text\n12 apples\n"), None, "a number without the marker is content");
        assert_eq!(numbered_line_span(""), None);
    }
}
