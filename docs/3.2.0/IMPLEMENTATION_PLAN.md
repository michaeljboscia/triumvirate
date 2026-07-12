# Triumvirate 3.2.0 Implementation Plan — Observability & Token Economics

**Spec:** `specs/OBSERVABILITY_TOKEN_ECONOMICS.md`
**Target version:** `3.2.0`
**Baseline:** tag `3.1.0` (commit `53a5583`)
**Working Directory:** `/Users/you/projects/triumvirate`
**Git Branch:** `main`

---

## Build Overview

| Wave | Tasks | Description |
|------|-------|-------------|
| Preflight (Wave -1) | T-100 | Version bump 3.1.0 → 3.2.0 |
| Wave 0 | T-101, T-102 | Contracts: new metrics struct fields + token-economics crate scaffold |
| Wave 1 | T-103, T-104, T-105, T-106, T-107 | Observability: #[instrument] spans + new metrics + dead metrics wiring |
| Wave 2 | T-108, T-109, T-110 | Observability: structured logging + error context + WebSocket events |
| Wave 3 | T-111, T-112, T-113, T-114 | Token economics: scanner core + storage + attribution + parsers |
| Wave 4 | T-115, T-116, T-117 | Token economics: HTTP API + MCP tools + scanner lifecycle |
| Wave 5 | T-118, T-119, T-120, T-121, T-122, T-123 | Integration + public release |

**Total: 24 tasks across 7 waves.**

**Task ID numbering:** T-1xx to avoid collision with v3.1.0's T-0xx series.

**Dispatch strategy:** Same as v3.1.0 — `dispatch_codex_worktree` with three-ceremony closing block, sentinel file polling, manual cherry-pick (daemon reaper still broken). Parallel dispatch within each wave. Apply v3.1.0 lessons: Rule A (2-round audit cap), Rule B (empirical verification before dispatch), `--manifest-path daemon/Cargo.toml` on every cargo command, `cargo test -p <crate>` not `cargo test --workspace`.

---

## Preflight (Wave -1)

<task id="T-100" req="PREFLIGHT" wave="-1" depends="">
  <description>Bump Cargo workspace version from 3.1.0 to 3.2.0 and update ROADMAP.md</description>
  <files>daemon/Cargo.toml, daemon/Cargo.lock, ROADMAP.md</files>
  <scope_out>Do not touch any Rust source files. Do not touch docs/3.1.0/ (that sprint is archived). Version propagation is automatic via version.workspace = true and env!("CARGO_PKG_VERSION").</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep '^version = "3.2.0"' daemon/Cargo.toml exits 0. ./daemon/target/release/triumvirate --version prints 3.2.0. ROADMAP.md shows 3.2.0 as current.</reality_test>
  <done_when>Workspace at 3.2.0. ROADMAP.md updated. Binary prints 3.2.0.</done_when>
</task>

---

## Wave 0: Contracts

<task id="T-101" req="REQ-O4,REQ-O5,REQ-O6,REQ-O7,REQ-O8,REQ-O9,REQ-O10,REQ-O11" wave="0" depends="">
  <description>Add 8 new ABE Prometheus metric fields to DaemonMetrics in daemon-core/src/metrics.rs and register them</description>
  <files>daemon/crates/daemon-core/src/metrics.rs, daemon/Cargo.lock</files>
  <scope_out>Do not instrument any ABE functions yet (Wave 1). Just define and register the metrics. Do not touch main.rs or any handler code.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -c 'abe_task_dispatch_total\|abe_task_duration_seconds\|abe_wave_duration_seconds\|abe_timeout_total\|abe_worktree_setup_duration_seconds\|abe_failure_class_total\|abe_retry_total\|abe_validation_total' daemon/crates/daemon-core/src/metrics.rs returns 8. cargo check passes.</reality_test>
  <done_when>8 new metric fields on DaemonMetrics, all registered in new(). Existing 12 metrics untouched. cargo check passes.</done_when>
</task>

