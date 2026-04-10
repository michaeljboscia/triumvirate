# Test Plan — v3.2.0 Observability & Token Economics

**Spec:** specs/OBSERVABILITY_TOKEN_ECONOMICS.md
**Produced:** 2026-04-10 (retroactively, during postrodeo — /uncompromising-executor was skipped)

---

## Observability — Tracing Spans (REQ-O1, REQ-O2, REQ-O3)

### TP-O1-01: ABE functions have #[instrument] spans
- **Method:** `grep -rc '#\[instrument' daemon/crates/triumvirate/src/abe/`
- **Pass:** Count >= 30 (every public fn in 10 ABE files)
- **Verify with traces:** Set `RUST_LOG=triumvirate=trace`, dispatch an ABE task, confirm span hierarchy in structured JSON log

### TP-O1-02: Spans include structured fields
- **Method:** Grep for `fields(task_id` in ABE instrument attributes
- **Pass:** At least 15 spans include task_id field

### TP-O2-01: Ledger functions have #[instrument] spans
- **Method:** `grep -rc '#\[instrument' daemon/crates/ledger/src/`
- **Pass:** Count >= 20

### TP-O3-01: execute_ask_agent has parent trace span
- **Method:** `grep '#\[instrument' daemon/crates/triumvirate/src/agent_exec.rs`
- **Pass:** At least 1 instrument attribute with agent, session_id fields

### TP-O3-02: Agent request traces are visible end-to-end
- **Method:** Call `ask_agent(agent: "codex", message: "echo trace test")` with `RUST_LOG=trace`
- **Pass:** Daemon log shows nested spans: ask_agent → execute_ask_agent → codex response

## Observability — Prometheus Metrics (REQ-O4 through REQ-O15)

### TP-O4-01: abe_task_dispatch_total increments on dispatch
- **Method:** Dispatch a task via `dispatch_codex_worktree`, then `curl /metrics`
- **Pass:** `triumvirate_abe_task_dispatch_total{status="dispatched"}` > 0

### TP-O5-01: abe_task_duration_seconds records wall-clock time
- **Method:** Dispatch a task, wait for completion, `curl /metrics`
- **Pass:** `triumvirate_abe_task_duration_seconds` histogram has at least 1 observation

### TP-O7-01: abe_timeout_total increments on timeout
- **Method:** Dispatch a task with task_timeout_sec=5 and a worker that sleeps 30s
- **Pass:** `triumvirate_abe_timeout_total` > 0 after timeout fires

### TP-O8-01: abe_worktree_setup_duration_seconds records setup time
- **Method:** Dispatch any task, `curl /metrics`
- **Pass:** Histogram has >= 1 observation

### TP-O11-01: abe_validation_total records validation results
- **Method:** Dispatch a task that passes validation, `curl /metrics`
- **Pass:** `triumvirate_abe_validation_total{result="pass"}` > 0

### TP-O12-01: agent_tokens_total increments on ask_agent
- **Method:** Call `ask_agent`, then `curl /metrics`
- **Pass:** `triumvirate_agent_tokens_total` > 0

### TP-O13-01: fleet_active_total reflects running fleets
- **Method:** `fleet_spawn(...)`, then `curl /metrics`
- **Pass:** `triumvirate_fleet_active_total` >= 1

### TP-O14-01: reviews_total increments on review completion
- **Method:** `review_request(...)`, then `curl /metrics`
- **Pass:** `triumvirate_reviews_total` > 0

### TP-O15-01: ledger_spool_size_bytes updates on drain cycle
- **Method:** Write to spool, wait for drain, `curl /metrics`
- **Pass:** `triumvirate_ledger_spool_size_bytes` reflects actual spool size

## Observability — Structured Logging (REQ-O16, REQ-O17, REQ-O18)

### TP-O16-01: ABE dispatch start logged
- **Method:** Dispatch a task, grep daemon.log for `abe_dispatch_start` or `task_id`
- **Pass:** JSON log entry with task_id, wave, allowed_files count

