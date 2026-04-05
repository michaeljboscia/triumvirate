# Triumvirate v2 — Final Specification

**Date:** 2026-04-05
**Status:** FINAL — Goat Rodeo complete (9 rounds, 62 questions, 52 decisions)
**Builds from:** CONVERSATIONAL_PARITY.md + 9 rounds of architecture review
**Built by:** Codex | **Reviewed by:** Claude + Gemini + Codex + Mike

---

## Constitution (Inviolable Principles)

1. **Claude is the front door.** User talks to Claude. Always.
2. **Lifecycle is always visible.** No silent failures. Ever.
3. **Plain language in, structured results out.** No command ceremony.
4. **Failure is loud, immediate, and actionable.**
5. **Agents are always reachable.** Four fallback layers. Never "unavailable."

---

## User Stories

### US-1: Ask the Twins
```
GIVEN I'm in a Claude session
WHEN I say "ask the twins about X" (or any explicit trigger)
THEN Claude calls mcp__triumvirate__ask_twins
AND both agents receive role-adapted prompts simultaneously
AND I see per-agent lifecycle (sent/working/responded/failed) in real-time
AND results return non-blocking (Gemini at 12s, Codex at 25s — I see each as it arrives)
AND Claude evaluates quality, flags disagreements, recommends the better answer
AND I get synthesized results
```

### US-2: Direct Agent Routing
```
GIVEN I'm in a Claude session
WHEN I say "@gemini research X" or "send this to Codex" (explicit trigger)
THEN Claude calls mcp__triumvirate__ask_agent
AND I see lifecycle status for that agent
AND I get its response
```

### US-3: Plain Conversation
```
GIVEN I'm in a Claude session
WHEN I type a plain message without an explicit trigger
THEN Claude handles it directly (no daemon involvement)
AND the experience is identical to a normal Claude session
AND the MCP server's presence adds <=10ms latency (release blocker if exceeded)
```

### US-4: Failure Visibility
```
GIVEN I've sent a request to an agent
WHEN that agent fails
THEN I see the failure IMMEDIATELY
AND I see each retry attempt in real-time (up to 3, jittered backoff)
AND after 3 failures: dead drop fallback launches automatically
AND Claude tells me: "Codex failed. Gemini answered. Fallback launched. Want to retry or move on?"
AND dead drop result surfaces on my next MCP tool call
```

### US-5: Fleet Orchestration
```
GIVEN I'm in a Claude session
WHEN I say "/fleet 3 claude, 2 codex: build the auth module"
THEN the daemon spawns agents with worktrees
AND Claude receives headline milestones (task claimed/completed/merged)
AND detailed per-agent events go to dashboard only
AND merge follows dependency order
```

### US-6: Dashboard Monitoring
```
GIVEN the daemon is running
WHEN I open http://127.0.0.1:8080
THEN I see all active agents, their health, current tasks
AND I see BOTH MCP-path and fabric events, correlated by request_id
AND I see message history with lifecycle states
AND I see quota per agent (not cost — subscriptions, not API)
AND the layout works on my 4K monitor
```

### US-7: Zero Ceremony Startup
```
GIVEN I open a new terminal and start a Claude session
THEN the daemon is already running (launchd)
AND the MCP bridge connects to the daemon
AND if daemon is down: ONE auto-start attempt, then RED STATE with exact error
AND "ask the twins" works immediately
```

### US-8: Persistent Agent Sessions
```
GIVEN I'm working on a multi-hour task
WHEN I spawn a persistent session (or first ask triggers JIT spawn)
THEN agents stay ALIVE indefinitely (no kill, no hibernate, no TTL)
AND I can ask follow-up questions that build on prior context
AND sessions are machine-wide (shared across all terminals/projects)
AND 3-4 active projects = 6-8 sessions (machine handles it fine)
AND machine restart = clean slate (fresh spawns)
```

### US-9: Proactive Agent Contribution (Future — Tier 2+)
```
GIVEN agents are alive with project context
WHEN an agent detects something relevant to current work
THEN it surfaces it to Claude unprompted
AND Claude presents it: "Gemini noticed: [insight]"
AND the user can acknowledge or dismiss
```

---

## Architecture

### Two-Mode Binary