<task id="T-102" req="REQ-T1,REQ-T6,REQ-T7" wave="0" depends="">
  <description>Scaffold new token-economics crate with Cargo.toml, SQLite schema, and public API types</description>
  <files>daemon/crates/token-economics/Cargo.toml, daemon/crates/token-economics/src/lib.rs, daemon/crates/token-economics/src/storage.rs, daemon/Cargo.toml, daemon/Cargo.lock</files>
  <scope_out>Do not implement scanning, parsing, or attribution yet (Waves 3-4). Just the crate skeleton, the SQLite schema (token_records, scan_state, price_table), the public TokenRecord struct, and the storage API (open, migrate, insert, query). Storage module should compile and have unit tests for schema creation + basic insert/query round trip.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml, cargo test -p token-economics --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>cargo test -p token-economics --manifest-path daemon/Cargo.toml exits 0. SQLite schema test creates token_records + scan_state + price_table tables. Insert a test TokenRecord, query it back, assert fields match. A stub returning empty Vec cannot pass the round-trip assertion.</reality_test>
  <done_when>token-economics crate compiles as workspace member. SQLite storage with WAL mode. TokenRecord struct defined. Schema migration on open(). Round-trip unit test passes.</done_when>
</task>

---

## Wave 1: Observability — Tracing Spans + New Metrics

<task id="T-103" req="REQ-O1" wave="1" depends="T-101">
  <description>Add #[instrument(skip_all)] to all public functions in daemon/crates/triumvirate/src/abe/*.rs (10 files, ~45 functions)</description>
  <files>daemon/crates/triumvirate/src/abe/worktree_setup.rs, daemon/crates/triumvirate/src/abe/codex_spawn.rs, daemon/crates/triumvirate/src/abe/task_tracker.rs, daemon/crates/triumvirate/src/abe/orchestrator.rs, daemon/crates/triumvirate/src/abe/failure_handler.rs, daemon/crates/triumvirate/src/abe/build_artifacts.rs, daemon/crates/triumvirate/src/abe/resume.rs, daemon/crates/triumvirate/src/abe/wave_gate.rs, daemon/crates/triumvirate/src/abe/post_exit_validator.rs, daemon/crates/triumvirate/src/abe/mod.rs, daemon/Cargo.lock</files>
  <scope_out>Do not change any function behavior. Only add #[instrument] attributes with structured fields (task_id, wave, status where available). Do not add metrics recording yet (that's T-104). Do not touch mcp-tools, daemon-http, or daemon-core.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -rc '#\[instrument' daemon/crates/triumvirate/src/abe/ returns a count >= 40. cargo check passes. No function signatures changed (git diff shows only added #[instrument] lines, not modified fn signatures).</reality_test>
  <done_when>Every public fn in the 10 ABE files has #[instrument(skip_all)] with appropriate structured fields. cargo check passes. Zero behavioral change.</done_when>
</task>

<task id="T-104" req="REQ-O4,REQ-O5,REQ-O7,REQ-O8,REQ-O9,REQ-O10,REQ-O11" wave="1" depends="T-101">
  <description>Wire the 8 new ABE metrics into the ABE code paths where they should be recorded</description>
  <files>daemon/crates/triumvirate/src/abe/task_tracker.rs, daemon/crates/triumvirate/src/abe/orchestrator.rs, daemon/crates/triumvirate/src/abe/codex_spawn.rs, daemon/crates/triumvirate/src/abe/worktree_setup.rs, daemon/crates/triumvirate/src/abe/failure_handler.rs, daemon/crates/triumvirate/src/abe/post_exit_validator.rs, daemon/crates/triumvirate/src/abe/wave_gate.rs, daemon/crates/mcp-tools/src/abe.rs, daemon/Cargo.lock</files>
  <scope_out>Do not define new metrics (T-101 did that). Do not change ABE dispatch logic. Only add .inc() / .observe() calls at the appropriate code points.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -rc 'abe_task_dispatch_total\|abe_task_duration\|abe_timeout_total\|abe_worktree_setup\|abe_failure_class\|abe_retry_total\|abe_validation_total' daemon/crates/triumvirate/src/abe/ daemon/crates/mcp-tools/src/abe.rs returns count >= 8 (at least one recording site per metric). cargo check passes.</reality_test>
  <done_when>All 8 new metrics are recorded at their appropriate code paths. ABE dispatches will now produce non-zero Prometheus values. cargo check passes.</done_when>
</task>

