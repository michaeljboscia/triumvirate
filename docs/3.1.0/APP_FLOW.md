# v3.1 MCP Consolidation — App Flow

**Spec:** `specs/MCP_CONSOLIDATION.md`

---

## Process Model — Before

```
Claude Code
  ├── mcp__inter-agent__* → node start-unified.sh → TS MCP server (12 tools)
  │     spawn_daemon, ask_daemon, send_message, get_response, 
  │     dismiss_daemon, list_daemons, list_jobs, write_scratchpad,
  │     list_scratchpad, pythia_query, pythia_corpus_health, code_review
  │
  ├── mcp__triumvirate__* → triumvirate mcp → Rust daemon (35+ tools)
  │     spawn_session, ask_session, dismiss_session, list_sessions,
  │     ask_agent, dispatch_codex, dispatch_codex_worktree, fleet_*,
  │     ledger_*, lesson_*, memory_*, scratchpad_*, review_*, etc.
  │
  └── mcp__pythia-gtm__* → node pythia shim → Pythia MCP
        lcs_investigate, pythia_corpus_health, pythia_force_index, etc.
```

**Problem:** Two processes for overlapping functionality. Skills split across both.

## Process Model — After

```
Claude Code
  ├── mcp__triumvirate__* → triumvirate mcp → Rust daemon (40+ tools)
  │     ALL inter-agent tools (via aliases) + ALL existing Rust tools
  │     spawn_daemon (alias), ask_daemon (alias), send_message (alias→ask_session),
  │     dismiss_daemon (alias), list_daemons (alias), code_review (alias),
  │     + spawn_session, ask_session, dispatch_codex_worktree, fleet_*, etc.
  │
  └── mcp__pythia-gtm__* → node pythia shim → Pythia MCP (unchanged)
```

**Result:** One Rust process serves all inter-agent + ABE + fleet + ledger + lessons tools. The TS inter-agent MCP server (`mcp-server/`) is archived and no longer runs. The Pythia MCP shim (`node pythia/dist/daemon/shim.js`) is unchanged and still runs — it is a separate MCP server for Pythia/Oracle functionality and is outside this sprint's scope. Net effect: **inter-agent no longer requires Node.js; Pythia still does** (Pythia Node consolidation is a potential future sprint, not 3.1.0).

---

## Tool Call Flow — Alias Path

```
Claude calls: mcp__triumvirate__spawn_daemon { target: "gemini", session_name: "review" }
  │
  ├─ rmcp stdio transport receives tool call
  ├─ tool_router dispatches to aliases::spawn_daemon handler
  ├─ Alias handler:
  │   1. Maps parameters: { target: "gemini" } → { agent: "gemini" }
  │   2. Preserves: session_name, cwd, timeout_ms
  │   3. Logs: tracing::info!("tool_alias", old="spawn_daemon", new="spawn_session")
  │   4. Delegates to: inter_agent::spawn_session(mapped_params)
  │
  ├─ spawn_session handler:
  │   1. Creates SessionState in self.sessions
  │   2. Spawns CLI subprocess via agent_adapter
  │   3. Returns { session_id: "gd_review", status: "ready" }
  │
  └─ Response flows back: handler → tool_router → rmcp → stdio → Claude
```

## Tool Call Flow — Direct Path (Existing, Unchanged)

```
Claude calls: mcp__triumvirate__dispatch_codex_worktree { task_id: "T-001", ... }
  │
  ├─ rmcp stdio transport receives tool call
  ├─ tool_router dispatches to abe::dispatch_codex_worktree handler
  ├─ ABE handler (moved from main.rs to mcp-tools/src/abe.rs):
  │   1. Same logic as before, just in a different file
  │   2. Receives narrowed interface: TaskTracker + ObservabilityBus
  │   3. No behavioral change
  │
  └─ Response flows back unchanged
```

## Skill Migration Flow — send-to-codex (Before)

```
/send-to-codex "review this code"
  │
  ├─ Skill calls: mcp__inter-agent-codex__send_message { question: "review this code" }
  │   → TS server dispatches to Codex CLI
  │   → Returns immediately: { job_id: "j_123" }
  │
  ├─ Skill calls: mcp__inter-agent-codex__get_response { job_id: "j_123" }
  │   → TS server polls job state
  │   → Returns: { response: "..." }
  │
  └─ Claude displays response
```

## Skill Migration Flow — send-to-codex (After)

```
/send-to-codex "review this code"
  │
  ├─ Skill calls: mcp__triumvirate__ask_session { name: "codex", message: "review this code" }
  │   → Rust daemon sends to Codex CLI session
  │   → Waits for response (tokio async, Claude shows "working...")
  │   → Returns: { response: "..." }
  │
  └─ Claude displays response
```

**Simpler:** One call instead of two. Claude Code natively handles the wait.

---

## Crate Boundary Flow

```
                        ┌──────────────┐
                        │ triumvirate  │  main.rs (~300 lines)
                        │  (binary)    │  CLI parsing, config, startup wiring
                        └──────┬───────┘
                               │
                 ┌─────────────┼─────────────┐
                 │             │             │
          ┌──────▼──────┐ ┌───▼────┐ ┌──────▼──────┐
          │  mcp-tools  │ │daemon- │ │  daemon-    │
          │  (MCP layer)│ │  http  │ │   core      │
          │             │ │(HTTP   │ │(state, cfg, │
          │ inter_agent │ │ layer) │ │ sessions)   │
          │ abe         │ │        │ │             │
          │ fleet       │ │ routes │ │ DaemonState │
          │ knowledge   │ │ ws     │ │ ObsBus      │
          │ review      │ │ metrics│ │ SessionMgr  │
          │ gemini_query│ └───┬────┘ └──────┬──────┘
          │ aliases     │     │             │
          └──────┬──────┘     │             │
                 │      ┌─────┴─────────────┘
                 │      │
          ┌──────▼──────▼──────────────────────────────┐
          │  Domain Crates (unchanged logic)            │
          │  agent-adapter, fleet, ledger, peer-review, │
          │  shared-types, fallback-outbox, mcp-bridge  │
          └────────────────────────────────────────────┘
```

Both `mcp-tools` and `daemon-http` call domain crates directly. Neither calls the other. Domain crates are the single source of truth for business logic.

---

## Migration Sequence (Wave 4 Detail)

```
Step 1: Add alias tools to triumvirate MCP server
  ├─ spawn_daemon, ask_daemon, dismiss_daemon, list_daemons → aliases
  ├─ write_scratchpad, list_scratchpad → aliases (name swap only)
  ├─ code_review → alias to review_request with schema mapping
  └─ Verify: call each alias from Claude, confirm response matches TS

Step 2: Update skills to use triumvirate tools
  ├─ inter-agent-protocol skill → mcp__triumvirate__spawn_session
  ├─ goatrodeo skill → mcp__triumvirate__spawn_session / ask_session  
  ├─ send-to-codex → mcp__triumvirate__ask_session
  ├─ send-to-gemini → mcp__triumvirate__ask_session
  ├─ send-to-siblings → mcp__triumvirate__ask_session
  └─ crystallize → mcp__triumvirate__ask_session

Step 3: Remove inter-agent from ~/.claude.json
  ├─ Delete the inter-agent MCP entry
  ├─ Verify: all tools still work through triumvirate
  └─ Archive: mv mcp-server/ archive/mcp-server-ts/

Rollback at any step: restore inter-agent entry in ~/.claude.json
```
