# Goat Rodeo Round 2-6 — Decision Ledger

**Date:** 2026-04-05
**Spec:** /Users/mikeboscia/projects/triumvirate/SPEC.md
**Rounds:** 6 (2 decision rounds from GR1 already existed, 4 new decision rounds, 2 validation rounds)
**New REQ added:** REQ-7 (Dynamic Multi-Agent Fleet)

---

## YOUR CALLS

### 1. Purpose-Built Workflow Engine (GR2-D1) — REQ-4
**Decided:** Build a Rust workflow engine (~1,500 lines) informed by Temporal's open source Go code. SQLite WAL + event sourcing for durability. No Temporal sidecar, no Go dependency. True single binary.
**Alternative was:** Temporal as a managed sidecar process, or an immature Rust workflow engine (Sayiir, Tsumugi).
**Why this won:** You're already building the whole thing. Temporal's source is Apache 2.0 — you can read exactly how they solve crash recovery and adapt the 5% you need. No external dependency, no version coupling, no process orphaning.

### 2. Per-Agent Native Adapters (GR2-D2) — REQ-1, GR1-D2
**Decided:** Drop PTY entirely. Each agent gets its native structured JSON protocol. Claude: `--input-format stream-json` + `--output-format stream-json` + `--session-id`. Gemini: ACP pipe-mode JSON-RPC. Codex: `mcp-server` persistent JSON-RPC. All three are persistent subprocesses with bidirectional stdio.
**Alternative was:** PTY for all agents with regex scraping, or PTY for Claude only.
**Why this won:** PTY breaks Gemini (ANSI noise corrupts JSON-RPC) and is unnecessary for Claude (stream-json exists). All three CLIs already speak structured JSON. This is simpler, faster, and more reliable than what was originally planned.

### 3. Tokio Channels Now, NATS If Needed (GR2-D3) — REQ-4
**Decided:** In-process Tokio broadcast/mpsc/watch channels for the message fabric. Topic enum already maps to NATS subjects — swap is a future option, not a rewrite.
**Alternative was:** NATS as managed sidecar, or NATS from day one.
**Why this won:** NATS can't embed in Rust. The daemon is single-process — Tokio channels are zero-overhead for in-process routing. SQLite handles persistence. NATS adds value only if cross-process messaging is needed, which it isn't yet.

### 4. Summary Digests with Fallback to Explicit (GR2-D4) — REQ-5, GR1-D5
**Decided:** Idle agents get lightweight summary digests of what the active agent did. Per-agent quota meters in the dashboard. Auto-fallback from digests to explicit-only routing at 80% quota threshold. Digests scoped to task, not fleet (GR5-D2).
**Alternative was:** Passive firehose monitoring (burns quota), or explicit routing only (no self-activation).
**Why this won:** Summaries give idle agents enough context to self-activate on high-value moments without burning quota on every token. The daemon controls the cost. Observable via routing log and quota meters.

### 5. Daemon Extracts Decisions, Human Confirms (GR2-D5) — GR1-D4
**Decided:** No Markdown keyword protocol. JSON is the native transport from all three agents (per GR2-D2). Daemon extracts decision-like statements from structured JSON, proposes memory writes to the dashboard, human or second agent confirms.
**Alternative was:** `# DECISION:` Markdown keywords scraped from agent output, or structured JSON tool calls emitted by agents.
**Why this won:** Agents don't need to follow any protocol. The daemon does the work. Human confirms. Most resilient approach — doesn't depend on agent compliance.

### 6. No Hot Cache, Just SQLite (GR2-D6) — REQ-3
**Decided:** SQLite WAL handles all memory reads. No NATS KV, no in-memory HashMap, no moka cache. Add cache only if profiling demands it.
**Alternative was:** In-memory HashMap or moka crate as hot cache in front of SQLite.
**Why this won:** The memory dataset is dozens to hundreds of entries. SQLite reads are microseconds. A cache adds complexity for negligible speedup.

### 7. Provider Abstraction — CLI + API Backends (GR4-D1) — REQ-7, REQ-4
**Decided:** `AgentConnector` trait supports both CLI subprocess and API backends. Config says `claude_backend = "cli"` or `claude_backend = "api"`. Daemon doesn't care. Protects against Anthropic's April 4, 2026 policy change on third-party frameworks using subscriptions.
**Alternative was:** CLI-only (subscription), or API-only (pay-per-token).
**Why this won:** Costs nothing to build the abstraction. If Anthropic enforces, flip the config. No code change. You're running 5 Claude instances on subscription today — not blocked, but protected.

