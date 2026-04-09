# Triumvirate v3.1 — MCP Consolidation: Kill the TS Server

**Version:** v3.1 Spec
**Date:** 2026-04-08
**GitHub Issues:** #13 (Rust rewrite — reframed as MCP consolidation)
**Goat Rodeo:** Pending
**Codebase State:** ABE v3.0 shipped, stress test Phases 1-4 passed, daemon at commit `1ca4f58`
**Predecessor:** v3.0 (ABE). **Successor:** v3.2 (Observability + Token Economics)

---

## Problem Statement

The Triumvirate project runs TWO processes to serve MCP tools: a TypeScript inter-agent server (`mcp-server/`, 24 source files, Node.js) and a Rust daemon (`daemon/`, 12 crates, single binary). Claude's `~/.claude.json` points at the TS server. The Rust daemon has 35+ MCP tools — more than the TS server's 12 — but isn't the front door.

This creates:
- **Two processes to manage** — TS server crashes independently of the daemon
- **Two tool namespaces** — TS tools are `spawn_daemon`, Rust tools are `spawn_session` for the same operation
- **A proxy tax on every new feature** — any MCP tool added to the Rust daemon needs a TS shim
- **A 5,000-line main.rs** — all tool handlers, HTTP routes, metrics, and state management crammed into one file

The Rust daemon already won. This sprint makes it official.

---

## Constitution (Unchanged)

1. Claude is the front door. User talks to Claude.
2. Lifecycle is always visible. No silent failures.
3. Plain language in, structured results out. No command ceremony.
4. Failure is loud, immediate, and actionable.

---

## What This Covers

1. Extract the monolithic `main.rs` into a proper crate architecture with functional boundaries
2. Ensure every TS MCP tool has a Rust equivalent (or deliberate exclusion)
3. Update `~/.claude.json` to point at the Rust daemon binary
4. Delete the TS MCP server process from the runtime
5. Add the `ObservabilityBus` pattern to McpBridge for the v3.2 sprint
6. Add backwards-compatible tool aliases so existing skills/prompts don't break

## What This Does NOT Cover

- Oracle/Pythia tools (15 tools in `oracle-tools.ts`) — these stay as a separate MCP server or move to a separate Rust MCP server in a later sprint. Pythia already has its own MCP (`mcp__pythia-gtm__`).
- Dashboard (Svelte/TS) — stays as-is. It's a UI, not MCP infrastructure.
- Observability instrumentation — that's v3.2.
- Token economics — that's v3.2.
- New features — this is a refactor sprint. Same tools, better architecture.

---

## Current State Audit

### TS MCP Server Tools (unified-tools.ts — 12 tools)

| TS Tool | Schema | Semantics |
|---------|--------|-----------|
| `spawn_daemon` | `{target, cwd?, session_name?, timeout_ms?}` | Spawn persistent Gemini or Codex session |
| `send_message` | `{target, request_type, question, context?, cwd?, session_log?, timeout_ms?}` | Fire-and-forget message, returns job_id |
| `get_response` | `{job_id, timeout_ms?}` | Poll for job result by ID |
| `ask_daemon` | `{daemon_id, message, timeout_ms?}` | Send message to existing daemon, wait for response |
| `dismiss_daemon` | `{daemon_id}` | Kill a daemon session |
| `list_daemons` | `{target?}` | List active sessions |
| `list_jobs` | `{target?}` | List pending/completed jobs |
| `write_scratchpad` | `{key, value, namespace?}` | Write to shared scratchpad |
| `list_scratchpad` | `{namespace?}` | List scratchpad entries |
| `pythia_query` | `{query, project?}` | Query Pythia (delegated) |
| `pythia_corpus_health` | `{project?}` | Check Pythia index health (delegated) |
| `code_review` | `{diff, context?, target?}` | Request code review from twin |

### Rust Daemon MCP Tools (main.rs McpBridge — 35+ tools)

