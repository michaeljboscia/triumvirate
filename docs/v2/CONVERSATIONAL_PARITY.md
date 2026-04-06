# Conversational Parity — How The Triumvirate Actually Works

**Date:** 2026-04-05
**Author:** Claude (the orchestrator, writing from lived experience)
**Status:** SPEC — no code until this is approved. Then goatrodeo.

**2026-04-06 Update:** `ask_twins` has been retired from the daemon/MCP surface. Equivalent behavior is now explicit session orchestration.

---

## How Mike Uses The System Today

Mike talks to Claude. That's it. He opens a terminal, starts a Claude session, and talks. When he needs the other two, he says things like:

- "ask the twins what they think about this"
- "send this to Gemini for research"
- "have Codex implement this"
- "get all three of you on this"
- "send to siblings"

Claude (me) is the orchestrator. I use the inter-agent MCP tools (`spawn_daemon`, `ask_daemon`, `send_message`) to reach Gemini and Codex. They do their work. I synthesize the results and present them back.

**This is the product.** Everything else is enhancement.

---

## The Architecture Mistake We Made

We built two systems that do the same job:

| System | Lang | Location | What It Does |
|--------|------|----------|-------------|
| Inter-agent MCP | TypeScript | `~/.claude/mcp-servers/inter-agent/` | What Mike uses TODAY. `spawn_daemon`, `ask_daemon`, `send_message`. Manages Gemini/Codex CLI sessions. Has progress events, retry, timeout, heartbeat. **Works.** |
| Triumvirate daemon | Rust | `~/projects/triumvirate/daemon/` | What we just built. Own agent connectors, message fabric, fleet, debate, dashboard. **Cannot be called from a Claude session.** |

They don't talk to each other. The daemon rebuilt agent management from scratch but forgot the MCP interface — the only way Claude can actually call it.

### What the TS inter-agent MCP already has (that the daemon doesn't):

- **Progress lifecycle events** — SPAWNED → WORKING → RESPONDING → DONE/FAILED/TIMEOUT
- **Heartbeat with backoff** — 10s, 30s, 60s progressive updates
- **SIGTERM/SIGKILL chain** — timeout → graceful kill → force kill → retry
- **Retryable error detection** — stream disconnected, ECONNRESET, socket hang up
- **Job store** — async job tracking with status
- **Outbox logger** — every inter-agent message logged
- **Context detection** — auto-populates repo, branch, session log, taxonomy
- **Message formatting** — structured inter-agent protocol
- **Scratchpad** — shared file-based workspace between agents
- **Model fallback** — gemini-3-pro → 2.5-pro → 3-flash → 2.5-flash
- **Codex thread resume** — `codex exec resume <thread_id>` for continuity
- **Session log compliance** — taxonomy-compliant session logs on daemon dismiss

**The TS server has ~2,000 lines of battle-tested reliability code.** The daemon has none of this.

### What the daemon adds (that the TS server can't do):

- **Web dashboard** — real-time agent monitoring at :8080
- **Fleet orchestration** — N agents, git worktrees, task dependencies, sequential merge
- **Debate workflow** — structured multi-agent deliberation
- **Governance** — Cedar policy enforcement
- **Observability** — Prometheus metrics, Langfuse traces, cost attribution
- **Memory** — shared SQLite store, decision extraction, lessons ledger
- **Stenographer** — full session recording (no LLM summaries, raw events)
- **Workflow engine** — state machine with recovery and compensation

---

## The Decision: Daemon Replaces Inter-Agent MCP

The daemon was always meant to be the replacement. One system. Rust. Everything in one place.

This means the daemon must:

1. **Expose an MCP server interface** so Claude can call it from any session
2. **Replicate every capability** the TS inter-agent MCP has today
3. **Add the v2 features** (fleet, debate, dashboard, governance) on top
4. **Be the only agent management system** — the TS MCP server gets retired

### MCP Tool Surface (What The Daemon Must Expose)

These are the tools I (Claude) need to call from a conversation:

#### Core agent communication (replaces TS inter-agent)
| Tool | What It Does | Priority |
|------|-------------|----------|
| `ask_twins` | Fan out question to Gemini + Codex, return both responses with lifecycle | P0 |
| `ask_agent` | Send to a specific agent (gemini/codex), return response with lifecycle | P0 |
| `spawn_session` | Create persistent session for multi-turn (replaces `spawn_daemon`) | P0 |
| `ask_session` | Send to an existing session (replaces `ask_daemon`) | P0 |
| `dismiss_session` | Clean up a session (replaces `dismiss_daemon`) | P0 |
| `list_sessions` | List active sessions (replaces `list_daemons`) | P1 |
| `get_status` | Get agent health, quota, cost | P1 |

