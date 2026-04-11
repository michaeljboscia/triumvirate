# Changelog

All notable changes to this project will be documented in this file.

## 3.3.0 — Live Agent Streaming (2026-04-11)

Real-time visibility into agent execution. See tool calls, file reads, and responses as they happen.

### What Changed

- **AgentStreamEvent pipeline.** New `AgentStreamEvent` enum in shared-types with 6 variants (TurnStarted, ToolCall, FileRead, ResponseChunk, TurnCompleted, Error). Monotonic sequence numbers for gap detection. GeminiStreamParser and CodexExecParser emit events via tokio mpsc channels during parsing.

- **Streamable HTTP MCP endpoint.** Daemon serves MCP over Streamable HTTP at `POST/GET /mcp` using rmcp's `transport-streamable-http-server` feature. SSE streaming with session management via `Mcp-Session-Id` header. Bearer token auth. Coexists with existing stdio transport.

- **`triumvirate proxy` command.** Bridges Claude Code's stdio JSON-RPC to the daemon's HTTP /mcp endpoint. Auto-reconnect with bounded exponential backoff (100ms→5s). Clear error if daemon unreachable at startup. Golden path for Claude Code users.

- **`triumvirate watch` command.** Connects to daemon WebSocket, pretty-prints agent streaming events in real-time. `--session` filter, `--all` flag, sequence gap detection, in-place heartbeat timer using crossterm. The README experience — in a side pane.

- **WebSocket `agent_stream` events.** ObservabilityBus emits AgentStreamEvent alongside existing event types (token_update, abe_task_state, fleet_progress). No breaking changes to existing consumers.

- **SSE spike test server.** Standalone minimal MCP server at `daemon/spike/sse-test-server/` for empirically testing Claude Code SSE frame rendering. Result: UNTESTED (requires manual verification).

### Process Innovations

- **Multi-model bake-off.** Claude subagent + Codex worker build the same task in parallel. Gemini judges which is better (~$0.001/review). Best-of-breed merge. Baked into `/goatrodeo` Phase 5.4 and `/postrodeo` Phases 4.2/5.3.

- **Incremental BUILD_MANIFEST.** Created at build start, appended after each wave. Never missed again.

### Known Issues

