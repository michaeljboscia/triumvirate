# Triumvirate v2 — Specification

**Date:** 2026-04-05
**Author:** Mike Boscia + Claude + Gemini + Codex
**Status:** Final (Goat Rodeo R1-R6 complete)
**Origin:** nudge-reaper catastrophe → crystallize → first-principles rebuild → 6-round Goat Rodeo

---

## What This Is

A Rust binary (`triumvirate-agentd`) that runs on your machine and makes Claude, Gemini, and Codex work together as a fleet. Not a framework. Not a platform. Your everyday driver.

The thing that doesn't exist anywhere else: **cross-model multi-agent fleet coordination.** Claude Agent Teams is Claude-only. OpenAI Swarm is OpenAI-only. Google ADK is Gemini-only. This runs all three simultaneously — any number of each — coordinated by one daemon.

## The Seven Requirements

Everything in this spec serves one of these. If it doesn't serve one, it's not in v1.

### REQ-1: N-Agent Conversation
Claude, Gemini, and Codex in the same browser, all talking, all live. The human is the fourth participant. Not note-passing through files — real-time streaming collaboration where all participants see each other's output. Each agent connected via its native structured JSON protocol.

### REQ-2: Working Stenographer
Session notes that actually capture what happened, every time. No hallucinated summaries. No silent failures. Mechanical extraction of facts from the conversation, verified against reality.

### REQ-3: Reliable Memory
Shared across all agents and the human via SQLite WAL. Never silently fails. Never loses data. Never diverges between agents. When Claude learns something, Gemini and Codex know it next time they're asked. No hot cache — SQLite handles reads directly.

### REQ-4: It All Works Together
One system. One binary. One install. Not a pile of scripts hoping for the best. If any component fails, the system tells you — it doesn't silently degrade. No external dependencies (no NATS server, no Temporal cluster). Everything is in-process.

### REQ-5: Use All Three, All The Time
Three subscriptions. Three agents. All working simultaneously, not taking turns. When Claude is designing, Gemini is researching, and Codex is coding — in parallel. Idle agents receive lightweight mechanical digests (not LLM-generated summaries) so they can self-activate on high-value moments. Quota-gated: per-agent budget meters with auto-fallback to explicit routing at 80% threshold.

### REQ-6: Visual Dashboard
A web UI served from the binary via `rust-embed`. Shows agents working, tasks progressing, memory state, and fleet health. Two views: tasks (executive) and agents (debug). The interface you show someone so they understand what the system does in 30 seconds.

### REQ-7: Dynamic Multi-Agent Fleet
The daemon can spawn N instances of any agent type on demand. "Give me 3 Claudes on auth, 2 Codexes on API, 1 Gemini researching" is a valid command. Fleet composition is defined per-task, not per-daemon. The daemon manages task assignment, context distribution via git worktrees, collision prevention, quota tracking, and result aggregation. Cross-model fleet coordination — the killer feature.

---

## Architecture

```
triumvirate-agentd (single Rust binary)
│
├── Agent Pool (persistent CLI subprocesses, N per type)
│   ├── Claude  (--input-format stream-json --output-format stream-json --session-id)
│   ├── Gemini  (--acp, JSON-RPC over stdio)
│   └── Codex   (mcp-server, JSON-RPC over stdio)
│   Each agent type supports CLI or API backend (config switch)
│
├── Message Fabric (Tokio broadcast/mpsc/watch channels)
│   ├── agents.{type}.{id}.output  — agent streaming output
│   ├── tasks.stream               — task lifecycle events
│   ├── fleet.stream               — fleet coordination events
│   └── memory.stream              — memory writes/reads
│   Topic enum maps to NATS subjects (future swap path)
│
├── Workflow Engine (purpose-built, SQLite-backed)
│   ├── ConversationWorkflow  — N-way live collaboration
│   ├── DebateWorkflow        — structured Toulmin debate with vote
│   ├── TaskWorkflow          — plan → execute → verify
│   └── FleetWorkflow         — fan-out N agents, worktrees, sequential merge
│   Informed by Temporal source (Apache 2.0). Event-sourced state machine.
│   Crash recovery via write-ahead log in SQLite WAL.
│
├── Fleet Coordinator
│   ├── Git worktrees          — one per fleet member (isolation)
│   ├── Contract definitions   — interfaces defined before fan-out (Wave 0)
│   ├── Shared task list       — SQLite, dependency tracking, claim/complete
│   ├── Sequential merge       — one branch at a time, full context
│   └── Peer messaging         — fleet members communicate via fabric
│   Informed by Ruflo, Clash, Claude Agent Teams patterns.
│
├── Memory (shared context fabric)
│   ├── SQLite WAL             — persistent store (memories, sessions, decisions)
│   └── Daemon extraction      — daemon proposes memory writes from structured
│                                JSON, human or second agent confirms
│
├── Governance (Cedar, Rust-native)
│   ├── Cedar policies         — what each agent can/can't do
│   └── Human approval gates   — destructive ops require confirmation
│
├── Observability (OpenTelemetry)
│   ├── Per-agent spans        — latency, tokens, cost per turn
│   └── Trace correlation      — session_id across all agents
│
├── Stenographer (fabric consumer)
│   ├── Mechanical extraction  — structured JSON events, NOT LLM summaries
│   ├── Git diffs              — files changed between session start/end
│   ├── Tool results           — every invocation and outcome
│   └── Decisions              — syntax-gated via daemon extraction
│
└── Web Dashboard (axum + Svelte + Tailwind, rust-embed)
    ├── Tasks view             — grouped by work item, agent assignments
    ├── Agents view            — dynamic grid, one pane per running agent
    ├── Quota meters           — per-agent budget bars with thresholds
    ├── Routing log            — every message, who got what, token cost
    ├── Memory viewer          — query SQLite, display current state
    └── Workflow panel          — state machine visualization (replaces Temporal UI)
```

