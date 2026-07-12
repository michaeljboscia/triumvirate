# Implementation Plan — Triumvirate v2.1 "Flow State"

**Status:** Final
**Source:** PRD.md (FEAT-001 through FEAT-010), GOATRODEO_FLOW_STATE.md
**Working Directory:** /Users/you/projects/triumvirate/daemon-v2

---

## Phase 0: Types + Crate Scaffold (FEAT-009, FEAT-004)

**Goal:** Create agent-adapter crate with all types. Zero behavioral change.

### Step 0.1: Create crate
- Create `crates/agent-adapter/Cargo.toml` with deps: serde, serde_json, tokio, anyhow, tracing
- Create `crates/agent-adapter/src/lib.rs` — module root
- Create `crates/agent-adapter/src/types.rs` — WorkingState, WorkingStateEvent, ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, AgentVerbosity enum
- Create empty `crates/agent-adapter/src/gemini.rs`
- Create empty `crates/agent-adapter/src/codex.rs`
- Create empty `crates/agent-adapter/src/stuck.rs`
- Add `"crates/agent-adapter"` to workspace `Cargo.toml` members
- Add `agent-adapter = { path = "../agent-adapter" }` to `crates/triumvirate/Cargo.toml`

### Step 0.2: Add format_working_state()
- In `crates/agent-adapter/src/lib.rs`: `pub fn format_working_state(event: &WorkingStateEvent) -> String`
- Maps each WorkingState variant to human-readable: "Gemini: calling ReadFile (src/main.rs)"
- Extracts known parameter keys per tool: `file_path` for Read/Edit/Write, `command` for Bash, `pattern` for Grep/Glob

### Step 0.3: Add AgentVerbosity filter
- In `crates/agent-adapter/src/types.rs`: `pub fn should_display(state: &WorkingState, verbosity: AgentVerbosity) -> bool`
- Implements the mapping matrix from D-5

### Step 0.4: Verify
- `cargo check` across workspace
- `cargo test` across workspace — all existing tests pass
- New unit tests: WorkingStateEvent serialization roundtrip, format_working_state coverage, AgentVerbosity filter matrix

**Files created:** 5 new files in `crates/agent-adapter/`
**Files modified:** `Cargo.toml` (workspace), `crates/triumvirate/Cargo.toml`
**PR scope:** ~200 lines added

---

## Phase 1: Gemini Stream-JSON Parser (FEAT-001)

**Goal:** Parse all event types from Gemini stdout. Batch mode (still .output()).

### Step 1.1: Capture golden trace
- Run `gemini -o stream-json -p "read README.md and summarize it" 2>/dev/null > tests/fixtures/gemini-stream-trace.jsonl`
- Commit as test fixture

### Step 1.2: Implement parser
- In `crates/agent-adapter/src/gemini.rs`:
  - `pub struct GeminiStreamParser` with session_id, response_chunks, events, tool_calls, token_usage state
  - `pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent>` — parses one NDJSON line
  - `pub fn finish(self) -> ParsedAgentResult` — returns accumulated result
  - Handles: `init`, `message` (assistant delta), `tool_use`, `tool_result`, `error` (including LoopDetected), `result` (with stats)
  - Unknown types: `tracing::debug!("unknown gemini event type: {}", t)` and skip

### Step 1.3: Wire into main.rs
- In `run_gemini_cli_process_with_session()` (lines 1326-1412):
  - Replace manual parse loop (lines 1372-1399) with GeminiStreamParser
  - Feed each stdout line through `parser.parse_line()`
  - Call `parser.finish()` to get ParsedAgentResult
  - Map back to existing return type: `(result.response_text, result.session_id)`
  - If response_text empty, fall back to raw stdout trimming (REQ-012)

### Step 1.4: Verify
- Unit tests with golden trace fixture
- Verify session_id extraction matches current behavior
- All existing tests pass
- `cargo test`

**Files modified:** `crates/agent-adapter/src/gemini.rs`, `crates/triumvirate/src/main.rs` (~30 lines changed)
**PR scope:** ~200 lines added

---

## Phase 2: Codex Exec-JSON Parser (FEAT-002)

**Goal:** Parse JSONL events from Codex exec stdout. Batch mode (still .output()).

### Step 2.1: Capture golden trace
- Run `codex exec "read README.md and summarize it" --json 2>/dev/null > tests/fixtures/codex-exec-trace.jsonl`
- Commit as test fixture
- Document exact `type` field values observed

### Step 2.2: Implement parser
- In `crates/agent-adapter/src/codex.rs`:
  - `pub struct CodexExecParser` with thread_id, response_chunks, events, tool_calls, token_usage state
  - `pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent>` — same interface as Gemini
  - `pub fn finish(self) -> ParsedAgentResult`
  - Handles event families from binary analysis: `turn.started/completed`, `agent_message*`, `exec_command.begin/end`, `mcp_tool_call.begin/end`, `patch_apply.begin/end`, `TokenCountEvent`, `error`
  - Correlates `*.begin` → `*.end` via `item_id`
  - Unknown types: log and skip