| Rust Tool | Line | Category |
|-----------|------|----------|
| `ping` | 204 | Health |
| `ask_agent` | 209 | Inter-agent |
| `spawn_session` | 258 | Inter-agent |
| `ask_session` | 298 | Inter-agent |
| `dismiss_session` | 342 | Inter-agent |
| `list_sessions` | 370 | Inter-agent |
| `get_status` | 390 | Status |
| `memory_write` | 434 | Knowledge |
| `memory_read` | 460 | Knowledge |
| `scratchpad_write` | 484 | Knowledge |
| `scratchpad_list` | 502 | Knowledge |
| `outbox_recent` | 521 | Knowledge |
| `fallback_list` | 538 | Knowledge |
| `fallback_ack` | 557 | Knowledge |
| `fallback_gc` | 568 | Knowledge |
| `ledger_health` | 584 | Knowledge |
| `ledger_query` | 596 | Knowledge |
| `ledger_session` | 617 | Knowledge |
| `ledger_record` | 638 | Knowledge |
| `ledger_gc` | 655 | Knowledge |
| `lesson_add` | 671 | Knowledge |
| `lesson_query` | 689 | Knowledge |
| `lesson_validate` | 710 | Knowledge |
| `lesson_list` | 730 | Knowledge |
| `dispatch_codex` | 752 | ABE |
| `dispatch_codex_worktree` | 859 | ABE |
| `query_gemini` | 1051 | Gemini |
| `query_gemini_review` | 1078 | Gemini |
| `get_task_status` | 1125 | ABE |
| `get_task_output` | 1137 | ABE |
| `cancel_task` | 1149 | ABE |
| `fleet_spawn` | 1161 | Fleet |
| `fleet_status` | 1223 | Fleet |
| `fleet_cancel` | 1279 | Fleet |
| `review_request` | 1289 | Review |
| `review_submit` | 1316 | Review |
| `review_status` | 1334 | Review |

### Tool Gap Analysis

| TS Tool | Rust Equivalent | Gap |
|---------|----------------|-----|
| `spawn_daemon` | `spawn_session` | Name alias only |
| `send_message` | — | **GAP: async job model** |
| `get_response` | — | **GAP: async job model** |
| `ask_daemon` | `ask_session` | Name alias only |
| `dismiss_daemon` | `dismiss_session` | Name alias only |
| `list_daemons` | `list_sessions` | Name alias only |
| `list_jobs` | `get_status` (partial) | Shape difference |
| `write_scratchpad` | `scratchpad_write` | Name alias only |
| `list_scratchpad` | `scratchpad_list` | Name alias only |
| `pythia_query` | — | **OUT OF SCOPE** — stays in Pythia MCP |
| `pythia_corpus_health` | — | **OUT OF SCOPE** — stays in Pythia MCP |
| `code_review` | `review_request` | Name + shape mapping |

**Real gaps: 0** — `send_message`/`get_response` async model eliminated (Decision R1-D1: C). All tools map to existing Rust equivalents via aliases. Skills updated to use `ask_session` directly.

---

## Requirements

### Crate Architecture Refactor

**REQ-C1:** Extract McpBridge and all tool handler methods from `main.rs` into `mcp-tools` crate, organized by functional boundary. Each module receives NARROWED interfaces (not full `&McpBridge`) — e.g., `inter_agent.rs` gets `SessionStore + AgentExecutor`, `abe.rs` gets `TaskTracker + ObservabilityBus`, `knowledge.rs` gets `LedgerStoreFactory + MemoryStore`. (Twin consensus: narrowed interfaces mandatory for testability)