<task id="T-105" req="REQ-O2" wave="1" depends="">
  <description>Add #[instrument(skip_all)] to all public functions in daemon/crates/ledger/src/*.rs (10 files)</description>
  <files>daemon/crates/ledger/src/compression.rs, daemon/crates/ledger/src/gc.rs, daemon/crates/ledger/src/health.rs, daemon/crates/ledger/src/ingest.rs, daemon/crates/ledger/src/init.rs, daemon/crates/ledger/src/lessons.rs, daemon/crates/ledger/src/lib.rs, daemon/crates/ledger/src/pool.rs, daemon/crates/ledger/src/query.rs, daemon/crates/ledger/src/spool.rs, daemon/Cargo.lock</files>
  <scope_out>Do not change any function behavior. Only add #[instrument] attributes.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -rc '#\[instrument' daemon/crates/ledger/src/ returns count >= 20. cargo check passes.</reality_test>
  <done_when>Every public fn in ledger crate has #[instrument(skip_all)] with structured fields. cargo check passes.</done_when>
</task>

<task id="T-106" req="REQ-O12,REQ-O13,REQ-O14,REQ-O15" wave="1" depends="">
  <description>Wire 4 dead Prometheus metrics (agent_tokens_total, fleet_active_total, reviews_total, ledger_spool_size_bytes) to their correct recording sites</description>
  <files>daemon/crates/triumvirate/src/agent_exec.rs, daemon/crates/mcp-tools/src/fleet.rs, daemon/crates/mcp-tools/src/review.rs, daemon/crates/triumvirate/src/main.rs, daemon/Cargo.lock</files>
  <scope_out>Do not add new metrics. Only wire existing declared-but-never-updated ones to the correct code paths.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -n 'agent_tokens_total.*inc\|fleet_active_total.*set\|reviews_total.*inc\|ledger_spool_size_bytes.*set' in the listed files returns >= 4 matches. cargo check passes.</reality_test>
  <done_when>4 dead metrics now have recording call sites. After a real ABE dispatch + fleet spawn + review + ledger drain, all 4 will show non-zero values on /metrics.</done_when>
</task>

<task id="T-107" req="REQ-O3" wave="1" depends="">
  <description>Add parent trace span to execute_ask_agent linking MCP request → daemon dispatch → agent execution</description>
  <files>daemon/crates/triumvirate/src/agent_exec.rs, daemon/Cargo.lock</files>
  <scope_out>Do not change agent execution behavior. Only add tracing span with correlation fields.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>#[instrument] on execute_ask_agent with fields: agent, session_id, request_type. cargo check passes.</reality_test>
  <done_when>execute_ask_agent has a parent trace span visible in structured logs. cargo check passes.</done_when>
</task>

---

## Wave 2: Observability — Logging + Context + WebSocket

<task id="T-108" req="REQ-O16,REQ-O17,REQ-O18" wave="2" depends="T-103,T-104">
  <description>Add structured tracing::info/warn/error events for ABE lifecycle (dispatch start, task complete, wave start, wave gate pass, timeout, retry, collateral fix, worktree failure, escalation)</description>
  <files>daemon/crates/triumvirate/src/abe/orchestrator.rs, daemon/crates/triumvirate/src/abe/task_tracker.rs, daemon/crates/triumvirate/src/abe/codex_spawn.rs, daemon/crates/triumvirate/src/abe/failure_handler.rs, daemon/crates/triumvirate/src/abe/wave_gate.rs, daemon/crates/triumvirate/src/abe/worktree_setup.rs, daemon/Cargo.lock</files>
  <scope_out>Do not change behavior. Only add tracing events at lifecycle points.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -rc 'tracing::info!\|tracing::warn!\|tracing::error!' daemon/crates/triumvirate/src/abe/ returns a count significantly higher than pre-task baseline (take a before-count first).</reality_test>
  <done_when>All REQ-O16/O17/O18 lifecycle events emitted at the specified code points. Daemon log shows structured JSON for every ABE step.</done_when>
</task>