### Step 2.3: Wire into main.rs
- In `run_codex_cli_process_with_session()` (lines 1468-1560):
  - Feed stdout lines through CodexExecParser
  - Last-message file remains primary response source
  - Enrich with token_usage, tool_calls, events from parser
  - Function signature unchanged

### Step 2.4: Verify
- Unit tests with golden trace fixture
- Verify thread_id extraction matches current behavior
- All existing tests pass

**Files modified:** `crates/agent-adapter/src/codex.rs`, `crates/triumvirate/src/main.rs` (~30 lines changed)
**PR scope:** ~200 lines added

---

## Phase 3: Live Streaming (FEAT-003, FEAT-005 partial)

**Goal:** Switch both agents to .spawn() with line-by-line reading. THE KEY CHANGE.

**PR CHECKLIST (MANDATORY — from AR-22):**
- [ ] `process_group(0)` via `.pre_exec(|| unsafe { libc::setpgid(0, 0); Ok(()) })`
- [ ] Kill process group on drop: `killpg(-pid, SIGKILL)`
- [ ] Concurrent stderr drain task (to `tracing::debug!`)
- [ ] Output buffer accumulation for final `ParsedAgentResult`
- [ ] `kill_on_drop(true)` on all spawns
- [ ] Timeout wraps entire spawn+read future
- [ ] `AsyncBufReadExt::lines()` or `LinesCodec`/`FramedRead` (cancellation safe)
- [ ] Bounded mpsc channel (1024) with `try_send()` drop-oldest
- [ ] `TRIUMVIRATE_GEMINI_STREAMING=false` env var escape hatch

### Step 3.1: Rewrite Gemini runner
- In `run_gemini_cli_process_with_session()`:
  - Replace `Command::output().await` with `Command::spawn()`
  - Take stdout, create BufReader, read lines
  - Feed each line through `GeminiStreamParser::parse_line()` (same parser from Phase 1)
  - Send WorkingStateEvents via `mpsc::Sender<WorkingStateEvent>`
  - Spawn stderr drain task: `tokio::spawn(drain_stderr(child.stderr.take()))`
  - Accumulate raw lines for fallback
  - On exit, call `parser.finish()` → ParsedAgentResult

### Step 3.2: Rewrite Codex runner
- Same pattern as Step 3.1 but for Codex
- Uses `CodexExecParser` from Phase 2

### Step 3.3: Modify execute_ask_agent select! loop
- In `execute_ask_agent()` (lines 947-968), add third arm:
  ```rust
  Some(event) = events_rx.recv() => {
      if should_display(&event.state, verbosity) {
          emitter.emit(format_working_state(&event)).await;
      }
      // Real signal — push heartbeat back 30s
      next_heartbeat = started.elapsed() + Duration::from_secs(30);
  }
  ```
- Read `TRIUMVIRATE_AGENT_VERBOSITY` env var at startup
- Heartbeat timer only fires when no real events received

### Step 3.4: Add env var readers
- In `crates/mcp-bridge/src/lib.rs`: `pub fn agent_verbosity() -> AgentVerbosity`
- In `crates/mcp-bridge/src/lib.rs`: `pub fn gemini_streaming_enabled() -> bool`

### Step 3.5: Verify
- Integration test with mock script emitting NDJSON lines with delays
- Verify WorkingStateEvents arrive during execution, not just at end
- Verify stderr drain prevents deadlock
- Verify timeout still works
- Verify env var fallback to batch mode
- All existing tests pass

**Files modified:** `crates/triumvirate/src/main.rs` (~150 lines changed), `crates/mcp-bridge/src/lib.rs` (~20 lines), `crates/agent-adapter/src/gemini.rs` (minor), `crates/agent-adapter/src/codex.rs` (minor)
**PR scope:** ~300 lines changed (highest risk phase)

---

## Phase 4: Stuck Detection + Token Display (FEAT-006, FEAT-007)

**Goal:** Detect stuck agents. Display token counts in completion message.

### Step 4.1: Implement StuckDetector
- In `crates/agent-adapter/src/stuck.rs`:
  - `pub struct StuckDetector` with last_event_ts, tool_call_history, idle_timeout, max_repeats
  - `pub fn observe(&mut self, event: &WorkingStateEvent) -> Option<StuckReason>`
  - Idle detection: no events >60s after TurnStarted
  - Loop detection: same tool + same arguments >5x
  - Input loop: repeated requestUserInput denials
  - Frozen: no events >90s after any event

### Step 4.2: Wire StuckDetector into execute_ask_agent
- Create detector per attempt
- Feed each WorkingStateEvent through detector
- If stuck detected, emit `WorkingState::Stuck` through channel
- Optionally kill subprocess and retry (existing retry loop)