---

## REQ-1: N-Agent Conversation

### How It Works

1. **Daemon boots.** Spawns persistent CLI subprocesses per config. Holds them alive for the daemon's lifetime. Never cold starts.

2. **Human types.** Input goes to the daemon via the web dashboard, not directly to a CLI. The daemon decides routing:
   - Direct message: `@claude migrate auth to supabase` → routes to Claude only
   - Open question: `how should we handle auth?` → routes to lead agent per GR1-D3
   - Debate trigger: `/debate should we use Redis or Postgres for caching?` → structured Toulmin workflow

3. **Agents respond.** Each agent's stdout is parsed from its native JSON protocol (stream-json, ACP, or MCP), published to the fabric, and rendered in the dashboard in real-time.

4. **Agents see each other.** When Claude publishes a response, other agents receive a mechanical digest via the fabric. On their next turn, their prompt includes what other agents said. They can challenge, agree, or build on each other's ideas.

5. **Human interrupts.** ESC sends cancellation to the active agent. Human input always takes priority.

### Agent Connector Design

```rust
#[async_trait]
pub trait AgentConnector: Send + Sync {
    fn agent_id(&self) -> AgentId;
    async fn spawn(&mut self, bus: Arc<MessageBus>) -> Result<()>;
    async fn send(&self, message: &str) -> Result<()>;
    async fn shutdown(&mut self) -> Result<()>;
    fn health(&self) -> HealthStatus;
    fn health_watch(&self) -> watch::Receiver<HealthStatus>;
}
```

Each connector:
- Spawns with `tokio::process::Command`, stdin/stdout piped (no PTY)
- Dedicated reader task: stdout → JSON parse → fabric publish
- Dedicated writer task: fabric subscribe → JSON serialize → stdin write
- Health check: liveness (PID exists), readiness (output within timeout)
- On death: auto-restart with jitter/backoff

### Per-Agent Protocols

| Agent | Command | Protocol | Persistent? | Multi-turn |
|-------|---------|----------|-------------|------------|
| Claude | `claude --input-format stream-json --output-format stream-json --session-id <id>` | Bidirectional JSONL | Yes | Yes (session-id) |
| Gemini | `gemini --acp` | JSON-RPC over stdio | Yes | Yes (session state) |
| Codex | `codex mcp-server` | JSON-RPC/MCP over stdio | Yes | Yes (codex-reply tool) |

All three are persistent subprocesses with structured JSON. System prompt sent once. Subsequent messages via stdin. Truly warm.

### Provider Abstraction

The `AgentConnector` trait supports both CLI subprocess and API backends:

```toml
# ~/.triumvirate/config.toml
[agents.claude]
backend = "cli"       # or "api"
instances = 1          # default; REQ-7 overrides per-task
# api_key = "sk-..."  # only if backend = "api"
```

If a provider restricts CLI usage for third-party frameworks, flip to API. No code change.

---

## REQ-2: Working Stenographer

