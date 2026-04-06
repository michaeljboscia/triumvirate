# PRD — Triumvirate v2.1 "Flow State"

**Status:** Final (Goatrodeo-approved, 4 rounds)
**Date:** 2026-04-06
**Source:** `docs/v2/FLOW_STATE_SPEC.md`, `docs/v2/GOATRODEO_FLOW_STATE.md`
**Inherits:** `docs/v2/PRD.md` (v2.0 features unchanged unless noted)

---

## Product Vision

When you ask triumvirate to dispatch work to Gemini or Codex, you see what they're doing in real time — thinking, calling tools, writing files, executing commands, stuck in a loop — not dumb timer heartbeats.

---

## Features

### FEAT-001: Gemini Stream-JSON Parser
**Priority:** P0 (Core)
**REQs:** REQ-001, REQ-012, REQ-015 (AR-15, AR-20, AR-29)
**User Story:** As a user dispatching work to Gemini, I want the daemon to parse all stream-json event types so that tool calls, errors, and token usage are captured — not just the final text.
**Acceptance Criteria:**
- Parser handles all 6 event types: `init`, `message`, `tool_use`, `tool_result`, `error`, `result`
- Parser is line-at-a-time (`fn parse_line(&str) -> Option<WorkingStateEvent>`) — stream-ready from day one
- Unknown event types logged at debug level and skipped, never panic
- If parser produces empty response, falls back to raw stdout trimming
- Returns `ParsedAgentResult` with `response_text`, `session_id`, `token_usage`, `events`, `tool_calls`

### FEAT-002: Codex Exec-JSON Parser
**Priority:** P0 (Core)
**REQs:** REQ-002, REQ-012, REQ-026 (AR-15, AR-16, AR-20, AR-24, AR-29)
**User Story:** As a user dispatching work to Codex, I want the daemon to parse codex exec --json JSONL events so that tool calls, command execution, file changes, and token usage are captured.
**Acceptance Criteria:**
- Parser handles event families: `turn.started/completed`, `agent_message*`, `exec_command.begin/end`, `mcp_tool_call.begin/end`, `patch_apply.begin/end`, `TokenCountEvent`, `error`
- Correlates `*.begin` → `*.end` via `item_id` / `process_id`
- Captures token usage: `input_token_count`, `output_token_count`, `cached_token_count`, `reasoning_token_count`
- Line-at-a-time parser, same pattern as FEAT-001
- Separate adapter from app-server (different wire format: flat JSONL vs JSON-RPC 2.0)
- Falls back to raw stdout + last-message file on parse failure

### FEAT-003: Live Streaming (.spawn() Migration)
**Priority:** P0 (Core — THE KEY CHANGE)
**REQs:** REQ-003, REQ-007, REQ-014, REQ-022 (AR-3, AR-10, AR-22, AR-27)
**User Story:** As a user, I want to see what agents are doing AS they work, not after they finish.
**Acceptance Criteria:**
- Both agent runners switch from `Command::output().await` to `Command::spawn()` + line-by-line async reading
- Uses `AsyncBufReadExt::lines()` or `LinesCodec`/`FramedRead` (cancellation safe)
- WorkingStateEvents sent via bounded `mpsc` channel (capacity 1024) with `try_send()` drop-oldest
- Concurrent stderr drain task (to `tracing::debug!`, elevated to `error!` on non-zero exit)
- All subprocesses use `kill_on_drop(true)` + `process_group(0)` for grandchild cleanup
- Timeout wraps entire spawn+read future (same timeout behavior as current `.output()`)
- Parser accumulates raw stdout for fallback
- Gemini wired first in PR, Codex second
- Disableable via `TRIUMVIRATE_GEMINI_STREAMING=false` (falls back to batch)

### FEAT-004: Unified WorkingState Protocol
**Priority:** P0 (Core)
**REQs:** REQ-006, REQ-016 (AR-4, AR-8)
**User Story:** As a daemon developer, I want one type that represents what any agent is doing, regardless of which CLI protocol produced the event.
**Acceptance Criteria:**
- `WorkingState` enum with 14 variants: TurnStarted, Thinking, Planning, Generating, ToolCalling, ToolRunning, ToolDone, ExecutingCommand, WritingFile, WaitingForApproval, ContextCompacting, Stuck, Error, TurnCompleted
- `ToolKind` enum: Read, Edit, Execute, Search, Fetch, Delete, Move, Think, Mcp, Other
- `WorkingStateEvent` struct with agent, state, detail, item_id, session_name, turn_id, ts_ms
- `ParsedAgentResult` struct with response_text, session_id, token_usage, events, tool_calls, duration_ms
- `ToolCallRecord` struct with tool_name, tool_id, success, duration_ms
- `TokenUsage` struct with input_tokens, output_tokens, cached_tokens, reasoning_tokens, total_tokens

### FEAT-005: AgentVerbosity Display System
**Priority:** P0 (Core)
**REQs:** REQ-004, REQ-005, REQ-023 (D-2, D-5)
**User Story:** As a user, I want to control how much detail I see about agent activity, from "just the answer" to "show me everything."
**Acceptance Criteria:**
- Four levels: Quiet, Standard (default), Detailed, Raw
- Env var: `TRIUMVIRATE_AGENT_VERBOSITY=quiet|standard|detailed|raw`
- Missing env var → default Standard. Invalid value → warn + default Standard.
- Mapping matrix:
  - Always visible: Error, Stuck, Completed, WaitingOnApproval, WaitingOnUserInput
  - Standard+: TurnStarted, TurnCompleted, ToolCall, Generating, Initializing, ConfigWarning, ModelRerouted
  - Detailed+: Thinking, ToolResult (duration/success)
  - Raw only: Heartbeat, per-delta streams, timestamps