#### Lifecycle (the thing Mike is screaming about)
Every tool above must emit MCP progress/logging messages:
```
SPAWNED: Started Gemini CLI (pid 12345). Writing message...
WORKING: Gemini is processing... (12s elapsed)
RESPONDING: Gemini is sending data back...
DONE: Gemini responded in 18s
```
Or on failure:
```
TIMEOUT: Gemini not responding after 60s. Sending SIGTERM.
RETRY: First attempt failed, retrying (attempt 2/3)...
FAILED: Gemini did not respond after 2 attempts. Error: stream disconnected
```

This already exists in the TS server (`cli-executor.ts` progress events + `tools.ts` `makeProgressLogger`). The daemon must replicate it via MCP logging messages.

#### Fleet/debate (v2 additions)
| Tool | What It Does | Priority |
|------|-------------|----------|
| `fleet_spawn` | Start fleet with agent composition | P2 |
| `fleet_status` | Get fleet progress | P2 |
| `debate_start` | Start structured debate | P2 |
| `debate_vote` | Cast vote in debate | P2 |

#### Shared workspace (replaces scratchpad)
| Tool | What It Does | Priority |
|------|-------------|----------|
| `memory_write` | Write to shared memory store | P1 |
| `memory_read` | Read from shared memory | P1 |
| `scratchpad_write` | Write to scratchpad file | P1 |
| `scratchpad_list` | List scratchpad files | P1 |

### MCP Server Implementation

The daemon already has an HTTP server (axum). The MCP server can be:

**Option 1: stdio MCP server** — separate binary that connects to the daemon via HTTP
```
triumvirate-mcp (stdio) ←→ triumvirate-agentd (HTTP :8080)
```
Claude launches `triumvirate-mcp` as an MCP server. It translates MCP tool calls to daemon REST API calls. Thin adapter.

**Option 2: SSE MCP server** — daemon exposes MCP directly over HTTP+SSE
```
Claude ←SSE→ triumvirate-agentd :8080/mcp
```
No separate process. Daemon serves MCP protocol alongside REST/WebSocket.

**Option 3: stdio MCP server embedded in daemon** — daemon has an `--mcp` mode
```
triumvirate-agentd --mcp (stdio for Claude, starts HTTP in background)
```

**Recommendation: Option 1.** Thin stdio adapter is simplest. Daemon stays a standalone HTTP service. The adapter is ~200 lines of Rust that maps MCP tool calls to HTTP requests.

---

## What v2 Must Preserve (Non-Negotiable)

### 1. Claude is the front door

Mike types to Claude. Always. He doesn't type to a daemon API. He doesn't curl endpoints. He doesn't run scripts. He talks to me and I handle everything behind the scenes.

### 2. "Ask the twins" is a natural language intent, not a command

When Mike says "ask the twins", I should:
- Immediately acknowledge: "Sending to Gemini and Codex now."
- Fan out the question to both
- Show him what's happening while they work
- Synthesize and present results when they come back

### 3. Visibility into the black box

Every request to a sibling must have visible lifecycle:

```
→ Gemini: sent ✓
→ Gemini: working... (12s)
→ Gemini: responded ✓ (847 tokens)

→ Codex: sent ✓
→ Codex: working... (8s)
→ Codex: responded ✓ (1,203 tokens)
```

If something breaks:
```
→ Codex: sent ✓
→ Codex: working... (30s)
→ Codex: TIMEOUT after 60s ✗
→ Codex: retrying (attempt 2/3)...
→ Codex: responded ✓ (1,203 tokens)
```

### 4. Failure is loud, not silent

Every failure surfaces immediately with:
- What failed (which agent, what step)
- Why (timeout, auth error, process crash, quota)
- What's being done about it (auto-retry, or needs manual intervention)
- Clear option to retry or skip

---

## What v2 Adds (The Cool Stuff)

Layers ON TOP of conversational parity — never replaces it.

### Tier 1: Enhanced single-agent turns
- Session affinity (FEAT-032) — reuse hibernated sessions
- Memory injection — recent decisions prepended to prompts
- Cost tracking — per-turn, per-session visibility