```
triumvirate daemon   — launchd-managed, persistent, owns everything
triumvirate mcp      — short-lived stdio bridge, Claude spawns/kills freely
```

One Rust codebase. Two entrypoints. Daemon persists across Claude sessions. MCP bridge is ephemeral — dying doesn't affect agents, sessions, or dashboard.

### Process Layout

```
Claude ←stdio→ triumvirate mcp ←HTTP→ triumvirate daemon ←stdio→ Gemini/Codex CLIs
                                         ├── HTTP :8080 (dashboard)
                                         ├── SQLite (sessions, outbox, memory)
                                         └── Agent processes (alive, contextualized)
```

### Technology

| Component | Technology |
|-----------|-----------|
| MCP bridge | Rust + `rmcp` crate (stdio transport, progress notifications, logging) |
| Daemon HTTP | Rust + `axum` |
| Agent connectors | `tokio::process::Command` piped stdio |
| Storage | SQLite WAL (`rusqlite`) |
| Event parser | Carry over from proto crate (BUG-001 fixes) |
| Dashboard | Svelte 5 + Tailwind (deferred, existing code as reference) |
| Mock CLIs | Carry over (mock-claude, mock-gemini, mock-codex) |

### Daemon Does NOT Speak MCP

The daemon is a pure HTTP/axum server. The MCP bridge is the only MCP-speaking component. This keeps the daemon usable by the dashboard, future CLI tools, and any non-MCP client.

### Local Auth

Daemon generates auth token on startup → saves to `~/.triumvirate/daemon.token`. MCP bridge reads token and passes as Bearer on every HTTP request. Prevents local process hijacking.

### Thread Safety

Daemon APIs are thread-safe for concurrent MCP bridges (3+ Claude sessions). SQLite transactions, request IDs, idempotent APIs. `Arc<tokio::sync::Mutex>` for shared state.

---

## Explicit Trigger Routing

Fan-out to agents ONLY fires on explicit phrases:
- "ask the twins", "send to siblings", "ask the twins about X"
- "@gemini", "send to Gemini", "send this to Gemini"
- "@codex", "send to Codex", "have Codex implement this"
- `/send-to-siblings`, `/send-to-gemini`, `/send-to-codex` (skill aliases → MCP tools)

Everything else stays with Claude. No intent detection. No false positives. Expandable later via user config.

Claude passes `cwd`, `repo_root`, `branch`, `session_log_path` explicitly on every MCP tool call. The bridge does not infer context.

---

## MCP Tool Surface

### Core (replaces TS inter-agent)

| Tool | What It Does |
|------|-------------|
| `ask_twins` | Fan out to Gemini + Codex with role-adapted prompts. Progress notifications throughout. Returns both responses. |
| `ask_agent` | Send to a specific agent with lifecycle. |
| `spawn_session` | Create persistent named session for multi-turn. |
| `ask_session` | Ask within an existing session. |
| `dismiss_session` | Clean up session + write session log. |
| `list_sessions` | List active/alive sessions. |
| `get_status` | Agent health, quota, pending fallback results. |

### Shared Workspace

| Tool | What It Does |
|------|-------------|
| `memory_write` | Write to shared SQLite memory store. |
| `memory_read` | Read from shared memory. |
| `scratchpad_write` | Write to project scratchpad file. |
| `scratchpad_list` | List scratchpad files. |

### Fleet/Debate (Tier 3 — deferred)

| Tool | What It Does |
|------|-------------|
| `fleet_spawn` | Start fleet with agent composition. |
| `fleet_status` | Get fleet progress. |
| `debate_start` | Start structured debate. |
| `debate_vote` | Cast vote in debate. |

---

## Lifecycle Visibility

Every tool emits MCP progress notifications via `rmcp`:

```
SPAWNED: Started Gemini CLI (pid 12345). Writing message...
WORKING: Gemini is processing... (12s elapsed) [stage: reading files]
RESPONDING: Gemini is sending data back...
DONE: Gemini responded in 18s
```

On failure:
```
TIMEOUT: Gemini not responding after 60s. Sending SIGTERM.
RETRY: Attempt 2/3, jittered backoff...
FAILED: Gemini did not respond after 3 attempts. Error: stream disconnected
FALLBACK: Dead drop launched in Terminal. Tracking PID 67890.
```

