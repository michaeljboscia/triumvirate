# Triumvirate v2 — Specification

**Date:** 2026-04-04
**Author:** Mike Boscia + Claude + Gemini + Codex
**Status:** Draft
**Origin:** nudge-reaper catastrophe → crystallize → first-principles rebuild

---

## What This Is

A Go binary (`triumvirate-agentd`) that runs on your machine and makes Claude, Gemini, and Codex work together as a team. Not a framework. Not a platform. Your everyday driver.

## The Four Requirements

Everything in this spec serves one of these. If it doesn't serve one, it's not in v1.

### REQ-1: 3-Agent Conversation
Claude, Gemini, and Codex in the same terminal, all talking, all live. The human is the fourth participant. Not note-passing through files — real-time streaming collaboration where all four participants see each other's output.

### REQ-2: Working Stenographer
Session notes that actually capture what happened, every time. No hallucinated summaries. No silent failures. Mechanical extraction of facts from the conversation, verified against reality.

### REQ-3: Reliable Memory
Shared across all three agents and the human. Never silently fails. Never loses data. Never diverges between agents. When Claude learns something, Gemini and Codex know it next time they're asked.

### REQ-4: It All Works Together
One system. One binary. One install. Not a pile of scripts hoping for the best. If any component fails, the system tells you — it doesn't silently degrade.

### REQ-5: Use All Three, All The Time
Three subscriptions. Three agents. All working simultaneously, not taking turns. When Claude is designing, Gemini is researching, and Codex is coding — in parallel. No agent sits idle while another thinks. The system should maximize utilization of every subscription dollar. When implementation starts, Codex writes Go. Claude architects. Gemini validates. All at the same time.

---

## Architecture

```
triumvirate-agentd (single Go binary)
│
├── Agent Connectors (persistent CLI subprocesses)
│   ├── Claude  (claude --resume <sid> | claude -p --stream-json)
│   ├── Gemini  (gemini CLI | future: Go SDK direct)
│   └── Codex   (codex CLI | future: Go SDK direct)
│
├── Message Fabric (embedded NATS + JetStream)
│   ├── agents.stream    — all agent messages, debate turns, responses
│   ├── tasks.stream     — task lifecycle events
│   ├── tools.stream     — tool invocations and results
│   └── memory.stream    — memory writes, reads, updates
│
├── Orchestration (Temporal.io)
│   ├── ConversationWorkflow  — 4-way live collaboration
│   ├── DebateWorkflow        — structured Toulmin debate with vote
│   └── TaskWorkflow          — plan → execute → verify
│
├── Memory (shared context fabric)
│   ├── SQLite WAL             — persistent key-value + event store
│   ├── NATS KV                — fast ephemeral state
│   └── Sync engine            — pushes memory to all agents on session start
│
├── Governance (embedded OPA)
│   ├── Rego policies          — what each agent can/can't do
│   └── Human approval gates   — destructive ops require confirmation
│
├── Observability (OpenTelemetry)
│   ├── Per-agent spans        — latency, tokens, cost per turn
│   └── Trace correlation      — session_id across all agents
│
└── TUI (BubbleTea)
    ├── Agent panes            — one per agent, streaming output
    ├── Human input pane       — always has priority
    ├── Event log pane         — NATS message stream (optional)
    └── Status bar             — who's thinking, idle, streaming
```

---

## REQ-1: 3-Agent Conversation

### How It Works

1. **Daemon boots.** Spawns 3 CLI subprocesses, one per agent. Holds them alive for the daemon's lifetime. Never cold starts.

2. **Human types.** Input goes to the daemon, not directly to a CLI. The daemon decides routing:
   - Direct message: `@claude migrate auth to supabase` → routes to Claude only
   - Open question: `how should we handle auth?` → routes to all 3 in parallel
   - Debate trigger: `/debate should we use Redis or Postgres for caching?` → structured Toulmin workflow