<task id="T-109" req="REQ-O19" wave="2" depends="">
  <description>Add anyhow::Context to critical I/O paths in ABE modules (file read/write, subprocess spawn, JSON parse, git operations)</description>
  <files>daemon/crates/triumvirate/src/abe/worktree_setup.rs, daemon/crates/triumvirate/src/abe/codex_spawn.rs, daemon/crates/triumvirate/src/abe/build_artifacts.rs, daemon/crates/triumvirate/src/abe/orchestrator.rs, daemon/Cargo.lock</files>
  <scope_out>Focus on bare .unwrap() and uncontexted ? in I/O paths. Not every single ? — measure by grepping for bare .unwrap() after the task.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -c '.with_context\|.context(' in the listed files returns >= 10. grep -c '.unwrap()' returns a count lower than pre-task baseline.</reality_test>
  <done_when>Critical I/O paths have context annotations. Errors include file path, operation name, and task_id. cargo check passes.</done_when>
</task>

<task id="T-110" req="REQ-O20,REQ-O21,REQ-O22,REQ-O23" wave="2" depends="T-104">
  <description>Wire WebSocket events (abe_task_state, abe_wave_state, fleet_progress) + REQ-O23 worktreeConfig hotfix</description>
  <files>daemon/crates/triumvirate/src/abe/task_tracker.rs, daemon/crates/triumvirate/src/abe/wave_gate.rs, daemon/crates/mcp-tools/src/fleet.rs, daemon/crates/triumvirate/src/abe/worktree_setup.rs, daemon/Cargo.lock</files>
  <scope_out>Do not change the WebSocket transport itself. Only add publish_event() calls at the right lifecycle points. The worktreeConfig fix is a one-liner.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -c 'abe_task_state\|abe_wave_state\|fleet_progress' in the listed files returns >= 3. grep -q 'extensions.worktreeConfig' daemon/crates/triumvirate/src/abe/worktree_setup.rs returns 0. cargo check passes.</reality_test>
  <done_when>3 new WS events emitted at lifecycle points. worktreeConfig auto-enabled. cargo check passes.</done_when>
</task>

---

## Wave 3: Token Economics — Scanner Core

<task id="T-111" req="REQ-T5" wave="3" depends="">
  <description>Extend GeminiStreamParser and TokenUsage struct with thinking_tokens, latency_ms, tool_calls fields</description>
  <files>daemon/crates/agent-adapter/src/gemini.rs, daemon/crates/shared-types/src/lib.rs, daemon/Cargo.lock</files>
  <scope_out>Do not change the stream parsing logic itself. Only extend the struct and the final-result extraction to include the new fields.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml, cargo test -p agent-adapter --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>grep -q 'thinking_tokens' daemon/crates/shared-types/src/lib.rs. grep -q 'latency_ms' daemon/crates/shared-types/src/lib.rs. cargo test -p agent-adapter passes.</reality_test>
  <done_when>TokenUsage has thinking_tokens, latency_ms, tool_calls as Option fields. GeminiStreamParser extracts them from stream-json result events. Existing parsing behavior unchanged for other fields.</done_when>
</task>

<task id="T-112" req="REQ-T1,REQ-T2,REQ-T3,REQ-T4" wave="3" depends="T-102">
  <description>Implement token scanner wrapping tokscale-core for Claude/Codex/Gemini session file parsing</description>
  <files>daemon/crates/token-economics/src/scanner.rs, daemon/crates/token-economics/src/lib.rs, daemon/crates/token-economics/Cargo.toml, daemon/Cargo.lock</files>
  <scope_out>Implement scanning only — not attribution or daemon integration. The scanner reads files, parses tokens, returns TokenRecords. Does not write to SQLite (that's storage.rs from T-102). Does not attribute to builds/tasks (that's T-113).</scope_out>
  <tools>cargo check -p token-economics --manifest-path daemon/Cargo.toml, cargo test -p token-economics --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check -p token-economics --manifest-path daemon/Cargo.toml</verify>
  <reality_test>Unit test: create a mock JSONL file with known token counts, run scanner, assert returned TokenRecords match. Tests for Claude, Codex, and Gemini formats.</reality_test>
  <done_when>Scanner reads Claude JSONL, Codex JSONL, and Gemini JSON/telemetry formats. Returns Vec<TokenRecord> per file. Incremental by mtime. Tests pass.</done_when>
</task>