### Activity Stages

Coarse stages derived from connector telemetry:
- `bootstrapping` — agent CLI starting up
- `reading files` — agent reading project context
- `planning` — agent reasoning about approach
- `drafting response` — agent generating output
- `finalizing` — agent completing response

### Event Tiers

| Destination | What It Gets |
|------------|-------------|
| Claude (MCP progress) | Headlines: sent, working (with stage), retry, failed, done |
| Dashboard (HTTP/WebSocket) | Full firehose: every event, every agent, every state change |

Fleet operations get periodic rollups in Claude (e.g., "3/5 agents done") with full detail in dashboard only.

---

## Retry + Fallback Chain

### Layer 1: Direct (fast path)
Claude → MCP bridge → daemon → alive agent → response

### Layer 2: Retry (transient failures)
3x retry with jittered backoff (250ms, 1s, 2s). Every attempt visible. Retryable errors: stream disconnected, ECONNRESET, socket hang up.

### Layer 3: Dead Drop (daemon-level failure)
After 3 failures:
1. Daemon writes detailed dead drop: `{date}_{time}_{agent}_{project}_{hash}.md`
2. Daemon spawns agent in Terminal window via `osascript` (detect `$TERM_PROGRAM`, fall back to Terminal.app)
3. PID captured and tracked by daemon
4. Agent works the prompt, writes `_response.md` to same directory
5. On Claude's next MCP tool call, bridge checks for completed fallback results
6. Claude presents: "Fallback completed. Here's what Codex said."

Dead drop GC: completed pairs deleted after 48h. Failed drops (no response after 24h) flagged in diagnostic log. Everything older than 7 days auto-deleted on daemon startup.

### Layer 4: TS Inter-Agent (migration safety net)
During migration, TS inter-agent MCP server remains registered. If Rust daemon fails catastrophically, swap `~/.claude.json` config back to TS. One-line change.

### Layer 5: Manual
User opens a terminal and talks to Gemini or Codex directly. The agents are on the machine. They're always reachable.

---

## Failure Pattern Detection

Daemon tracks failure rates per agent. If threshold exceeded (e.g., >5% daily or 3 consecutive failures), daemon injects warning into next MCP response:

> "Note: Codex has failed its last 3 requests. Common error: MCP ceremony timeout. This may indicate a CLI update broke the connector."

Diagnostic logging on every dead drop trigger: agent, error chain, timestamps, request content. Postmortem package auto-assembled per incident.

---

## Agent Session Management

### JIT Spawn
Agents NOT pre-warmed on daemon boot. First `ask_twins` or `ask_agent` call spawns the session. Lightweight bootstrap prompt: "Session bootstrap: acknowledge readiness. Workspace: <path>."

### Stay Alive
Once spawned, sessions stay alive indefinitely. No TTL. No kill. No hibernate. Machine restart = clean slate.

### Context Per Request
Real project context injected per request from task metadata (CLAUDE.md, relevant files), not dumped on spawn. Keeps bootstrap cheap, requests contextualized.

### Machine-Wide
Daemon is machine-wide (launchd). Agent sessions shared across all terminals and projects. Ask twins from terminal 1 → warm. Ask from terminal 2 → same sessions, instant.

### Concurrent Access
Multiple Claude sessions can hit the daemon simultaneously. FIFO queue per project. If queue delay exceeds threshold, spawn temporary overflow session. Emit `reused|queued|overflow_spawned` in lifecycle.

---

## Prompt Adaptation

`ask_twins` sends the user's raw question to both agents, wrapped in role-specific templates:

- **Gemini:** research/evidence/analysis framing
- **Codex:** implementation/tradeoffs/testing framing

Templates are hardcoded defaults in the daemon binary. User can override via `~/.triumvirate/templates/`. Bad templates → delete override, revert to compiled-in defaults.

Claude evaluates both responses: confidence per agent, explicit disagreement detection, recommendation. Not blind equal-weight presentation.

---

## TS Parity Checklist (All Must Pass Before Migration)

### Agent Communication
- [ ] Gemini: spawn persistent session
- [ ] Gemini: multi-turn ask within session
- [ ] Gemini: dismiss with session log write
- [ ] Gemini: model fallback chain (--model flag)
- [ ] Codex: send message (codex exec --json with stdin)
- [ ] Codex: thread resume (codex exec resume <thread_id> --json)
- [ ] Codex: code review (codex review with scope flags)
- [ ] Codex: dismiss with session log write
- [ ] Claude: per-turn invocation (claude -p --output-format stream-json)