3. **Agents respond.** Each agent's stdout is parsed by jsoniter (zero-allocation streaming), published to NATS `agents.stream`, and rendered in its BubbleTea pane in real-time.

4. **Agents see each other.** When Claude publishes a response, Gemini and Codex receive it via NATS subscription. On their next turn, their prompt includes the other agents' recent messages. They can challenge, agree, or build on each other's ideas.

5. **Human interrupts.** ESC key sends AbortError to the active agent's CLI. Human input always takes priority. Three modes: `interrupt` (stop now), `append` (finish sentence), `queue` (process after current turn).

### Agent Connector Design

```go
type AgentConnector struct {
    Name        string           // "claude", "gemini", "codex"
    Process     *exec.Cmd        // persistent subprocess
    Stdin       io.WriteCloser   // dedicated writer goroutine reads from channel
    Stdout      io.ReadCloser    // dedicated reader goroutine publishes to NATS
    WriteCh     chan []byte       // serialized writes to stdin (no interleaving)
    Health      HealthStatus     // liveness + readiness + progress
    SessionID   string           // for --resume (Claude) or equivalent
}
```

Each connector:
- Spawns with `SysProcAttr{Setpgid: true, Pdeathsig: syscall.SIGTERM}`
- Dedicated reader goroutine: stdout → jsoniter parse → NATS publish
- Dedicated writer goroutine: NATS subscribe → channel → stdin write + `\n`
- Buffered channel between pipe and NATS (backpressure without blocking subprocess)
- Health check: liveness (PID exists), readiness (output within timeout), progress (expected events within SLA)
- On death: auto-restart with jitter/backoff, replay last N messages from JetStream

### NATS Topic Structure

```
agents.claude.output    — Claude's streaming token output
agents.gemini.output    — Gemini's streaming token output
agents.codex.output     — Codex's streaming token output
agents.human.input      — Human's typed messages
agents.broadcast        — Messages all agents should see

debate.proposals        — Toulmin claims
debate.challenges       — Toulmin rebuttals
debate.votes            — Agent votes on proposals

tasks.created           — New task assignments
tasks.progress          — Status updates
tasks.completed         — Finished tasks with outcomes
```

### TUI Layout

```
┌──────────────────┬──────────────────┐
│ Claude (Opus)    │ Gemini (Pro 2M)  │
│ [streaming...]   │ [idle]           │
│                  │                  │
├──────────────────┼──────────────────┤
│ Codex (GPT-5.2) │ Event Log        │
│ [thinking...]    │ debate.proposals │
│                  │ agents.claude... │
├──────────────────┴──────────────────┤
│ > you: how should we handle auth?   │
│ [interrupt] [append] [queue]        │
└─────────────────────────────────────┘
```

---

## REQ-2: Working Stenographer

### The Problem With Current Stenographer
It depends on Ollama + qwen2.5:7b for summarization. LLMs hallucinate. The nudge-reaper's notes system proved this catastrophically — it generated fake session notes about JWT middleware that never existed.

### The Fix: Mechanical Extraction, Not Summarization

Stenographer v2 does NOT use an LLM to generate notes. It mechanically extracts facts:

1. **From NATS streams:** Every agent message, tool call, file edit, and debate turn is already in JetStream. Stenographer subscribes to relevant topics and builds a structured log.

2. **From git:** `git diff` and `git log` provide ground truth about what code actually changed. Stenographer diffs the repo at session start vs. session end.

3. **From tool results:** Every tool invocation (file read, bash command, web search) and its result is logged in `tools.stream`. Stenographer includes these as verifiable facts.