### TP-O16-02: Task completion logged
- **Method:** After task completes, grep for `abe_task_completed`
- **Pass:** JSON entry with task_id, commit_sha, duration_ms

### TP-O17-01: Timeout logged as warning
- **Method:** Dispatch a task that times out, grep for WARN level
- **Pass:** Warning entry with task_id, timeout_sec, signal info

### TP-O18-01: Worktree setup failure logged as error
- **Method:** Dispatch with invalid SHA, grep for ERROR level
- **Pass:** Error entry with project_root, error description

## Observability — Error Context (REQ-O19)

### TP-O19-01: I/O errors include file path context
- **Method:** `grep -rc '.with_context\|.context(' daemon/crates/triumvirate/src/abe/`
- **Pass:** >= 10 context annotations across ABE I/O paths

### TP-O19-02: Bare unwrap count reduced
- **Method:** `grep -rc '.unwrap()' daemon/crates/triumvirate/src/abe/`
- **Pass:** Count lower than pre-sprint baseline (measure before/after)

## Observability — WebSocket Events (REQ-O20, REQ-O21, REQ-O22)

### TP-O20-01: abe_task_state event emitted on task state change
- **Method:** Connect to ws://localhost:8080/ws, dispatch a task, listen for events
- **Pass:** Receive `abe_task_state` event with task_id, wave, status, duration_ms

### TP-O21-01: abe_wave_state event emitted on wave start/complete
- **Method:** Same WS connection during a multi-task wave
- **Pass:** Receive `abe_wave_state` with wave number, status, task_count

### TP-O22-01: fleet_progress event emitted on fleet state changes
- **Method:** Spawn a fleet, listen on WS
- **Pass:** Receive `fleet_progress` events (not just on bootstrap)

## Observability — Hotfix (REQ-O23)

### TP-O23-01: worktreeConfig auto-enabled
- **Method:** `grep 'extensions.worktreeConfig' daemon/crates/triumvirate/src/abe/worktree_setup.rs`
- **Pass:** `git config extensions.worktreeConfig true` call present before hooksPath set

## Token Economics — Scanner (REQ-T1, REQ-T2, REQ-T3, REQ-T4)

### TP-T1-01: token-economics crate compiles
- **Method:** `cargo check -p token-economics --manifest-path daemon/Cargo.toml`
- **Pass:** Exit 0

### TP-T2-01: Claude session scanner parses JSONL
- **Method:** Create mock JSONL with known token counts, run `scan_claude_file`
- **Pass:** Returns TokenRecords matching expected counts

### TP-T3-01: Codex session scanner parses JSONL
- **Method:** Create mock Codex JSONL, run `scan_codex_file`
- **Pass:** Returns TokenRecords matching expected counts

### TP-T4-01: Gemini scanner parses chat JSON
- **Method:** Create mock Gemini chat JSON, run `scan_gemini_chat_file`
- **Pass:** Returns TokenRecords

### TP-T4-02: Gemini telemetry scanner parses telemetry.jsonl
- **Method:** Create small mock telemetry.jsonl (10 lines), run `scan_gemini_telemetry_file`
- **Pass:** Returns TokenRecords with thinking_tokens populated

## Token Economics — Extended TokenUsage (REQ-T5)

### TP-T5-01: TokenUsage has new fields
- **Method:** `grep 'thinking_tokens\|latency_ms\|tool_calls' daemon/crates/shared-types/src/lib.rs`
- **Pass:** All 3 fields present as `Option<u64>`

### TP-T5-02: GeminiStreamParser extracts new fields
- **Method:** Feed parser a mock stream-json result event with thoughtsTokenCount
- **Pass:** Parsed TokenUsage has thinking_tokens populated

## Token Economics — Storage (REQ-T6, REQ-T7)

### TP-T6-01: SQLite schema creates 3 tables
- **Method:** `cargo test -p token-economics` (storage tests)
- **Pass:** token_records, scan_state, price_table all created

### TP-T6-02: Insert + query round-trip
- **Method:** Insert a TokenRecord, query it back
- **Pass:** All fields match