```
daemon/crates/mcp-tools/src/
├── lib.rs              # McpBridge struct, tool_router registration, ObservabilityBus
├── inter_agent.rs      # spawn_session, ask_session, dismiss_session, list_sessions, ask_agent
├── abe.rs              # dispatch_codex, dispatch_codex_worktree, get_task_status/output, cancel_task
├── fleet.rs            # fleet_spawn, fleet_status, fleet_cancel
├── knowledge.rs        # ledger_*, lesson_*, memory_*, scratchpad_*, outbox_*, fallback_*
├── review.rs           # review_request, review_submit, review_status
├── gemini_query.rs     # query_gemini, query_gemini_review
└── aliases.rs          # Backwards-compatible tool aliases + parameter mapping
```

**REQ-C2:** Extract ALL `*_route` async functions from `main.rs` into `daemon-http` crate: `ask_agent_route`, `ledger_wake_route`, `ledger_health_route`, `ledger_query_route`, `ledger_session_route`, `ledger_record_route`, `ledger_gc_route`, `lesson_add_route`, `lesson_query_route`, `lesson_validate_route`, `lesson_list_route`, `memory_write_route`, `memory_read_route`, `scratchpad_write_route`, `scratchpad_list_route`, `outbox_recent_route`, `fallback_list_route`, `fallback_ack_route`, `fallback_gc_route`, and the `ws_route` WebSocket handler. HTTP routes call the same domain logic as MCP tools — both are thin presentation layers.

**REQ-C3:** Extract DaemonState construction, session management, and startup logic from `main.rs` into `daemon-core`. The `main.rs` in the `triumvirate` crate becomes startup wiring only — under 300 lines.

**REQ-C4:** After extraction, `main.rs` contains ONLY: CLI arg parsing, config loading, `DaemonState` construction, McpBridge construction, HTTP server spawn, MCP transport spawn, background task spawns (scanner, ledger drain), and graceful shutdown. No tool handlers. No route handlers. No business logic.

### Async Job Model — ELIMINATED (Decision R1-D1: C)

The TS `send_message` + `get_response` async pattern was a workaround for Node.js single-threaded blocking. Rust's `ask_session` is async internally (tokio) and Claude Code handles long-running MCP calls natively.