### Step 4.3: Token display in completion message
- In `format_working_state()` for TurnCompleted: include token counts if available
- Format: "responded (1,247 in / 503 out / 200 cached tokens, 3 tools, 8.2s)"

### Step 4.4: Verify
- StuckDetector unit tests with synthetic event streams
- Use `tokio::test(start_paused = true)` for deterministic timeout testing
- Verify idle detection, loop detection, input loop detection
- Verify token counts display correctly

**Files modified:** `crates/agent-adapter/src/stuck.rs`, `crates/agent-adapter/src/lib.rs`, `crates/triumvirate/src/main.rs` (~50 lines)
**PR scope:** ~200 lines added

---

## Phase 5: ask_twins Removal (FEAT-008)

**Goal:** Clean the tool surface. ~750 lines deleted.

### Step 5.1: Delete from main.rs
- Delete `#[tool] ask_twins` (lines 516-566)
- Delete `execute_ask_twins()` (lines 1176-1439)
- Delete `ask_twins_route()` (lines 2158-2181)
- Delete route registration `.route("/ask-twins", ...)` (line 2516)
- Delete `fetch_daemon_ask_twins()` (lines 2739-2741)
- Delete 5 test functions (~400 lines)
- Clean imports: remove `AskTwinsRequest`, `AskTwinsResponse`, `AgentResult` (old), `daemon_ask_twins_url`

### Step 5.2: Delete from shared-types
- Delete `AskTwinsRequest` (lines 30-35)
- Delete `AskTwinsResponse` (lines 44-50)
- Delete `AgentResult` (lines 37-42) — replaced by `ParsedAgentResult` in agent-adapter

### Step 5.3: Delete from mcp-bridge
- Delete `build_role_adapted_prompts()` (lines 21-31)
- Delete `daemon_ask_twins_url()` (lines 94-97)
- Remove `AskTwinsRequest` from import (line 6)
- Delete `prompt_builder_includes_role_labels` test (lines 197-207)

### Step 5.4: Scrub stale references
- README.md: remove `TRIUMVIRATE_DAEMON_ASK_TWINS_URL` documentation
- Any env var docs mentioning ask_twins
- Any test fixtures referencing ask_twins

### Step 5.5: Verify
- `cargo check` — no compile errors from removed types
- `cargo test` — all remaining tests pass
- Grep for "ask_twins" across entire workspace — zero results

**Files modified:** `crates/triumvirate/src/main.rs`, `crates/shared-types/src/lib.rs`, `crates/mcp-bridge/src/lib.rs`, `README.md`
**PR scope:** ~750 lines deleted, ~10 lines changed

---

## Phase 6: Polish + Tests (FEAT-010, REQ-021, REQ-027)

**Goal:** Version telemetry, comprehensive tests, progress.txt update.

### Step 6.1: Version telemetry
- Add `cli_version: Option<String>` and `parser_mode: String` to ParsedAgentResult
- Populate from `init` event (Gemini) and startup (Codex)

### Step 6.2: Integration tests
- Mock Gemini script emitting stream-json NDJSON with tool calls and delays
- Mock Codex script emitting exec --json JSONL with commands and delays
- Verify end-to-end: MCP call → live events → formatted progress messages
- Verify fallback paths

### Step 6.3: Update progress.txt
- Mark v2.1 Flow State as shipped

**Files modified:** `crates/agent-adapter/src/types.rs`, new test files, `docs/v2.1/progress.txt`
**PR scope:** ~150 lines

---

## Dependency Graph

```
Phase 0 (types + crate)
  |
  +---> Phase 1 (Gemini parser)
  |       |
  +---> Phase 2 (Codex parser) --- both needed for Phase 3
  |       |
  +-------+---> Phase 3 (live streaming) <--- THE KEY
  |               |
  +---> Phase 4 (stuck detection + tokens) --- needs Phase 3
  |
  +---> Phase 5 (ask_twins removal) --- independent
  |
  +---> Phase 6 (polish + tests) --- needs all above
```

**Critical path:** 0 → 1 → 2 → 3 → 4 → 6
**Parallel track:** 5 (independent, any time)

---

## Estimated Scope

| Phase | Lines Added | Lines Deleted | Lines Changed | Risk |
|-------|------------|--------------|--------------|------|
| 0 | ~200 | 0 | ~5 | Low |
| 1 | ~200 | 0 | ~30 | Low |
| 2 | ~200 | 0 | ~30 | Low |
| 3 | ~100 | ~60 | ~150 | **High** |
| 4 | ~200 | 0 | ~50 | Medium |
| 5 | ~10 | ~750 | ~5 | Low |
| 6 | ~150 | 0 | ~20 | Low |
| **Total** | **~1,060** | **~810** | **~290** | |

**Net: ~250 lines added.** The bulk of the work is restructuring, not growing.
