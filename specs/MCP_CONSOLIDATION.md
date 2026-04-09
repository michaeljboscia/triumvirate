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
- **A 6,216-line main.rs** (as of 2026-04-09, HEAD `373256451`) — all tool handlers, HTTP routes, metrics, and state management crammed into one file. Production code through line ~2,824; rest is test module.

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

### TS MCP Server Tools (unified-tools.ts — 12 tools, schemas verified against source)

Schemas verified against `mcp-server/src/unified-tools.ts` at HEAD 373256451. Any alias implementation must match these shapes exactly.

| TS Tool | Schema (verified from source) | File:Line | Semantics |
|---------|------------------------------|-----------|-----------|
| `spawn_daemon` | `{target: "gemini"\|"codex", cwd?, session_name?, timeout_ms?}` | unified-tools.ts:50-72 | Spawn persistent Gemini or Codex session |
| `send_message` | `{target, request_type, question, context?, cwd?, session_log?, timeout_ms?}` | unified-tools.ts:74-91 | Fire-and-forget message, returns job_id (TS workaround — eliminated in 3.1.0) |
| `get_response` | `{job_id, timeout_ms?}` | unified-tools.ts:224-232 | Poll for job result by ID (deprecated shim in 3.1.0) |
| `ask_daemon` | `{daemon_id, question, timeout_ms?}` | unified-tools.ts:139-159 | Send question to existing daemon, wait for response. **Param is `question`, NOT `message`.** |
| `dismiss_daemon` | `{daemon_id, hard?}` | unified-tools.ts:161-184 | Kill a daemon session. `hard` is Gemini-only (ignored for Codex). |
| `list_daemons` | `{target?, cwd?}` | unified-tools.ts:93-114 | List active sessions |
| `list_jobs` | `{target?, cwd?}` | unified-tools.ts:116-137 | List pending/completed jobs |
| `write_scratchpad` | `{topic, content, cwd?, owner?, daemon_id?}` | unified-tools.ts:186-222 | Write markdown artifact to shared scratchpad. **Params are `topic`/`content`/`owner`/`daemon_id`, NOT `key`/`value`/`namespace`.** Owner auto-derived from daemon_id prefix. |
| `list_scratchpad` | `{cwd?}` | unified-tools.ts:234-241 | List scratchpad entries. **Only param is `cwd`.** |
| `pythia_query` | `{question, intent, cwd?}` | unified-tools.ts:243-252 | Query Pythia — OUT OF SCOPE (not routed through inter-agent in practice) |
| `pythia_corpus_health` | `{cwd?}` | unified-tools.ts:254-261 | Pythia corpus health — OUT OF SCOPE |
| `code_review` | `{cwd?, uncommitted?, base_branch?, commit_sha?, timeout_ms?}` | unified-tools.ts:263-274 | Run `codex review` on uncommitted changes, base-branch comparison, or specific commit. **Params are `cwd`/`uncommitted`/`base_branch`/`commit_sha`, NOT `diff`/`context`/`target`.** |

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

### Tool Gap Analysis — post-Decision R1-D1

After killing the async job model (R1-D1: C), every TS tool maps to a Rust equivalent. There are zero unresolved gaps. The table below is the post-decision state, NOT the original pre-decision state.

| TS Tool | Rust Equivalent | Resolution |
|---------|----------------|------------|
| `spawn_daemon` | `spawn_session` | alias (name + `target`→`agent` param rename) |
| `send_message` | `ask_session` (synchronous) | alias — maps to sync call, returns response directly. Async pattern eliminated. |
| `get_response` | deprecated shim | returns deprecation notice pointing at `ask_session` |
| `ask_daemon` | `ask_session` | alias (name + `daemon_id`→`name`, `question`→`message`) |
| `dismiss_daemon` | `dismiss_session` | alias (name + `daemon_id`→`name`; drop `hard` param) |
| `list_daemons` | `list_sessions` | alias (name + optional `target` filter) |
| `list_jobs` | `get_status` (shape-mapped) | alias with shape translation |
| `write_scratchpad` | `scratchpad_write` | alias (name + params `topic/content/owner/daemon_id` mapped into Rust schema) |
| `list_scratchpad` | `scratchpad_list` | alias (name + `cwd` passthrough) |
| `pythia_query` | — | OUT OF SCOPE — not in TS inter-agent server in practice, stays in Pythia MCP |
| `pythia_corpus_health` | — | OUT OF SCOPE — same |
| `code_review` | `review_request` | alias with shape translation (`cwd/uncommitted/base_branch/commit_sha` → review request schema) |

**Real unresolved gaps: 0.** Every TS tool has an explicit resolution: alias, deprecated shim, or explicit out-of-scope.

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

**Context for REQ-J naming:** The original goat rodeo reserved REQ-J1 through REQ-J4 for the async job queue (send_message + get_response + list_jobs + TTL reaper). Round 1's Decision D1 killed the async model entirely. REQ-J1 (the queue itself) became DROPPED. REQ-J2–J4 were REUSED for the downstream consequence of that decision: three skill files had to be updated to use synchronous `ask_session` instead of the eliminated async pattern. The naming is historical — if you prefer clarity, read REQ-J2–J4 as "skill updates required because REQ-J1 was dropped." The J prefix is retained because the work traces back to the original async model's removal.