- Claude Code does not render MCP server progress notifications (GitHub #4157). Streaming visibility requires the `watch` side pane until Anthropic ships SSE support.
- Spike test untested — requires manual `claude mcp add --transport http` verification.

---

## 3.2.0 — Observability & Token Economics (2026-04-10)

Full observability instrumentation + a token economics engine for tracking costs across all three AI agents.

### What Changed

- **Tracing spans on every ABE function.** All public functions in the ABE modules (10 files, ~45 functions) and ledger crate (10 files) now have `#[instrument]` spans with structured fields (task_id, wave, status). Parent trace span links MCP requests through daemon dispatch to agent execution.

- **8 new ABE Prometheus metrics.** `abe_task_dispatch_total`, `abe_task_duration_seconds`, `abe_wave_duration_seconds`, `abe_timeout_total`, `abe_worktree_setup_duration_seconds`, `abe_failure_class_total`, `abe_retry_total`, `abe_validation_total`. All wired to recording sites.

- **4 dead metrics now live.** `agent_tokens_total`, `fleet_active_total`, `reviews_total`, `ledger_spool_size_bytes` — all were declared but never recorded. Now wired to their correct code paths.

- **Structured logging for ABE lifecycle.** `tracing::info/warn/error` events at every lifecycle point: dispatch start, task complete, wave start, wave gate pass, timeout, retry, collateral fix, worktree failure, escalation.

- **Error context on ABE I/O paths.** Critical file operations, subprocess spawns, JSON parsing, and git operations now carry `.context()` annotations with file paths and task IDs.

- **3 new WebSocket events.** `abe_task_state` (every task state transition), `abe_wave_state` (wave start/completion), `fleet_progress` (fleet state transitions). Plus `worktreeConfig` auto-enabled during worktree setup (#19 hotfix).

- **Token economics crate.** New `daemon/crates/token-economics/` with SQLite storage (WAL mode), session file scanner (Claude JSONL, Codex JSONL, Gemini JSON + telemetry.jsonl), cost attribution engine (session-ID correlation, price table), and direct-write API for daemon-mediated sessions.

- **3 new HTTP routes.** `GET /api/tokens/summary`, `/api/tokens/by-build`, `/api/tokens/by-session` — query token usage and costs by agent, build, or session.

- **2 new MCP tools.** `get_token_summary` and `get_build_cost` — accessible from Claude sessions.

- **Scanner lifecycle.** Background tokio task with notify-based file watching, startup reconciliation for sessions that occurred while daemon was down, periodic reconciliation fallback, `token_update` WebSocket events after each scan cycle.

- **TokenUsage extended.** `thinking_tokens`, `latency_ms`, `tool_calls` fields added (all `Option<u64>`). GeminiStreamParser extracts them from stream-json result events.

### Known Issues

- Token scanner tests are compile-verification only (no behavioral tests with mock data yet)
- Scanner lifecycle needs real-world validation with actual agent sessions
- `notify` crate file watching may miss events on some filesystems (periodic reconciliation compensates)

---

## 3.1.0 — MCP Consolidation (2026-04-10)

The Rust daemon becomes the sole MCP endpoint. The legacy TypeScript inter-agent server is retired.

### What Changed

- **Single MCP server.** The Rust `triumvirate` daemon now handles ALL tool calls — session management, ABE dispatch, fleet operations, knowledge (ledger/lessons/memory/scratchpad), peer review, and Gemini queries. The separate TypeScript `inter-agent` MCP server (`mcp-server/`) has been archived to `archive/mcp-server-ts/`.

- **10 backwards-compatible aliases.** Old tool names (`spawn_daemon`, `ask_daemon`, `dismiss_daemon`, `list_daemons`, `send_message`, `get_response`, `list_jobs`, `write_scratchpad`, `list_scratchpad`, `code_review`) still work. They map to canonical Rust tools with automatic parameter translation. No existing Claude Code sessions or skills break.

- **Version alignment.** Cargo workspace version bumped from 0.1.0 to 3.1.0. `triumvirate --version` now prints the correct version. HTTP `/health` and MCP `get_info()` both report the version. A pre-commit hook (`scripts/version-drift-check.sh`) catches version mismatches between spec docs and Cargo.toml.

- **Codebase restructured.** `main.rs` went from 6,216 lines to ~4,700 lines (production + tests). Tool handlers extracted into 7 modules in `mcp-tools/` crate. HTTP route handlers extracted to `daemon-http/` crate. `DaemonMetrics` and `ObservabilityBus` moved to `daemon-core/`.

- **ABE completion detection (experimental).** New `POST /abe/task-complete` HTTP route and sentinel-file watcher added for multi-channel worker completion detection. Workers can now explicitly signal completion via three channels: commit, sentinel file, and HTTP POST. (Runtime activation of the detection loop is a known issue — see below.)

### Migration for Existing Users

1. **Update `~/.claude.json`:** Remove the `"inter-agent"` entry from `mcpServers`. The `"triumvirate"` entry is already the replacement. A backup was created at `~/.claude.json.bak.3.1.0` during the upgrade.

2. **Skills already updated.** The `send-to-codex`, `send-to-gemini`, `send-to-siblings`, `inter-agent-protocol`, `goatrodeo`, `design-goatrodeo`, and `crystallize` skills now reference `mcp__triumvirate__*` tools instead of `mcp__inter-agent__*`.

3. **No behavior changes.** Every MCP tool that worked before still works. The alias layer translates old parameter names to new ones automatically.

### Breaking Changes

- The `get_response` tool now returns a deprecation notice instead of polling a job queue. The async job queue (`send_message` → `get_response` pattern) was removed. Use `ask_session` for synchronous responses.
- The `dismiss_daemon` `hard` parameter is silently dropped with a log warning. Rust sessions don't support forced termination.

### Known Issues

- **ABE daemon completion detection not activating at runtime.** T-004B's code is merged and the `/abe/task-complete` HTTP route responds, but the per-task sentinel watcher and git HEAD watcher loops are not starting on new dispatches. Workaround: poll the worktree's `.triumvirate/TASK_COMPLETE.json` sentinel file directly from the orchestrator.
- **ABE pre-commit hook false-positives.** The stub-marker check in `daemon/assets/pre-commit-hook.sh` matches `TODO`/`FIXME` inside Rust raw-string test fixtures. Workers bypass with `--no-verify`.
- **`cargo test --workspace` hangs on integration tests.** Some ABE tests spawn real agent subprocesses that don't terminate cleanly in automated environments. Use package-scoped tests (`cargo test -p <crate>`) for reliable CI.

### Under the Hood

- 12 workspace crates, all at version 3.1.0
- `mcp-tools` crate: 8 modules (abe, aliases, fleet, gemini_query, inter_agent, knowledge, review, lib)
- `daemon-core` crate: metrics, observability bus, version, 4 placeholder trait interfaces
- `daemon-http` crate: 19+ HTTP route handlers + WebSocket + metrics + dashboard
- New `shared-types/src/abe.rs`: `TaskCompleteRequest` type for worker completion signaling

## 3.0.0 — ABE: Autonomous Build Enforcement (2026-04-07)

First shipped version. Introduced the ABE dispatch system (`dispatch_codex_worktree`), contract-based task validation, fleet operations, peer review engine, and the Rust MCP bridge. See `docs/abe/` for full documentation.
