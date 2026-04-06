# Backend Structure — Triumvirate v2.1 "Flow State"

**Status:** Final
**Inherits:** `docs/v2/BACKEND_STRUCTURE.md` (unchanged components not repeated)

---

## Workspace Structure (Post v2.1)

```
daemon-v2/
  Cargo.toml                    # Workspace root
  crates/
    shared-types/src/lib.rs     # MCP bridge DTOs (AskTwins types REMOVED)
    mcp-bridge/src/lib.rs       # Config, URL resolution, env var readers (ask_twins helpers REMOVED)
    daemon-core/src/lib.rs      # File I/O, state persistence (UNCHANGED)
    agent-adapter/              # NEW — protocol parsers and types
      src/
        lib.rs                  # format_working_state(), re-exports
        types.rs                # WorkingState, ParsedAgentResult, TokenUsage, AgentVerbosity
        gemini.rs               # GeminiStreamParser
        codex.rs                # CodexExecParser
        stuck.rs                # StuckDetector
    triumvirate/src/main.rs     # Binary: MCP tools, HTTP routes, agent execution
```

## Dependency DAG

```
shared-types ← mcp-bridge ← triumvirate
             ← daemon-core ←
             ← agent-adapter ←
```

`agent-adapter` depends on `shared-types` (for LifecycleEvent if needed) but NOT on `mcp-bridge`, `daemon-core`, `rmcp`, or `axum`. It is protocol-parsing only.

---

## New Types (agent-adapter/src/types.rs)

### WorkingState

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkingState {
    TurnStarted,
    Thinking,
    Planning,
    Generating,
    ToolCalling { tool: String, kind: ToolKind },
    ToolRunning { tool: String, detail: String },
    ToolDone { tool: String, success: bool, duration_ms: Option<u64> },
    ExecutingCommand { command: String },
    WritingFile { path: String },
    WaitingForApproval,
    ContextCompacting,
    Stuck { reason: String },
    Error { message: String },
    TurnCompleted { status: String },
}
```

### ToolKind

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKind {
    Read, Edit, Execute, Search, Fetch, Delete, Move, Think,
    Mcp { server: String },
    Other,
}
```

### AgentVerbosity

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentVerbosity {
    Quiet,
    Standard,
    Detailed,
    Raw,
}

impl Default for AgentVerbosity {
    fn default() -> Self { Self::Standard }
}

impl AgentVerbosity {
    pub fn from_env() -> Self {
        match std::env::var("TRIUMVIRATE_AGENT_VERBOSITY")
            .ok()
            .as_deref()
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("quiet") => Self::Quiet,
            Some("standard") => Self::Standard,
            Some("detailed") => Self::Detailed,
            Some("raw") => Self::Raw,
            Some(other) => {
                tracing::warn!("invalid TRIUMVIRATE_AGENT_VERBOSITY={other:?}, using standard");
                Self::Standard
            }
            None => Self::Standard,
        }
    }
}
```

### WorkingStateEvent

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingStateEvent {
    pub agent: String,
    pub state: WorkingState,
    pub detail: String,
    pub item_id: Option<String>,
    pub session_name: Option<String>,
    pub turn_id: Option<String>,
    pub ts_ms: u128,
}
```

### ParsedAgentResult

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedAgentResult {
    pub response_text: String,
    pub session_id: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub events: Vec<WorkingStateEvent>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub duration_ms: Option<u64>,
    pub cli_version: Option<String>,
    pub parser_mode: String,
}
```

### TokenUsage

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}
```

