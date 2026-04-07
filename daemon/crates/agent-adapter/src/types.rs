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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParsedAgentResult {
    pub response_text: String,
    pub session_id: Option<String>,
    pub events: Vec<WorkingStateEvent>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub token_usage: Option<TokenUsage>,
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