### Reliability
- [ ] Progress events: SPAWNED, WORKING (with activity stage), RESPONDING, DONE, FAILED, TIMEOUT
- [ ] Heartbeat backoff: 10s, 30s, 60s progressive updates
- [ ] Timeout: configurable, default 5min, max 10min
- [ ] SIGTERM → 5s grace → SIGKILL chain
- [ ] Auto-retry on retryable errors (stream disconnected, ECONNRESET, socket hang up)
- [ ] ANSI stripping from CLI output

### Context & Logging
- [ ] Auto-detect: repo, branch, cwd, session log path, taxonomy (from explicit MCP params)
- [ ] Inter-agent protocol message formatting
- [ ] Outbox log: every request/response/error in SQLite, indexed by request_id/session_id/status
- [ ] Session log: taxonomy-compliant log on session dismiss
- [ ] Scratchpad: shared file workspace between agents with reaper/TTL

### MCP Interface
- [ ] rmcp-based stdio MCP server (triumvirate mcp)
- [ ] Tool: ask_twins (fan-out, role adaptation, lifecycle)
- [ ] Tool: ask_agent (single agent, lifecycle)
- [ ] Tool: spawn_session (persistent multi-turn)
- [ ] Tool: ask_session (ask existing session)
- [ ] Tool: dismiss_session (cleanup + log)
- [ ] Tool: list_sessions
- [ ] Tool: get_status (health, quota, pending fallbacks)
- [ ] Tool: scratchpad_write / scratchpad_list
- [ ] Tool: memory_write / memory_read
- [ ] MCP progress notifications for all lifecycle events
- [ ] MCP logging messages for all status updates

### Dead Drop Fallback
- [ ] osascript Terminal spawn (detect $TERM_PROGRAM)
- [ ] PID tracking on launch
- [ ] Canonical naming: {date}_{time}_{agent}_{project}_{hash}.md
- [ ] Response detection on next MCP tool call
- [ ] GC: 48h completed, 7d everything
- [ ] Diagnostic logging per dead drop trigger
- [ ] Failure pattern detection + alert injection

### Infrastructure
- [ ] Local auth token (daemon.token, Bearer on every request)
- [ ] Thread-safe concurrent access (multiple MCP bridges)
- [ ] launchd plist generation
- [ ] `triumvirate install` CLI
- [ ] `triumvirate doctor` health check
- [ ] MCP registration in ~/.claude.json

---

## Reliability Baseline