4. **Structured output, not narrative:**
```json
{
  "session_id": "abc-123",
  "duration_minutes": 47,
  "agents_involved": ["claude", "gemini"],
  "files_modified": ["src/auth.go", "src/auth_test.go"],
  "git_commits": ["a1b2c3d Fix auth middleware"],
  "tool_calls": 23,
  "debates": [{
    "topic": "Redis vs Postgres for caching",
    "winner": "Postgres",
    "vote": "2-1",
    "evidence": ["debate.proposals:msg-47", "debate.challenges:msg-52"]
  }],
  "key_decisions": [
    "Use Postgres JSONB instead of Redis (Gemini's rebuttal: fewer infrastructure deps)"
  ],
  "unresolved": [
    "Rate limiting strategy not decided"
  ],
  "resume_command": "triumvirate-agentd --resume abc-123"
}
```

5. **Verification against reality:** Every claim in the session notes must be traceable to a NATS message ID or git commit. "47 tests pass" → show the tool result that ran the tests. If it can't be traced, it's not in the notes.

### How It Integrates

Stenographer is a NATS consumer inside `triumvirate-agentd`. Not a separate process. Not an LLM. It subscribes to all streams, maintains a running session summary, and writes the structured log on session end (or periodically for long sessions).

---

## REQ-3: Reliable Memory

### The Problem With Current Memory
- Claude has `MEMORY.md` (flat files in `~/.claude/`)
- Gemini has CLI state (opaque, session-scoped)
- Codex has threads (opaque, not queryable)
- None of them share. When Claude learns something, Gemini doesn't know.
- Silent failures: a memory write can fail and nobody notices for weeks.

### The Fix: Shared Memory Fabric

One memory store. All three agents read and write to it. The daemon ensures consistency.

```
Memory Layer:
├── SQLite WAL database (persistent, crash-safe)
│   ├── memories table (key, value, type, agent, timestamp, verified)
│   ├── sessions table (session_id, start, end, agents, summary_json)
│   └── decisions table (decision_id, debate_id, outcome, evidence)
│
├── NATS KV bucket "memory" (fast access, ephemeral)
│   └── Hot cache of recently accessed memories
│
└── Sync Engine
    ├── On agent session start: inject relevant memories into agent's system prompt
    ├── On memory write: validate → SQLite → NATS KV → confirm to writing agent
    └── On memory read failure: LOUD error in TUI, not silent skip
```

### Memory Write Protocol

```
Agent writes memory → daemon receives via NATS
  → Validate: is this a real fact? (check against tool results, git, debate outcomes)
  → Deduplicate: does this memory already exist?
  → Persist: SQLite WAL write (crash-safe, fsync'd)
  → Cache: NATS KV update
  → Confirm: publish confirmation to writing agent
  → If ANY step fails: error in TUI status bar + NATS error topic
```

### Memory Types (from existing system, preserved)
- **user** — who the user is, preferences, expertise
- **feedback** — corrections, what to do/not do
- **project** — ongoing work, goals, deadlines
- **reference** — where to find things in external systems

### Agent Memory Injection

On every agent turn, the daemon prepends relevant memories to the prompt:
```
[SHARED MEMORY — verified as of 2026-04-04T10:30:00]
- User is a senior enterprise AE who builds through AI-assisted development
- Prefer Agent Teams over sub-agents for parallel work
- nudge-reaper is DISABLED — do not re-enable without fixing detect.sh
- Triumvirate v2 research at /Users/mikeboscia/projects/triumvirate/research/
[END SHARED MEMORY]
```

This replaces per-agent MEMORY.md files. One source of truth. All agents see the same thing.

---

## REQ-4: It All Works Together

### Single Binary Install

```bash
# Install
go install github.com/michaeljboscia/triumvirate-agentd@latest

# Or build from source
git clone https://github.com/michaeljboscia/triumvirate-agentd
cd triumvirate-agentd
go build -o triumvirate-agentd .

# Run
./triumvirate-agentd
```

One binary. Embeds NATS server, OPA engine, SQLite, BubbleTea TUI. No Docker. No npm. No Python. No node_modules.

### Startup Sequence