**REQ-J1:** ~~Async job queue~~ **DROPPED.** The `send_message` alias routes to `ask_session` (synchronous from Claude's perspective). No DashMap, no TTL reaper, no job IDs.

**REQ-J2:** Update `send-to-codex` skill to use `mcp__triumvirate__ask_session` instead of `mcp__inter-agent-codex__send_message`.

**REQ-J3:** Update `send-to-gemini` skill to use `mcp__triumvirate__ask_session` instead of `mcp__inter-agent-gemini__send_message`.

**REQ-J4:** Update `send-to-siblings` skill to use `mcp__triumvirate__ask_session` for both agents instead of per-agent `send_message` calls.

### Tool Aliases (Backwards Compatibility)

**REQ-A1:** Register backwards-compatible aliases so existing Claude skills, prompts, and CLAUDE.md instructions that reference TS tool names continue to work:

| Alias (old TS name) | Routes to (Rust handler) |
|---------------------|-------------------------|
| `spawn_daemon` | `spawn_session` |
| `ask_daemon` | `ask_session` |
| `dismiss_daemon` | `dismiss_session` |
| `list_daemons` | `list_sessions` |
| `code_review` | `review_request` (with schema mapping) |

**REQ-A2:** Aliases accept the TS schema (e.g., `{target: "gemini"}` instead of `{agent: "gemini"}`) and map parameters internally. The Rust canonical schema is the source of truth; aliases are a compatibility shim.

**REQ-A3:** Aliases emit a `tracing::info!("tool_alias", old_name, new_name)` event so we can track usage and deprecate them when safe.

### ObservabilityBus (Pre-wire for v3.2)

**REQ-B1:** Create an `ObservabilityBus` struct:

```rust
pub struct ObservabilityBus {
    pub metrics: Arc<DaemonMetrics>,
    pub ws_events: broadcast::Sender<String>,
}
```

Constructed in `main()`, injected into both `DaemonState` (for HTTP routes) and `McpBridge` (for MCP tools). This is the wiring that v3.2's observability sprint builds on.

**REQ-B2:** `DaemonMetrics` struct moves from `main.rs` into `mcp-tools` crate (or a shared location accessible by both HTTP and MCP layers). Currently defined at main.rs:1446.

**REQ-B3:** `publish_ws_event` function moves from `main.rs` into `ObservabilityBus` as a method. Currently defined at main.rs:1582.

### Front Door Swap

**REQ-F1:** The Rust daemon binary serves MCP over stdio transport (same as the TS server does today). The `mcp-bridge` crate already uses `rmcp` with stdio — verify this works as the primary MCP endpoint.

**REQ-F2:** Update `~/.claude.json` to replace the TS MCP server entry (`inter-agent`) with the Rust daemon binary. The tool namespace changes from `mcp__inter-agent__*` to `mcp__triumvirate__*` (or keep `inter-agent` as the server name in the Rust binary for zero-change migration).

**REQ-F3:** The Rust MCP server MUST declare all tool schemas via the `rmcp` tool_router macro. Every tool MUST have a description string of at least 20 characters explaining what it does and when to use it. Tool descriptions are user-facing (Claude reads them to decide which tool to call). Alias tools must include "Alias for [canonical_name]" in their description.

**REQ-F4:** Verify that the Rust MCP server handles the MCP lifecycle correctly: `initialize`, `tools/list`, `tools/call`, `notifications/progress`. The TS server uses `@modelcontextprotocol/sdk` — verify protocol parity.

### Cleanup

**REQ-X1:** After the front door swap is verified, the TS MCP server (`mcp-server/`) is archived to `archive/mcp-server-ts/`. Not deleted — preserved for reference during v3.2 if needed.

**REQ-X2:** Remove the TS MCP server's entry from `~/.claude.json` (the `inter-agent` MCP config).

**REQ-X3:** Remove Node.js runtime dependency for MCP. The daemon is the only process needed.

### Oracle/Pythia Decision

**REQ-P1:** ~~Oracle split~~ **DROPPED.** Oracle tools are NOT registered in the TS inter-agent server (`server.ts:5` confirms: "Oracle tools are NOT registered here — they live in Pythia"). No split needed. Pythia MCP (`mcp__pythia-gtm__`) is already a separate server and is unaffected by this migration.

**REQ-P2:** ~~Oracle entrypoint split~~ **DROPPED.** See REQ-P1.

---

## Architecture — Target State

### Crate Dependency Graph

```
triumvirate (binary — startup wiring only)
├── mcp-tools          # MCP tool handlers by module
│   ├── mcp-bridge     # rmcp protocol, stdio transport
│   ├── daemon-core    # DaemonState, config, sessions
│   ├── agent-adapter  # CLI subprocess adapters
│   ├── fleet          # Fleet orchestration
│   ├── ledger         # SQLite event ledger
│   ├── peer-review    # Review engine
│   ├── shared-types   # Cross-crate types
│   └── fallback-outbox
├── daemon-http        # Axum routes (thin layer)
│   ├── daemon-core
│   ├── mcp-bridge
│   └── shared-types
└── daemon-core
```

### Process Model — Before and After

**Before (v3.0):**
```
~/.claude.json → node mcp-server/dist/server.js  (TS, inter-agent tools)
~/.claude.json → triumvirate daemon              (Rust, ABE/fleet/ledger/etc.)
~/.claude.json → pythia MCP                       (separate)
```

**After (v3.1):**
```
~/.claude.json → triumvirate daemon              (Rust, ALL tools except oracle)
~/.claude.json → oracle MCP                       (TS, pythia/oracle tools only)
~/.claude.json → pythia MCP                       (separate, unchanged)
```

### File Changes

| File | Change |
|------|--------|
| `daemon/crates/triumvirate/src/main.rs` | Shrinks from ~5,000 lines to ~300 (startup only) |
| `daemon/crates/mcp-tools/src/*.rs` | NEW — 7 modules, ~2,500 lines extracted from main.rs |
| `daemon/crates/mcp-tools/src/jobs.rs` | NEW — async job queue (~200 lines) |
| `daemon/crates/mcp-tools/Cargo.toml` | Updated deps — all domain crates |
| `daemon/crates/daemon-http/src/lib.rs` | Expanded — HTTP routes extracted from main.rs |
| `daemon/crates/daemon-core/src/lib.rs` | Expanded — DaemonState, config, sessions |
| `~/.claude.json` | Updated — triumvirate replaces inter-agent |
| `mcp-server/` | Archived to `archive/mcp-server-ts/` |
| `oracle-mcp-server/` | NEW — oracle tools split from inter-agent |

---

## Acceptance Criteria

1. `cargo test --workspace` passes after all changes
2. `cargo build --release` produces a single binary that serves both MCP (stdio) and HTTP
3. Every TS MCP tool name works through the Rust daemon (via alias or native)
4. `send-to-codex`, `send-to-gemini`, and `send-to-siblings` skills work via `ask_session`
5. `~/.claude.json` points at the Rust binary only (plus oracle + pythia)
6. The TS `node` process for inter-agent MCP is gone
7. `main.rs` is under 300 lines
8. All existing Claude skills, CLAUDE.md instructions, and goat rodeo prompts work without modification
9. The daemon's `/metrics` endpoint still serves Prometheus metrics
10. WebSocket events still flow to the dashboard

---

## Risk Mitigation

### Rollback Plan
Keep the archived TS server. If the Rust front door breaks mid-migration:
1. Restore the `inter-agent` entry in `~/.claude.json`
2. `node mcp-server/dist/server.js` — TS server runs immediately
3. Debug the Rust issue without time pressure

### Testing Strategy
1. **Tool parity test:** For each of the 12 TS tools, call the equivalent Rust tool with the same parameters and verify the response shape matches
2. **Integration test:** Run a full goat rodeo through the Rust front door (this goat rodeo IS the test)
3. **Regression test:** Existing `cargo test` suite (156 tests) must pass
4. **Alias test:** Call every alias name and verify it routes correctly

---

## Build Strategy

This sprint uses ABE's `dispatch_codex_worktree` for the build — the first real dogfood run. The irony: we're using the TS MCP server to dispatch the build that kills the TS MCP server.

**Waves (6 total):**
- Preflight (Wave -1): Fix 3 pre-existing test compile errors, bump Cargo to 3.1.0, wire version reporting, install drift hook
- Wave 0: Contracts — ObservabilityBus type, module trait interfaces, alias schema types
- Wave 1: Extract — move tool handlers from main.rs to mcp-tools modules (no behavior change)
- Wave 2: Extract — move HTTP routes to daemon-http, DaemonState to daemon-core (no behavior change)
- Wave 3: Build — tool aliases, parameter mapping, skill updates (send-to-* skills)
- Wave 4: Swap — add inter-agent tools to triumvirate MCP, verify, update ~/.claude.json, archive TS server
- Wave 5: **Public Release** — repo hygiene, cross-platform binaries, CHANGELOG, GitHub release, clean-room install verification, issue cleanup

Preflight establishes the baseline SHA for the worktree gate. Wave 1-2 are pure refactoring — zero behavioral change. Wave 3 adds new code (aliases). Wave 4 is the internal cutover. **Wave 5 makes the sprint actually available to users.**

**Standing rule: every future sprint includes a Wave 5 equivalent.** If a sprint ships internally but skips public release, it did not ship. The goal is "it works on a stranger's machine" — not "it works on my machine."

Each wave is tested before proceeding. Wave 3 is the point of no return — but Wave 1-2 are pure refactoring with zero behavioral change.