### The Fix: Mechanical Extraction, Not Summarization

Stenographer v2 does NOT use an LLM to generate notes. It mechanically extracts facts:

1. **From the fabric:** Every agent message, tool call, and decision is already structured JSON in the fabric. Stenographer subscribes and builds a structured log.

2. **From git:** `git diff` and `git log` provide ground truth about what code actually changed. Stenographer diffs the repo at session start vs. session end.

3. **From tool results:** Every tool invocation and its result is logged. Stenographer includes these as verifiable facts.

4. **Structured output, not narrative:**
```json
{
  "session_id": "abc-123",
  "duration_minutes": 47,
  "agents_involved": ["claude-1", "claude-2", "gemini-1"],
  "fleet_composition": {"claude": 2, "gemini": 1, "codex": 0},
  "files_modified": ["src/auth.rs", "src/auth_test.rs"],
  "git_commits": ["a1b2c3d Fix auth middleware"],
  "tool_calls": 23,
  "debates": [{
    "topic": "Redis vs Postgres for caching",
    "winner": "Postgres",
    "vote": "2-1",
    "evidence": ["fabric:msg-47", "fabric:msg-52"]
  }],
  "key_decisions": [
    "Use Postgres JSONB instead of Redis (Gemini's rebuttal: fewer infrastructure deps)"
  ],
  "unresolved": ["Rate limiting strategy not decided"],
  "resume_command": "triumvirate-agentd --resume abc-123"
}
```

5. **Verification against reality:** Every claim in the session notes must be traceable to a fabric message ID or git commit. If it can't be traced, it's not in the notes.

### How It Integrates

Stenographer is a Tokio task inside `triumvirate-agentd`. Not a separate process. Not an LLM. It subscribes to the fabric firehose, maintains a running session summary, and writes the structured log on session end.

---

## REQ-3: Reliable Memory

### Shared Memory Fabric

One memory store. All agents read and write to it. The daemon ensures consistency.

```
Memory Layer:
├── SQLite WAL database (persistent, crash-safe)
│   ├── memories table (key, value, type, agent, timestamp, verified)
│   ├── sessions table (session_id, start, end, agents, summary_json)
│   └── decisions table (decision_id, outcome, evidence)
│
└── Daemon Extraction
    ├── On agent output: daemon parses structured JSON for decision-like content
    ├── On detection: proposes memory write to dashboard
    ├── On confirmation: human or second agent approves → SQLite write
    └── On failure: LOUD error in dashboard, not silent skip
```

No NATS KV cache. No in-memory HashMap. SQLite WAL mode supports concurrent reads at microsecond latency for this data volume (dozens to hundreds of entries). Add cache only if profiling demands it.

### Memory Types (preserved from existing system)
- **user** — who the user is, preferences, expertise
- **feedback** — corrections, what to do/not do
- **project** — ongoing work, goals, deadlines
- **reference** — where to find things in external systems

### Agent Memory Injection

On every agent turn, the daemon prepends relevant memories to the prompt:
```
[SHARED MEMORY — verified as of 2026-04-05T10:30:00]
- User is a senior enterprise AE who builds through AI-assisted development
- Prefer Agent Teams over sub-agents for parallel work
- nudge-reaper is DISABLED — do not re-enable without fixing detect.sh
[END SHARED MEMORY]
```

One source of truth. All agents see the same thing.

---

## REQ-4: It All Works Together

### Single Binary Install

```bash
# Build from source
git clone https://github.com/michaeljboscia/triumvirate-agentd
cd triumvirate-agentd/daemon
cargo build --release

# Run
./target/release/triumvirate-agentd
```

One binary. Embeds workflow engine, Cedar policy engine, SQLite, axum web server, Svelte dashboard. No Docker. No npm. No external processes.

### Startup Sequence

```
triumvirate-agentd boot:
  1. Initialize tracing (so all subsequent steps can log)
  2. Load config from ~/.triumvirate/config.toml
  3. Start message fabric (Tokio broadcast channels)
  4. Open SQLite WAL databases (memory.db, workflow.db)
  5. Load Cedar policies
  6. Initialize OpenTelemetry exporter
  7. Spawn agent subprocesses per config → health check → mark ready
  8. Start health monitor
  9. Start Stenographer consumer
  10. Start web dashboard on :8080
  11. Ready. All participants online.

  If ANY step fails: display error in terminal, don't silently continue.
  If an agent fails health check: mark degraded, show in dashboard, retry with backoff.
```