```
triumvirate-agentd boot:
  1. Load config from ~/.triumvirate/config.toml
  2. Start embedded NATS + JetStream (in-process, no network)
  3. Start embedded OPA with Rego policies
  4. Open SQLite WAL database
  5. Initialize OpenTelemetry exporter
  6. Spawn Claude CLI subprocess → health check → mark ready
  7. Spawn Gemini CLI subprocess → health check → mark ready
  8. Spawn Codex CLI subprocess → health check → mark ready
  9. Start BubbleTea TUI
  10. Start Stenographer consumer
  11. Ready. All 4 participants online.

  If ANY step fails: display error in terminal, don't silently continue.
  If an agent fails health check: mark degraded, show in status bar, retry with backoff.
```

### Failure Modes (visible, never silent)

| What Fails | What Happens | User Sees |
|---|---|---|
| Agent subprocess dies | Auto-restart with backoff, replay from JetStream | Status bar: "Claude: restarting..." |
| Memory write fails | Retry 3x, then ERROR in TUI | Red banner: "Memory write failed: [reason]" |
| NATS internal error | Log + alert, attempt recovery | Status bar: "FABRIC: degraded" |
| Daemon crash | On restart: load SQLite snapshot, replay JetStream tail | Resume prompt: "Session abc-123 recovered" |
| Agent unresponsive | Health check fails after timeout | Status bar: "Gemini: unresponsive (45s)" |

### REQ-6: Visual Dashboard

A web UI that shows people what the system does. Served from the same Go binary via `//go:embed` + `net/http`. Opens at `http://localhost:8080` when the daemon starts.