### ToolCallRecord

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub tool_id: Option<String>,
    pub success: bool,
    pub duration_ms: Option<u64>,
}
```

---

## Parser Interfaces

Both parsers follow the same line-at-a-time pattern:

```rust
// Same interface for both agents
pub trait AgentStreamParser {
    fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent>;
    fn finish(self) -> ParsedAgentResult;
}
```

### GeminiStreamParser

Parses Gemini `--output-format stream-json` NDJSON:

| Input line `type` | Output WorkingState |
|-------------------|-------------------|
| `init` | TurnStarted (capture session_id, model) |
| `message` (assistant, delta) | Generating (accumulate response_text) |
| `tool_use` | ToolCalling { tool_name, kind: Other } |
| `tool_result` (success) | ToolDone { success: true } |
| `tool_result` (error) | ToolDone { success: false } |
| `error` (Loop detected) | Stuck { reason } |
| `error` (other) | Error { message } |
| `result` | TurnCompleted (capture TokenUsage from stats) |

### CodexExecParser

Parses `codex exec --json` JSONL:

| Input line event | Output WorkingState |
|-----------------|-------------------|
| `turn.started` | TurnStarted |
| `agent_reasoning*` | Thinking |
| `plan_delta` | Planning |
| `agent_message*` | Generating (accumulate response_text) |
| `mcp_tool_call.begin` | ToolCalling { tool, kind } |
| `mcp_tool_call.end` | ToolDone { success, duration_ms } |
| `exec_command.begin` | ExecutingCommand { command } |
| `exec_command.end` | ToolDone (correlate via item_id) |
| `patch_apply.begin` | WritingFile { path } |
| `patch_apply.end` | ToolDone |
| `*_approval_request` | WaitingForApproval |
| `context_compacted` | ContextCompacting |
| `TokenCountEvent` | (populate token_usage, no WorkingState emitted) |
| `error` | Error { message } |
| `turn.complete*` / `turn.aborted` | TurnCompleted { status } |

---

## StuckDetector

```rust
pub struct StuckDetector {
    last_meaningful_event: Instant,
    tool_history: Vec<(String, String)>,  // (tool_name, args_hash)
    input_request_count: u32,
    idle_timeout: Duration,       // default 60s
    frozen_timeout: Duration,     // default 90s
    max_tool_repeats: u32,        // default 5
}

impl StuckDetector {
    pub fn observe(&mut self, event: &WorkingStateEvent) -> Option<StuckReason> { ... }
}

pub enum StuckReason {
    IdleTimeout { elapsed: Duration },
    ToolLoop { tool: String, count: u32 },
    InputLoop { count: u32 },
    Frozen { elapsed: Duration },
}
```

---

## Modified Types (shared-types)

### Removed
- `AskTwinsRequest`
- `AskTwinsResponse`
- `AgentResult` (old — replaced by `ParsedAgentResult` in agent-adapter)

### Unchanged
All other types in shared-types remain as-is.

---

## Modified Functions (mcp-bridge)

### Removed
- `build_role_adapted_prompts()`
- `daemon_ask_twins_url()`

### Added
- `pub fn agent_verbosity() -> AgentVerbosity` — reads `TRIUMVIRATE_AGENT_VERBOSITY`
- `pub fn gemini_streaming_enabled() -> bool` — reads `TRIUMVIRATE_GEMINI_STREAMING`
- `pub fn codex_protocol_mode() -> String` — reads `TRIUMVIRATE_CODEX_PROTOCOL`

---

## Environment Variables (New)

| Variable | Default | Description |
|----------|---------|-------------|
| `TRIUMVIRATE_AGENT_VERBOSITY` | `standard` | Display filter: quiet, standard, detailed, raw |
| `TRIUMVIRATE_GEMINI_STREAMING` | `true` | Enable live streaming for Gemini (false = batch) |
| `TRIUMVIRATE_CODEX_PROTOCOL` | `exec` | Codex protocol: exec (v2.1) or app-server (v2.2) |

## Environment Variables (Removed)

| Variable | Reason |
|----------|--------|
| `TRIUMVIRATE_DAEMON_ASK_TWINS_URL` | ask_twins removed |
| `TRIUMVIRATE_ASK_TWINS_ROLE_ADAPT` | ask_twins removed |