### Failure Modes (visible, never silent)

| What Fails | What Happens | User Sees |
|---|---|---|
| Agent subprocess dies | Auto-restart with backoff | Dashboard: "Claude-1: restarting..." |
| Memory write fails | Retry 3x, then ERROR | Red banner: "Memory write failed: [reason]" |
| Fabric channel closed | Log + alert, attempt recovery | Dashboard: "FABRIC: degraded" |
| Daemon crash | On restart: load SQLite snapshot, resume workflows | Resume prompt: "Session abc-123 recovered" |
| Agent unresponsive | Health check fails after timeout | Dashboard: "Gemini-1: unresponsive (45s)" |
| Quota exhausted | Auto-fallback from digest to explicit routing | Dashboard: "Claude: 92% quota, digest OFF" |

---

## REQ-5: Use All Three, All The Time

### Quota-Gated Routing (not passive monitoring)

The original spec proposed passive monitoring where all agents read every message. This burns subscription quota (5-hour rolling windows, daily caps) for messages agents ignore.

Instead:

1. **Active agent** does the work — responds to the human's question.
2. **Idle agents on the same task** receive a **mechanical digest** — template-based extraction from structured JSON: "Claude-1 proposed JWT auth. [3 tool calls, 2 files modified]. Gemini-1, anything to add?" No LLM generates this. The daemon formats it from fabric events.
3. **Idle agents on OTHER tasks** receive nothing — digests are scoped to task, not fleet.
4. **Self-activation** — if an idle agent's digest triggers a response (e.g., Gemini spots a security gap), the daemon routes it to the conversation.
5. **Auto-fallback** — when an agent's quota meter hits 80%, digests stop. Only explicit @-mentions reach that agent.

### Background Tasks

The daemon can assign explicit background work to idle agents:
- Test generation (Codex)
- Documentation pre-fetch (Gemini)
- Code review (any agent)

Background outputs go to the Intelligence Feed in the dashboard, not the main conversation.

---

## REQ-6: Visual Dashboard

Served from the same Rust binary via `rust-embed` + `axum`. Opens at `http://localhost:8080`.

### Two Views

**Tasks View (default, executive):**
- Grouped by work item ("Auth Module", "API Design", "Database Schema")
- Shows which agents are assigned to each task
- Progress indicators, completion status
- Click into a task to see agent output

**Agents View (debug):**
- Dynamic grid — auto-layouts based on fleet size
- One pane per running agent with streaming output
- Health indicators per agent
- Quota meters

### Dashboard Panels
- **Routing log** — every message, target agent, type, token cost
- **Memory viewer** — query SQLite, display current state
- **Workflow panel** — state machine visualization (purpose-built, not Temporal)
- **Session history** — stenographer logs, searchable
- **Quota dashboard** — per-agent and per-model-type spend

---

## REQ-7: Dynamic Multi-Agent Fleet

### Fleet Spawning

```
User: "Build the auth system"
Daemon: Spawns fleet:
  - Claude-1: architect (design interfaces, define contracts)
  - Claude-2: review (challenge Claude-1's design, find gaps)
  - Gemini-1: research (JWT best practices, OWASP guidelines)
  - Codex-1: implement auth module
  - Codex-2: implement auth tests
```

Fleet composition is defined per-task at spawn time. The daemon provisions:
- One git worktree per fleet member (isolation)
- One persistent subprocess per fleet member
- Task assignments in the shared task list (SQLite)
- Contract definitions from Wave 0 before implementation begins

### Coordination Model

Informed by Claude Agent Teams, Ruflo, Clash, and swarms-rs patterns:

1. **Contracts first (Wave 0)** — before the fleet fans out, one agent (or the human) defines interfaces: function signatures, types, API shapes. Fleet members implement AGAINST those contracts. They can't conflict because boundaries are pre-defined.

2. **Git worktrees** — each fleet member works in an isolated worktree. Same `.git`, separate working directories. Prevents direct overwrites.

3. **Shared task list** — daemon decomposes work into subtasks with dependencies. Fleet members claim available tasks. Completed tasks unblock dependents. Tracked in SQLite.

4. **Sequential merge** — when fleet members finish, branches merge one at a time. Each merge gets full context of previous merges. Not parallel.

5. **Peer messaging** — fleet members communicate through the fabric. Codex-1 can message Codex-2: "I changed the User struct, heads up." No routing through a lead required.

