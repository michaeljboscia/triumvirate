# Triumvirate v3.0.1 + v3.0.2 — Observability & Token Economics

**Version:** Combined Sprint Spec — Round 1 Updated
**Date:** 2026-04-08
**GitHub Issues:** #19, #20, #21 (v3.0.1), #23 (v3.0.2)
**Goat Rodeo:** Round 1 complete (14 auto-resolved, 3 user decisions)
**Codebase State:** ABE v3.0 shipped, stress test Phases 1-4 passed, daemon at commit `1ca4f58`

---

## Problem Statement

The Triumvirate daemon shipped ABE v3.0 with 1,932 lines of Rust and zero observability. No tracing spans, no ABE-specific metrics, no structured logging. Four Prometheus metrics are declared but never updated. The daemon captures token counts from agent responses but the GeminiStreamParser only extracts input/output/cached/total from the `stream-json` `result` event — ignoring thinking tokens, latency, tool calls, and lines changed. There is no way to know what a build costs, which agent burns the most tokens, or where time is spent.

You can't optimize, debug, or trust what you can't see.

---

## Constitution (Unchanged)

1. Claude is the front door. User talks to Claude.
2. Lifecycle is always visible. No silent failures.
3. Plain language in, structured results out. No command ceremony.
4. Failure is loud, immediate, and actionable.

---

## What This Covers

### Workstream 1 — Observability (v3.0.1)

Instrument every ABE function, wire dead metrics, add ABE-specific metrics, add structured log events, add error context, wire missing WebSocket events, instrument the ledger crate, and move agent metrics inside the execution path so MCP-origin requests are measured.

### Workstream 2 — Token Economics (v3.0.2)

Build a Rust-native token scanner that reads Claude, Codex, and Gemini session logs, stores token counts in SQLite, attributes costs to ABE tasks/waves/builds via BUILD_MANIFEST, and serves a cost dashboard through the daemon's existing Axum HTTP server and WebSocket broadcast.

### What This Does NOT Cover