**REQ-J1:** ~~Async job queue~~ **DROPPED.** The `send_message` alias routes directly to `ask_session` (synchronous from Claude's perspective). No DashMap, no TTL reaper, no job IDs. This is a permanent decision, not a deferred one.

**REQ-J2:** (downstream of J1's drop) Update `send-to-codex` skill to use `mcp__triumvirate__ask_session` instead of `mcp__inter-agent-codex__send_message`. Orchestrator task (T-012) — not ABE-dispatched because the skill file lives outside the repo.

**REQ-J3:** (downstream of J1's drop) Update `send-to-gemini` skill to use `mcp__triumvirate__ask_session` instead of `mcp__inter-agent-gemini__send_message`. Orchestrator task (T-013).

**REQ-J4:** (downstream of J1's drop) Update `send-to-siblings` skill to use `mcp__triumvirate__ask_session` for both agents instead of per-agent `send_message` calls. Orchestrator task (T-014).

### Tool Aliases (Backwards Compatibility)

**REQ-A1:** Register backwards-compatible aliases for ALL 12 TS inter-agent tools so existing Claude skills, prompts, and CLAUDE.md instructions continue to work. This is the canonical alias matrix — every TS tool has an explicit status (alias-to-Rust, canonical-native, or intentionally-unsupported):

| TS tool (old name) | Status | Routes to (Rust handler) | Notes |
|---------------------|--------|--------------------------|-------|
| `spawn_daemon` | alias | `spawn_session` | Map `target` → `agent`, preserve `session_name`, `cwd`, `timeout_ms` |
| `ask_daemon` | alias | `ask_session` | Map `daemon_id` → `name` (preserving `gd_`/`cd_` prefix), `question` → `message` |
| `dismiss_daemon` | alias | `dismiss_session` | Map `daemon_id` → `name`. Drop `hard` param (Rust doesn't support — log warning if passed) |
| `list_daemons` | alias | `list_sessions` | Optional `target` filter applied post-fetch |
| `send_message` | alias (synchronous) | `ask_session` | Map `target` → `name`, `question` → `message`. Auto-spawn session if needed. Returns response directly (NOT a job_id — the async pattern was eliminated per Decision R1-D1) |
| `get_response` | deprecated shim | returns static message | "Use ask_session directly — the async job queue was removed in 3.1.0. See /Users/mikeboscia/.claude/skills/send-to-codex/SKILL.md for the new synchronous pattern." |
| `list_jobs` | alias | `get_status` (shape-mapped) | Returns a list with `job_id=session_id`, `state=agent_state`, `target=agent` to preserve the old shape |
| `write_scratchpad` | alias | `scratchpad_write` | Name swap only — schemas are identical |
| `list_scratchpad` | alias | `scratchpad_list` | Name swap only |
| `pythia_query` | OUT OF SCOPE | — | Not in TS server anyway (server.ts:5). Stays in Pythia MCP. No alias needed. |
| `pythia_corpus_health` | OUT OF SCOPE | — | Same — stays in Pythia MCP. |
| `code_review` | alias (schema-mapped) | `review_request` | Map `diff` + optional `context` to review request schema |

**Count check:** 10 aliases (8 active aliases + 1 deprecated shim + 1 shape-mapped), 2 out-of-scope (never in TS server). All 12 TS tools accounted for.

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
~/.claude.json → triumvirate daemon              (Rust, ALL inter-agent + ABE tools)
~/.claude.json → pythia MCP                       (separate, unchanged)
```

**Oracle tools note:** The TS `mcp-server/` that this sprint archives does NOT contain oracle tools. `server.ts:5` states: "Oracle tools are NOT registered here — they live in Pythia." Oracle/Pythia functionality stays in its own existing MCP server (`mcp__pythia-gtm__`) and is untouched by this sprint. REQ-P1/P2 were dropped after this was verified by code inspection.

### File Changes

| File | Change |
|------|--------|
| `daemon/crates/triumvirate/src/main.rs` | Shrinks from 6,216 lines (current) to under 300 (startup only) |
| `daemon/crates/mcp-tools/src/*.rs` | NEW — 8 modules (lib, inter_agent, abe, fleet, knowledge, review, gemini_query, aliases), ~2,500 lines extracted from main.rs |
| `daemon/crates/mcp-tools/Cargo.toml` | Updated deps — all domain crates |
| `daemon/crates/daemon-http/src/lib.rs` | Expanded — HTTP routes extracted from main.rs |
| `daemon/crates/daemon-core/src/lib.rs` | Expanded — DaemonState, config, sessions |
| `daemon/crates/daemon-core/src/version.rs` | NEW — env!("CARGO_PKG_VERSION") constants |
| `daemon/Cargo.toml` | Workspace version bumped from 0.1.0 → 3.1.0 |
| `scripts/version-drift-check.sh` | NEW — pre-commit hook script (tracked in repo) |
| `scripts/install-git-hooks.sh` | NEW — installer that symlinks the hook into .git/hooks/ |
| `/Users/mikeboscia/.claude.json` | Updated — inter-agent entry removed. Orchestrator-executed with user approval (not in repo). |
| `mcp-server/` | Archived to `archive/mcp-server-ts/` via git mv |
| `/Users/mikeboscia/.claude/skills/send-to-codex/SKILL.md` et al. | Updated — orchestrator edits (not in repo) |

**`.git/hooks/pre-commit`** is NOT in this list. It is local state, not repo state. The hook logic lives in `scripts/version-drift-check.sh` (tracked) and is symlinked into `.git/hooks/` by `scripts/install-git-hooks.sh` which each developer runs once after cloning.

---

## Acceptance Criteria

1. `cargo test --workspace` passes after all changes
2. `cargo build --release` produces a single binary that serves both MCP (stdio) and HTTP
3. Every TS MCP tool name works through the Rust daemon (via alias or native)
4. `send-to-codex`, `send-to-gemini`, and `send-to-siblings` skills work via `ask_session`
5. `~/.claude.json` has no `inter-agent` entry. The `triumvirate` entry (Rust binary) serves all inter-agent + ABE tools. The `pythia` MCP entry is unchanged.
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