6. **Human gate** — simple conflicts (two agents add to same list) = daemon resolves. Architectural conflicts = human decides via dashboard.

### Concurrency Limits

| Provider | Max Concurrent | Quota Model | Practical Fleet Size |
|----------|---------------|-------------|---------------------|
| Claude (Max 20x) | Unlimited instances | ~900 messages / 5-hour window shared | 3-5 on heavy tasks |
| Codex | ~8 concurrent API requests | RPM/TPM tiers | 2-3 |
| Gemini (Ultra) | Unlimited instances | 2,000 requests/day shared | 2-4 |

The daemon tracks quota per-instance and per-model-type. Fleet size is limited by quota, not architecture.

---

## Architecture Decisions

### Goat Rodeo Round 1 (Pre-Rust Pivot)

**GR1-D1: Web-Only UI (REQ-1, REQ-4, REQ-6)**
Daemon runs headless. Web dashboard at `localhost:8080` is the exclusive conversation interface.

**GR1-D3: Adaptive Lead with Human Override (REQ-1, REQ-5)**
Claude synthesizes by default. Topic-based lead rotation: architecture → Claude, research → Gemini, implementation → Codex. On diametric disagreements, human decides.

**GR1-D7: Mock CLIs for Dev, Real CLI E2E as Gate (ALL)**
Deterministic mock CLIs for fast development iteration. Real CLI E2E tests as acceptance gate before anything ships.

### Goat Rodeo Round 2-6 (Rust Pivot + REQ-7)

**GR2-D1: Purpose-Built Workflow Engine (REQ-4)**
Rust, SQLite-backed, informed by Temporal's open source Go code (Apache 2.0). No sidecar, no Go dependency. True single binary.

**GR2-D2: Per-Agent Native Adapters (REQ-1)**
Drop PTY entirely. Claude: stream-json. Gemini: ACP pipe-mode. Codex: mcp-server. All persistent subprocesses with bidirectional JSON stdio.

**GR2-D3: Tokio Channels, NATS If Needed (REQ-4)**
In-process broadcast/mpsc/watch. Topic enum makes NATS a future swap. No external dependency.

**GR2-D4: Summary Digests with Fallback (REQ-5)**
Idle agents get mechanical digests, not firehose. Auto-fallback at 80% quota. Digests scoped to task, not fleet.

**GR2-D5: Daemon Extracts Decisions (REQ-3)**
No Markdown keywords. JSON is native transport. Daemon proposes memory writes. Human/agent approves.

**GR2-D6: No Hot Cache (REQ-3)**
SQLite WAL handles concurrent reads. Add cache only if profiling demands it.

**GR4-D1: Provider Abstraction (REQ-7)**
AgentConnector trait supports CLI + API backends. Config switch. No code change if policy shifts.

**GR4-D2: Dynamic Dashboard (REQ-6, REQ-7)**
Tasks view + agents view, toggled. Dynamic grid scales with fleet size.

**GR4-D3: Worktrees + Contracts + Task List + Sequential Merge (REQ-7)**
Informed by Claude Agent Teams, Ruflo, Clash patterns. Applied cross-model.

**GR4-D4: Study and Borrow from Prior Art (REQ-7)**
Ruflo (multi-model routing), Clash (worktree conflict detection), swarms-rs (agent lifecycle). Attribute everything in NOTICE.md.

---

## Technology Stack (Locked)

| Component | Technology | Why |
|---|---|---|
| Language | Rust | Zero-cost abstractions, no GC, compile-time safety |
| Async Runtime | Tokio | Industry standard, full ecosystem |
| Message Fabric | Tokio channels (broadcast/mpsc/watch) | In-process, zero overhead, NATS-shaped topics |
| Workflow Engine | Purpose-built (SQLite + event sourcing) | Informed by Temporal source. No sidecar. |
| Governance | Cedar (`cedar-policy` crate) | AWS policy engine, Rust-native, millisecond evaluation |
| Persistence | SQLite WAL (`rusqlite` bundled) | Crash-safe, embedded, zero-config |
| Observability | OpenTelemetry | Distributed tracing, GenAI semantic conventions |
| Web Server | axum + tower-http | Tokio-native, fast, middleware ecosystem |
| Web Dashboard | Svelte + Tailwind (`rust-embed`) | Static build embedded in binary |
| JSON Parsing | serde_json | Rust-native, zero-copy deserialization |
| Process Mgmt | `tokio::process` + `portable-pty` (if needed) | Subprocess lifecycle, signal handling |
| File Watching | `notify` crate | Rust equivalent of fsnotify |
| Debate Schema | Toulmin model | claim/data/warrant/rebuttal — structured |
| CLI Connectors | Persistent subprocesses | Subscription accounts, full agent capabilities |