<task id="T-113" req="REQ-T8,REQ-T9" wave="3" depends="T-102">
  <description>Implement cost attribution engine — session-ID correlation, outbox matching, price calculation</description>
  <files>daemon/crates/token-economics/src/attribution.rs, daemon/crates/token-economics/src/lib.rs, daemon/Cargo.lock</files>
  <scope_out>Attribution logic only. Does not scan files (T-112 does that). Does not serve HTTP (Wave 4). Takes TokenRecords + outbox events as input, returns attributed records with build_id/task_id/cost_usd filled in.</scope_out>
  <tools>cargo check -p token-economics --manifest-path daemon/Cargo.toml, cargo test -p token-economics --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check -p token-economics --manifest-path daemon/Cargo.toml</verify>
  <reality_test>Unit test: create TokenRecords with known session_ids + mock outbox entries with matching session_ids + build_ids. Run attribution. Assert cost_usd is calculated and build_id/task_id are populated. Unmatched sessions go to "unattributed" bucket.</reality_test>
  <done_when>Attribution engine matches session IDs, calculates costs from price table, assigns build/task/wave. Tests pass.</done_when>
</task>

<task id="T-114" req="REQ-T1" wave="3" depends="T-102">
  <description>Implement direct-write API for daemon-mediated sessions (Lane 1: agent_exec writes TokenRecord directly)</description>
  <files>daemon/crates/token-economics/src/direct.rs, daemon/crates/token-economics/src/lib.rs, daemon/crates/triumvirate/src/agent_exec.rs, daemon/Cargo.lock</files>
  <scope_out>Only wire the direct-write path from agent_exec. Do not modify the scanner (T-112) or attribution (T-113). The direct write bypasses the scanner entirely — it has exact session context at call time.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>After an ask_agent call, a TokenRecord with the correct agent, session_id, and token counts exists in the SQLite database. Unit test mocks agent_exec response and verifies the direct write.</reality_test>
  <done_when>agent_exec calls record_daemon_tokens() after every agent response. TokenRecord written to SQLite with exact attribution. cargo check passes.</done_when>
</task>

---

## Wave 4: Token Economics — HTTP + MCP + Lifecycle

<task id="T-115" req="REQ-T10,REQ-T11,REQ-T12" wave="4" depends="T-112,T-113,T-114">
  <description>Add 3 new Axum HTTP routes for token data (GET /api/tokens/summary, /by-build, /by-session)</description>
  <files>daemon/crates/daemon-http/src/lib.rs, daemon/crates/daemon-http/Cargo.toml, daemon/crates/token-economics/src/queries.rs, daemon/crates/token-economics/src/lib.rs, daemon/Cargo.lock</files>
  <scope_out>Do not modify existing HTTP routes. Only add 3 new ones + the query functions they call.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>After inserting test data into token-economics.db, curl /api/tokens/summary returns JSON with agent breakdown. curl /api/tokens/by-build?build_id=test returns per-task cost. curl /api/tokens/by-session?session_id=test returns token breakdown.</reality_test>
  <done_when>3 new HTTP routes serve token data from SQLite. cargo check passes.</done_when>
</task>

<task id="T-116" req="REQ-T14,REQ-T15" wave="4" depends="T-115">
  <description>Add 2 new MCP tools (get_token_summary, get_build_cost) to mcp-tools</description>
  <files>daemon/crates/mcp-tools/src/token_tools.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/mcp-tools/Cargo.toml, daemon/crates/triumvirate/src/main.rs, daemon/Cargo.lock</files>
  <scope_out>Only add 2 new MCP tools. Do not modify existing tools. Each tool calls the same query functions as the HTTP routes (shared code in token-economics/queries.rs).</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>Calling get_token_summary via MCP returns JSON matching the HTTP /api/tokens/summary response shape. Calling get_build_cost with a known build_id returns per-task cost data.</reality_test>
  <done_when>2 new MCP tools registered on McpBridge and callable. cargo check passes.</done_when>
</task>