Before migration, run 1,000 synthetic requests through the daemon:
- Mix of ask_twins, ask_agent, spawn/dismiss sessions
- Record: success rate, failure rate by type, response times (p50/p95/p99), retry rates
- Set SLO AFTER seeing the data, not before
- Floor: better than 84% (the TS system's measured rate), and every failure is visible

---

## Build Increments

Each increment produces a runnable, demonstrable slice. Real e2e test at EVERY increment.

### Increment 1a: Scaffold + Ping
- Cargo workspace in daemon-v2/
- rmcp setup with stdio transport
- Dummy `ping` tool validates Claude-to-MCP connection
- **Test:** Claude calls ping, gets pong

### Increment 1b: ask_agent Happy Path
- One agent connector (Gemini)
- `ask_agent` tool wired to mock-gemini
- Progress notifications (SPAWNED, WORKING, DONE)
- **Test:** "@gemini what is 2+2" → response with lifecycle events in Claude

### Increment 2: ask_twins
- Add Codex connector
- `ask_twins` fans out to both with role-adapted prompts
- Non-blocking: first response returns immediately, second follows
- **Test:** "ask the twins about X" → both respond, lifecycle visible

### Increment 3: Timeout + Retry + Failure
- Timeout detection (configurable, default 5min)
- 3x retry with jittered backoff, all visible
- SIGTERM/SIGKILL chain
- Loud failure surfacing
- **Test:** kill agent mid-request → TIMEOUT, RETRY, FAILED all visible

### Increment 4: Sessions
- spawn_session / ask_session / dismiss_session
- Session reuse (JIT spawn, stay alive)
- Session log write on dismiss
- **Test:** spawn Gemini, ask 3 questions in sequence, dismiss → multi-turn works

### Increment 5: Alive Sessions + Context
- Sessions stay alive indefinitely (no kill/hibernate)
- Lightweight bootstrap on spawn, context per request
- Machine-wide shared sessions
- **Test:** ask_twins, wait, ask again → same sessions, no cold start

### Increment 6: Outbox + Context Detection
- Outbox logging (every request/response/error to SQLite)
- Context detection from explicit MCP params
- Scratchpad
- **Test:** send 10 requests, query SQLite → all 10 logged with repo/branch

### Increment 7: Remaining TS Parity
- Model fallback (Gemini --model flag)
- Codex thread resume
- Heartbeat backoff
- ANSI stripping
- Session log compliance
- **Test:** TS parity checklist (all items), all pass

### Increment 8: Dead Drop Fallback
- osascript Terminal spawn
- PID tracking
- Canonical naming + GC
- Fallback result notification on next MCP call
- **Test:** stop daemon, ask twins → Terminal spawns, result comes back to Claude

### Increment 9: Diagnostics + Auth
- Failure pattern detection + alert injection
- Diagnostic logging + postmortem package
- Local auth token (daemon.token)
- Thread-safe concurrent access
- **Test:** 3 simultaneous Claude sessions, all work. Bad token rejected.

### Increment 10: Install + Ship
- `triumvirate install` CLI
- launchd plist generation
- `triumvirate doctor` health check
- MCP registration in ~/.claude.json
- Swap from TS to Rust
- Dashboard rebuild (if time permits, otherwise deferred)
- **Test:** `triumvirate install` on clean state → everything works. "Ask the twins" → lifecycle visible → response.

---

## Repo Strategy

- Branch: `feat/mcp-first`
- New code: `daemon-v2/`
- Old code: `daemon/` untouched as reference
- Proto crate event parsers, mock CLIs, SQLite schema: reference material
- Crate structure: Codex owns, granular preferred
- When all increments pass: `daemon-v2/` becomes `daemon/`, old code archived

---

## Migration

1. Build all 10 increments against mock CLIs + real CLIs
2. Run reliability baseline (1,000 requests)
3. Swap `~/.claude.json`: TS inter-agent → Rust triumvirate
4. If Rust breaks: one-line swap back to TS
5. After 1 week stable: archive TS inter-agent MCP server
6. Accept loss of 70 hibernated Gemini sessions (clean cutover)

---

## Out of Scope (Deferred)

- Fleet orchestration (Tier 3)
- Debate workflow (Tier 3)
- Governance / Cedar policies (Tier 4)
- Prometheus metrics (Tier 4)
- Langfuse traces (Tier 4)
- Stenographer (Tier 4)
- Dashboard rewrite (after backend stable)
- Proactive agent contribution / US-9 (Tier 2+)
- Intent-based routing (future, behind user config)
- Cost-per-token tracking (removed — subscriptions, not API)

---

## Anti-Patterns (Enforced)

| Anti-Pattern | Rule |
|-------------|------|
| **Stub trust** | No feature exists without a working implementation. No stubs. |
| **Passive hooks** | No feature relies on "the agent will notice this text." Tools must be explicitly invocable. |
| **Silent failure** | Every error path has user-visible notification. No exceptions. |
| **Invisible infrastructure** | Every system component has a defined user-facing integration point. |
| **False completion** | No workflow reports "done" without verification. |

---

## Goat Rodeo Provenance

9 rounds. 62 interrogator questions. 52 decisions (25 by user, 27 by clanker consensus). 2 crystallized lessons. 6 external research reports. 359 outbox messages mined. 118 daemon sessions cataloged. 3 live dead drop tests (Gemini, Codex, and Codex retry all confirmed via osascript Terminal spawn).

Decision ledger: `/Users/mikeboscia/projects/triumvirate/docs/v2/GOATRODEO_LEDGER.md`
Usage analysis: `/Users/mikeboscia/projects/triumvirate/research/usage-analysis-inter-agent-20260405.md`
UX research: `/Users/mikeboscia/projects/triumvirate/research/multi-agent-ux-patterns-20260405.md`