---

## What's NOT in v1

- Multi-machine deployment (local only)
- API-only connectors as default (CLI is primary, API is fallback)
- CRDTs for parallel editing (worktrees + sequential merge instead)
- Full speculative execution (pragmatic pre-fetching only)
- Tree-sitter AST operations (text editing for now)
- A2A protocol compliance (internal fabric messaging for now)
- Embedded NATS (Tokio channels for now, NATS as future upgrade)
- Embedded Temporal (purpose-built engine instead)

---

## Implementation Plan

### POC (Day 1): Prove The Chimera Lives

**POC 1: The Embedded Chimera** ✅ (completed, boots in 13ms)
Single Rust binary that boots: Tokio runtime, axum HTTP server with `rust-embed` on :8080, SQLite WAL database, message fabric. Scaffold at `daemon/`.

**POC 2: The Live Agent**
- Write the Claude connector: spawn `claude --input-format stream-json --output-format stream-json --session-id <id>`
- Parse JSONL output, publish to fabric
- Stream fabric events to browser via WebSocket
- Success: Claude responds in the browser. Fabric carries the message.

**POC 3: The Triplex**
- Add Gemini (ACP) and Codex (mcp-server) connectors
- All 3 outputs streaming to browser simultaneously
- Daemon-generated mechanical digest to idle agents
- Success: type a question, see all 3 respond in real-time.

**POC 4: The Fleet**
- Spawn 2 Claudes + 1 Codex on the same task
- Git worktree per fleet member
- Shared task list in SQLite
- Sequential merge of results
- Success: fleet produces coordinated output from 3 agents.

### Week 1: Foundation
- Formalize agent pools with health checks, auto-restart
- Context injection (memory prefix + delta from other agents)
- Workflow engine: ConversationWorkflow, basic state machine + SQLite persistence
- Cedar policies for destructive ops (human approval gate)
- Stenographer writing structured session logs from fabric

### Week 2: Fleet
- FleetWorkflow: fan-out, worktrees, task list, sequential merge
- Contract definitions (Wave 0) before fleet fan-out
- Peer messaging through fabric
- Dynamic dashboard: tasks view + agents view
- Quota management: per-agent meters, auto-fallback

### Week 3: Polish
- Full Svelte dashboard build (replace POC HTML)
- WebSocket streaming from fabric to browser
- Workflow panel (state machine visualization)
- `notify` crate for file change tracking
- E2E tests: mock CLIs + real CLI acceptance gate

### Week 4: Armor
- Crash recovery: kill daemon mid-workflow, restart, verify resume
- Process management: graceful shutdown, signal handling, orphan prevention
- OpenTelemetry tracing with GenAI semantic conventions
- Chaos testing: process death, stream truncation
- Reality-check matrix applied to everything

---

## Success Criteria

The spec is done when:

1. I can open a browser, go to `localhost:8080`, and have a conversation with N agents simultaneously
2. When I close the session, I get accurate notes about what actually happened — verified against git and tool results, decisions extracted by the daemon
3. When I start a new session, all agents know what we decided in the last one via shared SQLite memory
4. When something breaks, I see it immediately in the dashboard — not weeks later when my work gets destroyed
5. All agents are working simultaneously — the active ones responding, the idle ones receiving mechanical digests and self-activating when relevant
6. I can show someone the dashboard and they understand what it does in 30 seconds
7. I can say "give me 3 Claudes on auth, 2 Codexes on API, 1 Gemini researching" and the fleet spins up, coordinates via worktrees, and merges results — all visible in the dashboard

That's it. Build this.

---

## Acknowledgments

This spec was informed by the following open-source projects:

- **Temporal** (temporalio/temporal, Apache 2.0) — workflow engine patterns, crash recovery, event sourcing
- **Ruflo** (ruvnet/ruflo, open source) — multi-model agent orchestration, cost-optimized routing
- **Clash** — real-time git worktree conflict detection
- **swarms-rs** (open source) — Rust-native agent lifecycle management
- **Claude Agent Teams** (Anthropic) — worktree isolation, shared task list, peer-to-peer mailbox patterns

Specific code borrowed from these projects will be attributed inline and in NOTICE.md.
