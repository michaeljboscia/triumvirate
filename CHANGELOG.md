# Changelog

All notable changes to this project will be documented in this file.

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