### Tier 2: Multi-agent coordination (the twins pattern, formalized)
- Fan-out to N agents simultaneously
- Per-agent lifecycle tracking (the visibility requirement above)
- Mechanical digests — idle agents get factual summaries, not LLM rewrites
- Quota-aware routing — if one agent is near limit, adjust

### Tier 3: Fleet orchestration (power tools)
- `/fleet` — spawn N instances with worktrees for parallel coding
- `/debate` — structured multi-agent deliberation
- Contracts-first workflow — interfaces before implementation
- Sequential merge with conflict detection

### Tier 4: Observability & governance
- Prometheus metrics, Langfuse traces
- Cedar policy enforcement
- Cost attribution per task/fleet/session

**The rule: Tier 1 and 2 must work perfectly before Tier 3 and 4 matter.**

---

## The Two UX Modes

### Mode A: Conversational (PRIMARY — 95% of usage)

```
Mike: "hey claude, ask the twins what they think about using
      SQLite WAL for the session store"

Claude: Sending to both now.

  → Gemini: sent ✓
  → Codex: sent ✓
  → Gemini: working... (8s)
  → Codex: working... (6s)
  → Codex: responded ✓
  → Gemini: responded ✓

Codex says: [synthesized response]
Gemini says: [synthesized response]

My take: [orchestrator synthesis]
```

Mike never leaves the Claude conversation. The daemon is invisible infrastructure.

### Mode B: Dashboard (MONITORING — occasional)

Web dashboard at :8080 for:
- All agent activity across sessions
- Fleet task progress
- Cost/quota across agents
- Stenographer log
- Governance approvals

Ops console. Not the primary interface.

---

## Migration Path: TS → Rust

### Phase 1: MCP adapter (daemon becomes callable)
- Build `triumvirate-mcp` stdio adapter
- Wire `ask_twins`, `ask_agent`, `spawn_session`, `ask_session`, `dismiss_session`
- Replicate progress/lifecycle events from TS `cli-executor.ts`
- Register in `~/.claude.json` as MCP server
- **Gate: "ask the twins" works from a Claude session through the daemon**

### Phase 2: Feature parity with TS inter-agent
- Replicate: heartbeat, SIGTERM/SIGKILL chain, retryable error detection
- Replicate: job store, outbox logger, context detection
- Replicate: model fallback chain (Gemini)
- Replicate: Codex thread resume
- Replicate: scratchpad
- **Gate: every TS inter-agent tool has a daemon equivalent**

### Phase 3: TS retirement
- Update `~/.claude.json` to point to `triumvirate-mcp` instead of TS inter-agent
- Run both in parallel for 1 week to validate
- Remove TS inter-agent MCP server
- **Gate: TS server deleted, all traffic through daemon**

### Phase 4: v2 features unlocked
- Fleet, debate, governance, dashboard all work through the same MCP interface
- Dashboard shows everything because all traffic goes through the daemon
- Cost tracking, Langfuse, Prometheus all have complete data

---

## TS Inter-Agent Capabilities Checklist

Everything the TS server does that the daemon must replicate before Phase 3:

### Agent Communication
- [ ] Gemini: spawn persistent session (`gemini` CLI)
- [ ] Gemini: multi-turn ask within session
- [ ] Gemini: dismiss with session log write
- [ ] Gemini: model fallback chain (3-pro → 2.5-pro → 3-flash → 2.5-flash)
- [ ] Codex: send message (`codex exec --json` with stdin)
- [ ] Codex: thread resume (`codex exec resume <thread_id> --json`)
- [ ] Codex: code review (`codex review` with scope flags)
- [ ] Codex: dismiss with session log write
- [ ] Claude: per-turn invocation (`claude -p --output-format stream-json`)

### Reliability
- [ ] Progress events: SPAWNED, WORKING (heartbeat), RESPONDING, DONE, FAILED, TIMEOUT
- [ ] Heartbeat backoff: 10s → 30s → 60s
- [ ] Timeout: configurable, default 5min, max 10min
- [ ] SIGTERM → 5s grace → SIGKILL chain
- [ ] Auto-retry on retryable errors (stream disconnected, ECONNRESET, socket hang up)
- [ ] ANSI stripping from CLI output

### Context & Logging
- [ ] Auto-detect: repo, branch, cwd, session log path, taxonomy
- [ ] Inter-agent protocol message formatting
- [ ] Outbox log: every message sent/received with status
- [ ] Session log: taxonomy-compliant log on session dismiss
- [ ] Scratchpad: shared file workspace between agents