Shows:
- Agent panes with real-time streaming output (via WebSocket/SSE from NATS)
- Temporal workflow state (leverages Temporal's built-in Web UI on port 8233)
- Shared memory viewer (query SQLite, display current state)
- Session history (stenographer logs, searchable)
- Event stream (NATS messages flowing between agents)
- Agent health (liveness, readiness, progress per agent)

Temporal's Web UI provides ~50% of this for free: workflow visualization, activity status, retries, failures. We build a custom frontend for the chat view and memory viewer.

---

## Architecture Revisions (Post-Gemini Review)

### Pivot 1: Observe, Don't Intercept
CLIs execute their own tools internally. We can't intercept. Instead:
- Spawn CLIs inside PTYs (`creack/pty`) — they think they're in a real terminal
- Scrape PTY output with regex for tool executions, proposals, errors
- Watch filesystem with `fsnotify` for file changes
- Stenographer works by observing the exhaust pipe, not controlling the engine

### Pivot 2: Embedded Temporal (Not External Cluster)
Temporal server embeds as a Go library via `go.temporal.io/server/temporalite`. Uses SQLite for persistence. Web UI included. Single binary.

Gives us for free:
- Crash recovery with deterministic replay
- Automatic retries with configurable policies
- Saga compensation for multi-step operations
- Human-in-the-loop signals (pause workflow, wait for approval)
- Web UI showing every workflow, step, retry, failure

Risk mitigations:
- Separate SQLite files: `temporal.db` + `memory.db` (avoid write contention)
- Go workspaces for dependency isolation (NATS + Temporal + OPA gRPC conflicts)
- PTY I/O happens in Temporal Activities, never Workflows (determinism requirement)

### Pivot 3: Prompt Prefix Stuffing for Context
Every message to an agent gets a context header prepended to stdin:
```
[CONTEXT — do not reply to this section]
- Memory: We decided SQLite, not Postgres (debate 2026-04-04, 2-1)
- Claude proposed JWT auth — Gemini flagged revocation gap
[END CONTEXT]

How should we handle token revocation?
```
Works with ANY CLI. Universal. Burns some tokens but guarantees shared state.

### Pivot 4: Markdown Keywords for Agent Protocol
Agents output natural Markdown headers instead of JSON schemas:
- `# PROPOSAL:` — Claude proposing architecture
- `# CHALLENGE:` — Gemini finding a problem  
- `# APPROVED:` — Gemini passing review
- `# IMPLEMENTATION:` — Codex writing code
- `# BUG:` — Codex finding an issue

Daemon parses headers to route messages and trigger state transitions.
Internally translates to structured NATS events. Best of both worlds.

---

## What's NOT in v1

- Multi-machine deployment (local only)
- API connectors (CLI only, API is future upgrade)
- CRDTs for parallel editing (file-level locking for now)
- Full speculative execution (pragmatic pre-fetching only)
- Tree-sitter AST operations (text editing for now)
- A2A protocol compliance (internal NATS messaging for now)
- Toulmin JSON output from agents (Markdown keywords instead, structured internally)

---

## Implementation Plan

### POC (Day 1-2): Prove The Chimera Lives

Before building anything else, prove the foundational assumptions work.

**POC 1: The Embedded Chimera**
Single Go binary that boots:
- Embedded Temporal server (SQLite persistence) on localhost:7233
- Embedded NATS + JetStream on localhost:4222
- HTTP server with `//go:embed` serving a "hello world" page on localhost:8080

Success: binary runs, curl the HTTP server, connect to NATS, hit Temporal Web UI on port 8233.

**POC 2: The PTY Agent**
- Write Temporal Workflow: `RunAgentTurn`
- Write Temporal Activity: `ExecuteClaudePTY`
- Activity spawns `claude` in a PTY (`creack/pty`), sends "Say hello", reads response, returns to Workflow
- Stream PTY output to NATS topic and to the embedded web page via WebSocket

Success: Claude responds in the browser. Temporal UI shows the workflow completed.

**POC 3: The Triplex**
- Spawn all 3 CLIs in PTYs
- Broadcast human input from web UI to all 3
- Stream all 3 outputs to browser simultaneously in 3 panes
- Regex scraper catches `# PROPOSAL:` and tool execution patterns

Success: type a question in the browser, see Claude, Gemini, and Codex all respond in real-time.

If POC 3 works, the architecture is proven. Everything after is engineering.

### Week 1: The Foundation
- Formalize Agent Connector with health checks, auto-restart, JetStream replay
- Context injection (prompt prefix stuffing with shared memory)
- Markdown keyword parser + NATS event publishing
- Supervisor state machine (route messages based on keyword headers)
- SQLite WAL for memory persistence with loud-failure protocol
- **Prove:** 4-way conversation where agents reference each other's output

### Week 2: The Workflows
- `ConversationWorkflow` — multi-turn 4-way collaboration
- `DebateWorkflow` — proposal → challenge → vote → decide
- `TaskWorkflow` — plan → implement → verify with human approval gate
- OPA embedded with basic policies (destructive ops require approval)
- Stenographer consumer writing structured session logs from NATS streams
- **Prove:** structured debate between 3 agents with human approval gate

### Week 3: The Dashboard
- Custom web frontend (agent chat panes, memory viewer, session history)
- WebSocket streaming from NATS to browser
- Temporal UI integration (workflow state, retries, failures)
- `fsnotify` file watcher for ground-truth file change tracking
- **Prove:** show someone the dashboard and they understand what the system does in 30 seconds

### Week 4: The Armor
- Chaos testing (Toxiproxy, failpoint for stream truncation, process death)
- Crash recovery E2E: kill daemon mid-workflow, restart, verify resume
- Process management (Setpgid + Pdeathsig + graceful shutdown)
- E2E tests against real CLIs (reality-check matrix applied)
- OpenTelemetry tracing with GenAI semantic conventions
- **Prove:** kill -9 the daemon, restart, conversation resumes at exact point

---

## Technology Stack (Locked)

| Component | Technology | Why |
|---|---|---|
| Language | Rust | Zero-cost abstractions, no GC, compile-time safety, deterministic performance |
| Messaging | Embedded NATS + JetStream | In-process pub/sub, persistence, replay, zero network overhead |
| Orchestration | Temporal.io | Durable execution, retry, compensation, human-in-the-loop |
| Governance | Cedar (Rust-native) | AWS policy engine, embeds directly, millisecond evaluation |
| Persistence | SQLite WAL | Crash-safe, embedded, zero-config, mature Go bindings |
| Observability | OpenTelemetry | Distributed tracing, GenAI semantic conventions |
| CLI Interface | Colored scrolling conversation | Single-pane terminal with agent-labeled output |
| Web Dashboard | Svelte + Tailwind (rust-embed) | SOTA frontend, static build embedded in binary |
| JSON parsing | serde_json / simd-json | Rust-native JSON streaming and parsing |
| Debate schema | Toulmin model | claim/data/warrant/rebuttal — structured, not "give your opinion" |
| CLI connectors | Persistent subprocesses | Subscription accounts ($0), full agent capabilities |

---

## Goat Rodeo Round 1 Decisions

### GR1-D1: Web-Only UI (REQ-1, REQ-4, REQ-6)
Daemon runs headless. Terminal shows health logs + minimal CLI (`triumvirate status`). Web dashboard at `localhost:8080` is the exclusive conversation interface. TUI stubbed as `triumvirate ask` for later. One UI to build, one to maintain, one to show people.

### GR1-D2: Delta Context Injection (REQ-1, REQ-3)
Persistent PTYs hold their own conversation history. On each turn, daemon only injects what OTHER agents said since this agent's last turn. Full state injection only on agent restart (PTY crash recovery). Agents already know their own context — we only bridge the gap between them.

### GR1-D3: Adaptive Lead with Human Override (REQ-1, REQ-5)
Claude synthesizes by default. Topic-based lead rotation: architecture → Claude, research → Gemini, implementation → Codex. On diametric disagreements, human decides. Lead role is a soft default the conversation can override, not a hardcoded hierarchy.

### GR1-D4: Syntax-Gated Memory + Dual-Agent Approval (REQ-2, REQ-3)
Memory writes ONLY via `# DECISION:` keyword. Second agent must `# VALIDATE: APPROVE` to persist. No LLM interpretation. No conversation parsing. This also replaces stenographer decision capture — decisions are syntax-gated facts, not summarized text. Stenographer captures files changed (fsnotify/git), tool calls (PTY scraping), decisions (keyword protocol), and metadata (duration, agents, NATS counts).

### GR1-D5: Passive Monitoring + Background Tasks (REQ-5)
Idle agents passively monitor conversation stream and self-activate when relevant. Daemon also assigns background tasks: test generation (Codex), doc pre-fetch (Gemini), code review (all). Background outputs go to Intelligence Feed pane in dashboard, not main conversation.

### GR1-D6: Unified Dashboard, Temporal as DevTools (REQ-4, REQ-6)
Custom dashboard at :8080 is the one interface. Temporal UI at :8233 accessible via "Developer Tools" link from dashboard. Normal use = one browser tab. Deep workflow debugging = click through.

### GR1-D7: Mock CLIs for Dev, Real CLI E2E as Gate (ALL)
Deterministic mock CLIs (`mock-claude`, `mock-gemini`, `mock-codex`) for fast development iteration. Real CLI E2E tests as actual acceptance gate before anything ships. Mocks verify plumbing. Real CLIs verify reality. Reality-check matrix applies.

---

## Success Criteria

The spec is done when:

1. I can open a browser, go to `localhost:8080`, and have a conversation with all three agents simultaneously
2. When I close the session, I get accurate notes about what actually happened — verified against git and tool results, decisions syntax-gated through `# DECISION:` + `# VALIDATE:`
3. When I start a new session, all three agents know what we decided in the last one via shared SQLite memory
4. When something breaks, I see it immediately in the dashboard — not weeks later when my work gets destroyed
5. All three agents are working simultaneously — the active one responding, the idle ones monitoring and doing background work
6. I can show someone the dashboard and they understand what it does in 30 seconds

That's it. Build this. Everything else is a feature request for v2.