- Dashboard UI redesign (#12)
- Daemon Rust rewrite of non-ABE modules (#13)
- Gemini enforcement gate (#22)
- Notification system (#14)
- Remaining stress test phases 5-7

---

## Requirements — Workstream 1: Observability

### Tracing Spans

**REQ-O1:** Every public function in `daemon/crates/triumvirate/src/abe/*.rs` (17 functions across 9 files) must have a `#[instrument(skip_all)]` attribute with structured fields: `task_id`, `wave`, `status`, `elapsed` where applicable.

**REQ-O2:** Every public function in `daemon/crates/ledger/src/*.rs` must have `#[instrument(skip_all)]` with fields: `event_type`, `spool_size`, `operation`.

**REQ-O3:** `agent_exec::execute_ask_agent` must have an `#[instrument]` span that creates a parent trace context linking the MCP request through daemon dispatch to agent execution and back.

### Prometheus Metrics — New

**REQ-O4:** New counter `abe_task_dispatch_total{status}` incremented on every task dispatch. Labels: `dispatched`, `completed`, `failed`, `timeout`, `cancelled`.

**REQ-O5:** New histogram `abe_task_duration_seconds{wave}` recording wall-clock time from dispatch to completion/failure. `task_id` is recorded in structured logs and exemplars only (not as a Prometheus label) to avoid cardinality explosion. Per-task cost data lives in the token-economics SQLite DB. (Decision R1-D1: B)

**REQ-O6:** New histogram `abe_wave_duration_seconds{wave}` recording wall-clock time from first task dispatch to wave gate pass for each wave.

**REQ-O7:** New counter `abe_timeout_total` incremented on every SIGTERM or SIGKILL event in `codex_spawn::enforce_timeout`.

**REQ-O8:** New histogram `abe_worktree_setup_duration_seconds` recording time spent in `worktree_setup::setup_worktree`.

**REQ-O9:** New counter `abe_failure_class_total{class}` incremented per failure classification. Labels: `worker-error`, `contract-error`, `orchestrator-briefing-error`, `environment-error`.

**REQ-O10:** New counter `abe_retry_total{class}` incremented per retry dispatch, labeled by failure class.

**REQ-O11:** New counter `abe_validation_total{result}` incremented per post-exit validation. Labels: `pass`, `fail_scope`, `fail_format`, `fail_stub`, `fail_test`.

### Prometheus Metrics — Wire Dead

**REQ-O12:** `agent_tokens_total` must be updated from `ParsedAgentResult.token_usage` inside `execute_ask_agent`, not at the HTTP handler level. This ensures MCP-origin and ABE-origin requests are counted.

**REQ-O13:** `fleet_active_total` must reflect the actual count of running fleet orchestrations, updated on fleet spawn and completion.

**REQ-O14:** `reviews_total` must be incremented on every peer review completion, not just bootstrap.

**REQ-O15:** `ledger_spool_size_bytes` must be updated on every spool drain cycle by reading the spool directory size.

### Structured Logging

**REQ-O16:** `tracing::info!` events emitted on: ABE dispatch start (with task_id, wave, allowed_files count), task completed (with task_id, commit_sha, duration), wave started (with wave number, task count), wave gate passed (with wave number, test result, review verdict).

**REQ-O17:** `tracing::warn!` events emitted on: timeout triggered (with task_id, timeout_sec, signal sent), retry dispatched (with task_id, failure_class, attempt number), collateral fix detected (with task_id, extra files).

**REQ-O18:** `tracing::error!` events emitted on: worktree setup failed (with project_root, error), environment error (with task_id, error), escalation to human (with task_id, reason, all briefings attached as structured field).

### Error Context

**REQ-O19:** Critical I/O paths and failure boundaries in ABE modules must use `.with_context(|| format!("..."))` including the file path, operation name, and task_id where available. Focus on: file read/write, subprocess spawn, JSON parse, and git operations. Not every single `?` — measure by grepping for bare `.unwrap()` and uncontexted `?` after sprint. (Scoped down per twin consensus)

### WebSocket Events

**REQ-O20:** New WebSocket event `abe_task_state` emitted on every ABE task state transition (dispatched, running, completed, failed, timeout, cancelled). Payload: `{ task_id, wave, status, duration_ms, commit_sha }`.

**REQ-O21:** New WebSocket event `abe_wave_state` emitted on wave start and wave gate completion. Payload: `{ wave, status, task_count, duration_ms }`.

**REQ-O22:** Existing `fleet_progress` event must emit on every fleet state transition, not just bootstrap.

### Hotfix

**REQ-O23:** `worktree_setup::setup_worktree` must run `git config extensions.worktreeConfig true` on the project root before setting `core.hooksPath`. This is idempotent. (#19)

---

## Requirements — Workstream 2: Token Economics

### Scanner

**REQ-T1:** A new crate `daemon/crates/token-economics` depends on `tokscale-core` (git dependency from `junhoyeo/tokscale`, MIT license) for all file scanning, parsing, and pricing. Two ingestion lanes: (a) a direct-write API `record_daemon_tokens(record: TokenRecord)` called by `agent_exec` for daemon-mediated sessions (exact attribution, zero delay), and (b) a background scanner wrapping `tokscale-core::scanner` for external CLI sessions across ALL agents tokscale supports (Claude, Codex, Gemini, Cursor, +13 more). We build the attribution engine and daemon integration; tokscale-core handles parsing and pricing. (Decision R2: build on tokscale, not from scratch)

**REQ-T2:** Claude session scanning delegated to `tokscale-core` which reads `~/.claude/projects/**/*.jsonl` with SIMD-accelerated JSON parsing and rayon parallelism. Our wrapper adds: session-ID extraction for attribution, direct SQLite write with build/task correlation, and notify-based file watching. (Delegated to tokscale-core)

**REQ-T3:** Codex session scanning delegated to `tokscale-core` which reads `~/.codex/sessions/` with the same infrastructure. Our wrapper adds session-ID extraction and attribution. (Delegated to tokscale-core)

**REQ-T4:** Gemini session scanning delegated to `tokscale-core` which reads `~/.gemini/tmp/*/chats/*.json`. ADDITIONALLY, our wrapper reads `~/.gemini/telemetry.jsonl` (536MB, 5.1M lines) which contains richer data including `thoughtsTokenCount` and per-modality breakdowns not available in the chat files. Incremental by byte offset. Both sources are merged — tokscale's chat files for session structure, telemetry.jsonl for detailed token breakdowns. (Decision R1-D3 + R2 tokscale integration)

**REQ-T5:** Extend `GeminiStreamParser` (agent-adapter/src/gemini.rs:138-148) to parse additional fields from the `stream-json` `result` event: `thoughtsTokenCount` (thinking tokens), `totalLatencyMs` (latency), `tools.totalCalls` (tool calls), `files.linesAdded/Removed`. No mode switch needed — `stream-json` already emits these in the final `result` event, the parser just ignores them. Extend `TokenUsage` struct (types.rs) with `thinking_tokens: Option<u64>`, `latency_ms: Option<u64>`, `tool_calls: Option<u64>`. (Research confirmed: stream-json emits final stats)

### Storage

**REQ-T6:** Token records stored in SQLite via `rusqlite` (version 0.32, matching existing crate usage). Schema:

```sql
CREATE TABLE token_records (
    id INTEGER PRIMARY KEY,
    agent TEXT NOT NULL,          -- 'claude' | 'codex' | 'gemini'
    session_id TEXT NOT NULL,
    timestamp TEXT NOT NULL,       -- ISO 8601
    model TEXT,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cached_tokens INTEGER DEFAULT 0,
    thinking_tokens INTEGER DEFAULT 0,
    total_tokens INTEGER NOT NULL,
    cost_usd REAL,
    -- Gemini-specific
    latency_ms INTEGER,
    tool_calls INTEGER,
    lines_added INTEGER,
    lines_removed INTEGER,
    -- Codex-specific
    rate_limit_pct REAL,
    context_window INTEGER,
    -- ABE attribution
    build_id TEXT,
    task_id TEXT,
    wave INTEGER
);

CREATE TABLE scan_state (
    file_path TEXT PRIMARY KEY,
    last_mtime INTEGER NOT NULL,
    last_offset INTEGER NOT NULL
);
```

**REQ-T7:** Database file location: `~/.triumvirate/token-economics.db`. Created on first scan if it does not exist.

### Cost Attribution

**REQ-T8:** Attribution uses session-ID correlation, not timestamps. The daemon records spawned session IDs in outbox events. For daemon-mediated sessions: `agent_exec` writes `TokenRecord` directly with exact `build_id`, `task_id`, and `session_id`. For external CLI sessions: scanner matches session IDs from outbox events against file session IDs. Truly external sessions (no matching outbox entry) go to an "unattributed" bucket. No timestamp-window guessing. (Decision R1-D2: C)

**REQ-T9:** Cost calculation uses a temporal price table stored in the database:

```sql
CREATE TABLE price_table (
    id INTEGER PRIMARY KEY,
    model TEXT NOT NULL,
    input_per_mtok REAL NOT NULL,
    output_per_mtok REAL NOT NULL,
    cached_per_mtok REAL DEFAULT 0,
    effective_date TEXT NOT NULL,
    end_date TEXT  -- NULL = current price
);
CREATE INDEX idx_price_model_date ON price_table (model, effective_date);
```

Query: `WHERE model = ? AND effective_date <= ?ts AND (end_date IS NULL OR end_date > ?ts)`. When prices change: close old row (`UPDATE SET end_date`), insert new row. Cost is calculated at scan time using the price active at the session's timestamp and stored in `cost_usd`. Default prices seeded on first run. User can update via MCP tool. (Decision R1: temporal pattern per research)

### HTTP API

**REQ-T10:** New Axum route `GET /api/tokens/summary` returns JSON: total tokens by agent, total cost, time range. Query params: `?since=ISO8601&until=ISO8601&agent=claude|codex|gemini`.

**REQ-T11:** New Axum route `GET /api/tokens/by-build` returns JSON: per-build cost breakdown with task-level attribution. Query param: `?build_id=abe-v3-main`.

**REQ-T12:** New Axum route `GET /api/tokens/by-session` returns JSON: per-session token breakdown. Query param: `?session_id=xxx`.

### WebSocket

**REQ-T13:** New WebSocket event `token_update` emitted after each scan cycle. Payload: `{ agent, session_id, tokens_added, total_cost_usd, scan_duration_ms }`.

### MCP Tools

**REQ-T14:** New MCP tool `get_token_summary` returning the same data as `GET /api/tokens/summary`. Accessible from Claude sessions via the MCP bridge.

**REQ-T15:** New MCP tool `get_build_cost` returning per-build cost breakdown. Input: `build_id`. Accessible from Claude sessions.

### Scanner Lifecycle

**REQ-T16:** The scanner runs as a background tokio task within the daemon. For Claude and Codex: uses `notify` crate filesystem watcher for real-time detection of new/modified files, with a 10-minute periodic reconciliation fallback. For Gemini: tracks byte offset in `~/.gemini/telemetry.jsonl` (single file, append-only). Does not block the main event loop. No 60-second glob polling. (Decision R1: event-driven per twin consensus)

**REQ-T17:** On daemon startup, the scanner performs a full reconciliation scan (all files, not just changed ones) to catch any sessions that occurred while the daemon was down. Runs as an async tokio task — does not block daemon boot or HTTP readiness. (Decision R1: async startup per Codex flag)

---

## Architecture Notes

### ObservabilityBus (Round 1 — Architectural Addition)

McpBridge (where ABE MCP tools live) currently has NO access to `DaemonMetrics` or `ws_events`. Both are needed for REQ-O4–O11, O20, O21, T14, T15. Solution: create an `ObservabilityBus` struct wrapping shared observability state, injected into both DaemonState (for HTTP routes) and McpBridge (for MCP tools).

```rust
pub struct ObservabilityBus {
    pub metrics: Arc<DaemonMetrics>,
    pub ws_events: broadcast::Sender<String>,
    pub token_db: Arc<TokenDb>,  // SQLite connection for token-scanner
}
```

Constructed once in `main()`, cloned into DaemonState and McpBridge. All ABE functions that need metrics/events receive a reference to the bus.

### Token Ingestion Architecture (Round 1 — Dual Lane)

```
Lane 1 (Daemon-mediated):
  agent_exec → TokenRecord (exact build_id, task_id, session_id) → SQLite direct write

Lane 2 (External CLI sessions):
  notify watcher → detect new/modified files → parse JSONL → 
  match session_id against outbox events → attributed or "unattributed" → SQLite batch write
```

### Crate Dependency Graph (Post-Sprint)

```
triumvirate (main binary)
├── daemon-core
├── daemon-http
├── agent-adapter
├── agent-worker
├── mcp-bridge
├── mcp-tools
├── fleet
├── ledger
├── peer-review
├── shared-types
├── fallback-outbox
└── token-scanner (NEW)
    ├── rusqlite 0.32
    ├── serde + serde_json
    ├── shared-types (for TokenUsage, BuildState)
    ├── anyhow
    └── tracing
```

### File Inventory (Existing — Must Modify)

| File | Changes |
|------|---------|
| `daemon/crates/triumvirate/src/abe/worktree_setup.rs` | #[instrument], REQ-O23 worktreeConfig, REQ-O8 metric, REQ-O19 context |
| `daemon/crates/triumvirate/src/abe/codex_spawn.rs` | #[instrument], REQ-O7 timeout metric, REQ-O16-O18 log events |
| `daemon/crates/triumvirate/src/abe/task_tracker.rs` | #[instrument], REQ-O4 dispatch metric, REQ-O20 WS event |
| `daemon/crates/triumvirate/src/abe/orchestrator.rs` | #[instrument], REQ-O5/O6 duration metrics, REQ-O16-O18 events |
| `daemon/crates/triumvirate/src/abe/failure_handler.rs` | #[instrument], REQ-O9/O10 failure metrics |
| `daemon/crates/triumvirate/src/abe/build_artifacts.rs` | #[instrument], REQ-O19 context |
| `daemon/crates/triumvirate/src/abe/resume.rs` | #[instrument] |
| `daemon/crates/triumvirate/src/abe/wave_gate.rs` | #[instrument], REQ-O6 wave metric, REQ-O21 WS event |
| `daemon/crates/triumvirate/src/abe/post_exit_validator.rs` | #[instrument], REQ-O11 validation metric |
| `daemon/crates/triumvirate/src/agent_exec.rs` | REQ-O3 parent span, REQ-O12 token metric, REQ-T5 Gemini stats capture |
| `daemon/crates/triumvirate/src/main.rs` | New metrics registration, new HTTP routes, WS events, scanner task spawn |
| `daemon/crates/ledger/src/*.rs` | REQ-O2 #[instrument] on all public fns |
| `daemon/crates/daemon-http/src/lib.rs` | REQ-O22 fleet_progress wiring |

### File Inventory (New)

| File | Purpose |
|------|---------|
| `daemon/crates/token-economics/Cargo.toml` | New crate — depends on `tokscale-core` (git), `rusqlite`, `notify`, `shared-types` |
| `daemon/crates/token-economics/src/lib.rs` | Public API: `TokenEconomics` struct, `record_daemon_tokens()`, scanner lifecycle |
| `daemon/crates/token-economics/src/direct.rs` | Lane 1: daemon direct write from `agent_exec` |
| `daemon/crates/token-economics/src/scanner.rs` | Lane 2: wraps `tokscale-core::scanner` + notify watcher + telemetry.jsonl reader |
| `daemon/crates/token-economics/src/attribution.rs` | Session-ID correlation, BUILD_MANIFEST join, build/task/wave cost mapping |
| `daemon/crates/token-economics/src/storage.rs` | SQLite schema (token_records, scan_state, price_table), migrations, WAL mode |
| `daemon/crates/token-economics/src/queries.rs` | Summary, by-build, by-session query functions for HTTP + MCP |

---

## Build Strategy: ABE Fleet Dogfood

This build will be dispatched through ABE's own `dispatch_codex_worktree` system. The plan maps to ABE's wave/task model:

- **Wave 0:** Contracts — shared types, trait definitions, schema SQL
- **Wave 1:** Observability infrastructure — metrics struct, tracing spans, error context
- **Wave 2:** Observability wiring — dead metrics, WS events, ledger instrumentation
- **Wave 3:** Token scanner core — parsers, storage, incremental scan
- **Wave 4:** Token scanner integration — HTTP routes, MCP tools, WS events, daemon lifecycle
- **Wave 5:** Integration — scanner startup, full reconciliation, attribution engine

Each wave's tasks are parallel-safe within the wave. Workers get contracts with `allowed_files` scoped to their task. The daemon validates every commit against the contract.

**max_parallel: 7** (proven in stress test)

---

## Acceptance Criteria

1. `cargo test --workspace` passes after all changes
2. `curl localhost:8080/metrics` returns non-zero values for all declared metrics after a single ABE dispatch
3. Daemon log shows structured JSON entries for every ABE dispatch lifecycle step
4. WebSocket stream includes `abe_task_state`, `abe_wave_state`, and `token_update` events
5. `GET /api/tokens/summary` returns token counts from all three agents after scanner runs
6. `GET /api/tokens/by-build?build_id=abe-v3-main` returns per-task cost attribution
7. `get_token_summary` MCP tool returns data accessible from Claude session
8. Scanner reconciliation on daemon restart catches sessions from downtime period