### TP-T7-01: Database at ~/.triumvirate/token-economics.db
- **Method:** Start daemon, check file exists
- **Pass:** `test -f ~/.triumvirate/token-economics.db`

## Token Economics — Attribution (REQ-T8, REQ-T9)

### TP-T8-01: Session-ID correlation matches outbox entries
- **Method:** Create TokenRecords + mock outbox events with matching session_ids
- **Pass:** Attribution populates build_id and task_id from outbox

### TP-T8-02: Unmatched sessions go to "unattributed"
- **Method:** Create TokenRecord with session_id not in outbox
- **Pass:** build_id = "unattributed"

### TP-T9-01: Cost calculated from price table
- **Method:** Insert price entry, run attribution on TokenRecord
- **Pass:** cost_usd = (input * input_rate + output * output_rate) / 1M

## Token Economics — HTTP API (REQ-T10, REQ-T11, REQ-T12)

### TP-T10-01: GET /api/tokens/summary returns token data
- **Method:** Insert test data, `curl http://localhost:8080/api/tokens/summary`
- **Pass:** JSON with agent breakdown, total cost, time range

### TP-T10-02: Summary filters by agent
- **Method:** `curl /api/tokens/summary?agent=claude`
- **Pass:** Only Claude tokens in response

### TP-T11-01: GET /api/tokens/by-build returns per-task breakdown
- **Method:** Insert test data with build_id, `curl /api/tokens/by-build?build_id=test`
- **Pass:** JSON with task-level cost attribution

### TP-T12-01: GET /api/tokens/by-session returns session breakdown
- **Method:** `curl /api/tokens/by-session?session_id=test`
- **Pass:** JSON with token fields for that session

## Token Economics — MCP Tools (REQ-T14, REQ-T15)

### TP-T14-01: get_token_summary returns data via MCP
- **Method:** `mcp__triumvirate__get_token_summary()`
- **Pass:** Returns JSON matching /api/tokens/summary shape

### TP-T15-01: get_build_cost returns per-build cost via MCP
- **Method:** `mcp__triumvirate__get_build_cost(build_id: "test")`
- **Pass:** Returns JSON with task-level cost data

## Token Economics — Scanner Lifecycle (REQ-T13, REQ-T16, REQ-T17)

### TP-T13-01: token_update WS event emitted after scan
- **Method:** Connect to ws://localhost:8080/ws, trigger a scan (create a new agent session file)
- **Pass:** Receive `token_update` event with agent, tokens_added, scan_duration_ms

### TP-T16-01: Scanner runs as background task
- **Method:** Start daemon, verify scanner is running (check log for scan activity)
- **Pass:** Daemon log shows scan-related spans within 60s of boot

### TP-T16-02: File watcher detects new session files
- **Method:** Create a new file in ~/.claude/projects/test/*.jsonl, wait 10s
- **Pass:** Scanner picks up the file and emits token_update event

### TP-T17-01: Startup reconciliation runs on boot
- **Method:** Start daemon fresh, check log for reconciliation activity
- **Pass:** Log shows reconciliation scan within 30s of boot

### TP-T17-02: Reconciliation does NOT block HTTP readiness
- **Method:** Start daemon, immediately curl /health
- **Pass:** /health responds within 5s of daemon start (even if reconciliation is still running)

---

## Execution Notes

- Tests requiring agent responses (ask_agent, dispatch, fleet) need live Codex/Gemini access
- HTTP tests need daemon running with correct port (check TRIUMVIRATE_DAEMON_BIND_ADDR)
- Bearer token from ~/.triumvirate/daemon.token required for all HTTP routes
- WS tests require a WebSocket client (websocat, wscat, or browser)
- Scanner tests should use mock files in a temp directory, not production session dirs
- The 536MB telemetry.jsonl test (TP-T17-02) specifically validates the non-blocking hotfix
- With v3.2.0 tracing spans active, ALL test failures will produce structured log context — grep daemon.log for the test's task_id/session_id to diagnose