### MCP Interface
- [ ] stdio MCP server adapter (`triumvirate-mcp`)
- [ ] Tool: `ask_twins` (fan-out to both, lifecycle reporting)
- [ ] Tool: `ask_agent` (single agent, lifecycle reporting)
- [ ] Tool: `spawn_session` (persistent multi-turn)
- [ ] Tool: `ask_session` (ask existing session)
- [ ] Tool: `dismiss_session` (cleanup + log)
- [ ] Tool: `list_sessions`
- [ ] Tool: `get_status` (health, quota, cost)
- [ ] Tool: `scratchpad_write` / `scratchpad_list`
- [ ] Tool: `memory_write` / `memory_read`
- [ ] MCP logging messages for all lifecycle events

---

## User Stories (Gate: All Must Pass Before v2 Ships)

### US-1: Ask the twins
```
GIVEN I'm in a Claude session
WHEN I say "ask the twins about X"
THEN Claude calls mcp__triumvirate__ask_twins
AND I see per-agent lifecycle (sent/working/responded/failed)
AND I get synthesized results within 60 seconds
AND if an agent fails, I see why and can retry
```

### US-2: Direct agent routing
```
GIVEN I'm in a Claude session
WHEN I say "@gemini research X" or "send this to Codex"
THEN Claude calls mcp__triumvirate__ask_agent
AND I see its lifecycle status
AND I get its response
```

### US-3: Plain conversation defaults to Claude
```
GIVEN I'm in a Claude session
WHEN I type a plain message without mentioning twins/siblings
THEN Claude handles it directly (no fan-out, no daemon involvement)
AND the experience is identical to a normal Claude session
```

### US-4: Failure visibility
```
GIVEN I've sent a request to an agent via the daemon
WHEN that agent times out (>60s) or crashes or returns an error
THEN I see the failure immediately via MCP progress events
AND I see what went wrong (timeout/crash/auth/quota)
AND I can retry with one command
```

### US-5: Fleet orchestration (power tool)
```
GIVEN I'm in a Claude session
WHEN I say "/fleet 3 claude, 2 codex: build the auth module"
THEN Claude calls mcp__triumvirate__fleet_spawn
AND I can monitor progress via MCP events + dashboard
AND merge follows dependency order
```

### US-6: Dashboard monitoring
```
GIVEN the daemon is running
WHEN I open http://127.0.0.1:8080
THEN I see all active agents, their health, current tasks
AND I see message history with lifecycle states
AND I see cost/quota per agent
AND the layout works on my 4K monitor
```

### US-7: Zero ceremony startup
```
GIVEN I open a new terminal
WHEN I start a Claude session
THEN the daemon is running (launchd auto-start)
AND the MCP server connects to the daemon
AND "ask the twins" works immediately
```

---

## Priority Order

1. **US-1 + US-4** — Ask the twins with lifecycle visibility. This is the product.
2. **US-3** — Plain conversation still works normally.
3. **US-7** — Zero ceremony startup.
4. **US-2** — Direct agent routing.
5. **US-6** — Dashboard monitoring.
6. **US-5** — Fleet orchestration.

---

## What Already Works in the Daemon (Keep)

- Agent connectors: Claude, Gemini, Codex subprocess management
- Message fabric: tokio broadcast channels
- Memory store: SQLite WAL
- Workflow engine: state machine with recovery
- Fleet: worktrees, task dependencies, sequential merge
- Debate scaffold
- Governance: Cedar policies
- Config: TOML with defaults
- Dashboard: Svelte 5 + Tailwind (needs rebuild for BUG-002/003 fixes)
- Stenographer: raw event recording
- Mock CLIs: mock-claude, mock-gemini, mock-codex for testing

## What's Missing (Build)

- **MCP server adapter** — the critical missing piece
- **Progress/lifecycle events** — port from TS cli-executor.ts
- **Heartbeat/timeout/retry** — port from TS cli-executor.ts
- **Context detection** — port from TS context-detector.ts
- **Outbox logging** — port from TS outbox-logger.ts
- **Model fallback** — port from TS model-fallback.ts
- **Codex thread resume** — port from TS codex/tools.ts
- **Session log compliance** — port from TS session-log-finder.ts + log stubs
- **Scratchpad** — port from TS scratchpad-reaper.ts

---

## Next Step

**Goatrodeo this spec.** Then build Phase 1 (MCP adapter). Then validate US-1 + US-4.