- Heartbeat timer becomes fallback — only fires when no real events received within interval
- AgentVerbosity filters BEFORE ProgressEmitter — filtered events never serialized
- ProgressEmitter uses `LoggingLevel::Info` for all emitted events (unchanged MCP contract)
- Outbox receives ALL events regardless of verbosity level

### FEAT-006: Stuck Detection
**Priority:** P0 (Core)
**REQs:** REQ-009, REQ-010 (AR-4, AR-19)
**User Story:** As a user, I want to be told immediately when an agent is stuck — not discover it after minutes of silence.
**Acceptance Criteria:**
- Gemini: pass through built-in `LoopDetected` event as `WorkingState::Stuck`
- Codex StuckDetector:
  - No meaningful events for >60s after `turn.started` → Stuck (timeout)
  - Same tool + same arguments >5x in sequence → Stuck (loop)
  - Repeated `requestUserInput` denials → Stuck (input loop)
  - No events for >90s after last event → Stuck (frozen)
- Stuck events surface through AgentVerbosity at ALL levels (always visible)
- Stuck events logged to outbox

### FEAT-007: Token Usage Visibility
**Priority:** P1 (Important)
**REQs:** REQ-008 (AR-25, AR-26)
**User Story:** As a user with unlimited subscriptions, I want to see my actual token counts per agent per turn — input, output, cached, reasoning.
**Acceptance Criteria:**
- Gemini: extracted from `result.stats` (input_tokens, output_tokens, cached, per-model)
- Codex: extracted from `TokenCountEvent` (input_token_count, output_token_count, cached_token_count, reasoning_token_count)
- Stored in `ParsedAgentResult.token_usage`
- Displayed in completion message: "responded (1,247 in / 503 out / 200 cached tokens)"
- No dollar amounts — raw token counts only

### FEAT-008: ask_twins Removal
**Priority:** P0 (Cleanup)
**REQs:** REQ-015 (AR-13, D-4)
**User Story:** As a user, I want "ask the twins" to always route to the daemon session pattern, not a redundant MCP tool that causes confusion.
**Acceptance Criteria:**
- MCP tool `ask_twins` deleted from `#[tool_router]` impl
- HTTP route `/ask-twins` deleted
- `execute_ask_twins()` function deleted (~260 lines)
- `fetch_daemon_ask_twins()` proxy deleted
- 5 test functions deleted
- `AskTwinsRequest`, `AskTwinsResponse`, `AgentResult` (old) deleted from shared-types
- `build_role_adapted_prompts()`, `daemon_ask_twins_url()` deleted from mcp-bridge
- Stale references scrubbed from README.md, env var docs
- Route registration line removed
- ~750 lines total deletion

### FEAT-009: agent-adapter Crate
**Priority:** P0 (Infrastructure)
**REQs:** REQ-018 (AR-1)
**User Story:** As a developer, I want protocol-specific parsing logic separated from the MCP surface and daemon persistence layers.
**Acceptance Criteria:**
- New crate at `daemon-v2/crates/agent-adapter/`
- Contains: `types.rs`, `gemini.rs`, `codex.rs`, `stuck.rs`, `lib.rs`
- Added to workspace `Cargo.toml` members
- Added as dependency to `crates/triumvirate/Cargo.toml`
- Dependencies: `serde`, `serde_json`, `tokio`, `anyhow`, `tracing`
- Does NOT depend on `rmcp`, `axum`, or other MCP/HTTP crates
- Type named `ParsedAgentResult` (not `AgentResult`) to avoid collision during transition

### FEAT-010: Backward Compatibility
**Priority:** P0 (Safety)
**REQs:** REQ-012, REQ-013, REQ-019, REQ-021 (D-3)
**User Story:** As a user, I want flow state to be additive — nothing I use today should break.
**Acceptance Criteria:**
- `ask_session` MCP tool response format unchanged (final text string)
- WorkingState events are additive progress notifications only
- `TRIUMVIRATE_GEMINI_STREAMING=false` falls back to batch `.output()` mode
- `TRIUMVIRATE_CODEX_PROTOCOL=exec` remains the default (app-server opt-in)
- All existing workspace tests pass after every phase (`cargo test`)
- Version telemetry added: `cli_version`, `parser_mode` in results

---

## Features Deferred to v2.2

| Feature | Reason |
|---------|--------|
| Codex app-server mode (JSON-RPC over stdio) | Enrichment — exec --json sufficient for flow state |
| Gemini ACP mode (thinking tokens, tool kinds) | Enrichment — stream-json sufficient for v2.1 |
| Outbox enrichment (working_state, tool_name fields) | Nice-to-have, not blocking |
| Outbox GC/rotation (REQ-025) | Long-running daemon concern, not blocking v2.1 |
| Per-session AgentVerbosity override | Needs spawn_session parameter, future work |
| Dashboard/frontend for flow state | Visualization is separate from the signal |