### 8. Dynamic Dashboard — Tasks + Agents View (GR4-D2) — REQ-7, REQ-6
**Decided:** Dashboard has two views, toggled. Tasks view: grouped by work item, shows which agents are assigned, progress, results. Agents view: dynamic grid, one pane per running agent. Default to tasks.
**Alternative was:** Fixed 4-pane grid (doesn't scale), or agents-only view.
**Why this won:** Tasks view for the executive (you), agents view for debugging. Dynamic grid auto-layouts based on fleet size.

### 9. Worktrees + Contracts + Task List + Sequential Merge (GR4-D3) — REQ-7
**Decided:** Each fleet member gets a git worktree. Daemon defines contracts first (Wave 0 pattern). Shared task list in SQLite with dependency tracking. Agents claim tasks. Merge is sequential, not parallel. Peer messaging through the fabric. Human gate for architectural conflicts.
**Alternative was:** File-level locking only (breaks on shared imports), or no coordination (chaos).
**Why this won:** This is the convergent pattern from Claude Agent Teams, Ruflo, Clash, and community solutions. Proven across every serious multi-agent tool. Applied cross-model for the first time.

### 10. Study Ruflo, Clash, swarms-rs (GR4-D4) — REQ-7
**Decided:** During implementation planning, crack open source code from Ruflo (multi-model routing, cost optimization), Clash (real-time worktree conflict detection), and swarms-rs (Rust agent lifecycle). Borrow patterns that work. Attribute everything in NOTICE.md and inline comments.
**Alternative was:** Build everything from scratch, or use Ruflo as a dependency.
**Why this won:** No need to reinvent wheels that are already round. But no dependency on someone else's orchestrator — you own the stack. Same approach as studying Temporal's source for the workflow engine.

### 11. REQ-7: Dynamic Multi-Agent Fleet (NEW)
**Decided:** The daemon can spawn N instances of any agent type on demand. Fleet composition defined per-task: "3 Claudes + 2 Codexes + 1 Gemini" or "5 Codexes + 1 Claude." Connectors become pools. Daemon manages task assignment, context distribution, collision prevention, quota tracking, and result aggregation.
**Alternative was:** Fixed 1-1-1 topology (one Claude, one Gemini, one Codex).
**Why this won:** This is the feature that doesn't exist anywhere. Every multi-agent tool is single-provider. Cross-model fleet coordination is what makes Triumvirate worth building.

---

## CLANKER CONSENSUS (Auto-Resolved)

### 1. Fix "mature Go bindings" Copy-Paste Error — REQ-4
SQLite binding text updated from "mature Go bindings" to "rusqlite 0.32 bundled." Factual error, unanimous.

### 2. Stenographer Transport → Tokio Channels — REQ-2
Stenographer subscribes to Tokio broadcast channels instead of NATS JetStream. Direction unchanged (mechanical extraction), transport changed. Unanimous.

### 3. Mechanical Digests from Structured JSON (GR3-D1) — GR2-D4
Daemon generates summary digests mechanically from structured JSON — template-based extraction, not LLM summarization. Attach raw event IDs for drill-down. Resolves the contradiction between GR2-D4 (digests) and REQ-2 (no LLM summarization). Both twins unanimous on Option A.

### 4. Claude Confirmed Persistent Subprocess (GR3-D2) — REQ-1
Research confirmed: `--input-format stream-json` + `--output-format stream-json` is a persistent subprocess. System prompt sent once, subsequent messages via stdin. Agent SDK uses this exact pattern. Factual confirmation.

### 5. Cedar Ready for Week 2 — REQ-4
`cedar-policy` Rust crate is mature, actively maintained, RBAC/ABAC, millisecond evaluation. No decision needed — add during implementation. Factual.

### 6. Workflow Engine Adds FleetWorkflow Type (GR5-D1) — REQ-7
The purpose-built engine (GR2-D1) adds a FleetWorkflow type for fan-out N agents, track N worktrees, sequential merge. Same SQLite-backed state machine. Additive, not redesign.

### 7. Digests Scoped to Task, Not Fleet (GR5-D2) — REQ-7
With 7 agents on 3 tasks, digests go to agents on the SAME task, not the whole fleet. Scoping clarification.

### 8. Success Criteria Updated for Fleet (GR6-D1, GR6-D2)
Success criteria 1 and 5 updated from "three agents" to "N agents." New criterion 7 added: fleet demo — spin up mixed fleet, coordinate via worktrees, merge results, visible in dashboard.

### 9. Spec Prose Rewrite Required
All Go references (code examples, install commands, library names, struct syntax) must be replaced with Rust equivalents. The tech stack table is the source of truth. Acknowledged by all three reviewers.

---

## RESEARCH ARTIFACTS

- 032-multi-agent-coordination-landscape.md — Claude Teams, Ruflo, Clash, swarms-rs, Event Horizon, AGOR patterns
- 12 Gemini searches across concurrency limits, Cedar, Claude/Codex/Gemini CLI capabilities, Temporal alternatives, NATS in Rust, portable-pty
- Twin reviews from Gemini daemon (gd_mnl610cm_1) and Codex daemon (cd_mnl61ibv_1)
- Prior art: Ruflo (ruvnet/ruflo), Clash, swarms-rs, Event Horizon, CC Mirror, AGOR