<task id="T-117" req="REQ-T13,REQ-T16,REQ-T17" wave="4" depends="T-112,T-114">
  <description>Wire scanner lifecycle into daemon: background tokio task, notify-based file watching, startup reconciliation, token_update WebSocket event</description>
  <files>daemon/crates/token-economics/src/lifecycle.rs, daemon/crates/token-economics/src/lib.rs, daemon/crates/triumvirate/src/main.rs, daemon/crates/daemon-core/src/observability.rs, daemon/Cargo.lock</files>
  <scope_out>Do not modify the scanner parsing logic (T-112). Only wire it into the daemon's runtime: spawn the background task, set up file watchers, run startup reconciliation, emit WS events after each scan cycle. Add token_db field to ObservabilityBus.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml</tools>
  <verify>cargo check --workspace --manifest-path daemon/Cargo.toml</verify>
  <reality_test>After daemon restart, scanner runs a full reconciliation. New agent sessions trigger file-watcher-based incremental scans. WS clients receive token_update events after each scan cycle.</reality_test>
  <done_when>Scanner runs as background tokio task. File watching active for Claude + Codex paths. Gemini telemetry.jsonl tracked by byte offset. token_update WS events emitted. ObservabilityBus has token_db field. Startup reconciliation catches downtime sessions. cargo check passes.</done_when>
</task>

---

## Wave 5: Integration + Public Release

<task id="T-118" req="REQ-O1-O23,REQ-T1-T17" wave="5" depends="T-108,T-109,T-110,T-115,T-116,T-117">
  <description>Integration verification — full cargo check + package-scoped tests + manual smoke test of metrics + token dashboard</description>
  <files></files>
  <scope_out>Verification only. No code changes unless a bug is found during verification.</scope_out>
  <tools>cargo check --workspace --manifest-path daemon/Cargo.toml, cargo test -p daemon-core -p daemon-http -p token-economics -p mcp-tools -p ledger --manifest-path daemon/Cargo.toml, curl</tools>
  <verify>All package-scoped tests pass. curl /metrics shows non-zero ABE metrics. curl /api/tokens/summary returns data. MCP get_token_summary returns data.</verify>
  <reality_test>One full ABE dispatch + review + ask_agent call, then verify: (1) /metrics has non-zero abe_task_dispatch_total; (2) daemon log has structured JSON lifecycle events; (3) WS stream received abe_task_state event; (4) /api/tokens/summary shows token counts; (5) get_token_summary MCP tool returns data.</reality_test>
  <done_when>All 40 REQs verified. Integration smoke test passes.</done_when>
</task>

<task id="T-119" req="RELEASE" wave="5" depends="T-118" lane="orchestrator">
  <description>Repo hygiene, CHANGELOG 3.2.0 entry, RELEASE_NOTES</description>
  <files>CHANGELOG.md, ROADMAP.md, docs/3.2.0/RELEASE_NOTES.md</files>
  <done_when>CHANGELOG has 3.2.0 section. ROADMAP shows 3.2.0 shipped. Release notes written.</done_when>
</task>

<task id="T-120" req="RELEASE" wave="5" depends="T-119" lane="abe">
  <description>Build release binary (darwin-arm64 native, cross-platform if tooling available)</description>
  <files>daemon/target/release-dist/</files>
  <done_when>At least darwin-arm64 binary packaged with SHA256.</done_when>
</task>

<task id="T-121" req="RELEASE" wave="5" depends="T-119,T-120" lane="orchestrator">
  <description>GitHub release 3.2.0 — tag, push, publish</description>
  <files></files>
  <done_when>gh release view 3.2.0 shows binaries and release notes.</done_when>
</task>

<task id="T-122" req="RELEASE" wave="5" depends="T-121">
  <description>Clean-room install verification</description>
  <files>docs/3.2.0/INSTALL_VERIFIED.md</files>
  <done_when>Fresh-environment install works. INSTALL_VERIFIED.md written.</done_when>
</task>

<task id="T-123" req="RELEASE" wave="5" depends="T-121" lane="orchestrator">
  <description>Close resolved GitHub issues for 3.2.0</description>
  <files></files>
  <done_when>Issues #19, #20, #21 closed if resolved. #23 closed if token economics shipped. Remaining issues relabeled.</done_when>
</task>

---

## Execution Contract

```yaml
backlog_freeze: true
total_tasks: 24
active_tasks: 24
completed_tasks: []
dispatch_method: dispatch_codex_worktree
audit_method: Phase 5.3 twin audit (2-round cap per Rule A)
completion_detection: sentinel file polling (.triumvirate/TASK_COMPLETE.json)
closing_ceremony: commit + sentinel + HTTP POST (three-ceremony block)
manifest_path: --manifest-path daemon/Cargo.toml (MANDATORY on every cargo command)
test_scope: cargo test -p <crate> (NOT cargo test --workspace — hangs on integration tests)
```
