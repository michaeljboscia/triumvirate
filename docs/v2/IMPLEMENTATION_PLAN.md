# IMPLEMENTATION_PLAN — Triumvirate v2

**Version:** 1.0
**Date:** 2026-04-05
**Cross-refs:** PRD.md (FEAT-IDs), TECH_STACK.md, BACKEND_STRUCTURE.md
**Source:** SPEC.md Implementation Plan + Goat Rodeo decisions
**Prior art to study:** Ruflo (ruvnet/ruflo), Clash, swarms-rs, Temporal (temporalio/temporal)

**This file is the map. It does not get modified during execution.**

---

## Phase 0: Prior Art Research (Day 1 morning)

Before writing code, crack open source repos to extract patterns.

- [ ] **0.1** Clone and study Ruflo (`ruvnet/ruflo`): multi-model routing, cost optimization, swarm coordination. Document findings in `research/034-ruflo-source-analysis.md`. FEAT: FEAT-001, FEAT-010
- [ ] **0.2** Clone and study Clash: real-time worktree conflict detection algorithm. Document in `research/035-clash-source-analysis.md`. FEAT: FEAT-019
- [ ] **0.3** Clone and study swarms-rs: Rust agent lifecycle management patterns. Document in `research/036-swarms-rs-source-analysis.md`. FEAT: FEAT-001, FEAT-023
- [ ] **0.4** Study Temporal source (`temporalio/temporal`): `service/history/workflow/`, `internal/common/backoff/`, event sourcing patterns. Document in `research/037-temporal-workflow-patterns.md`. FEAT: FEAT-007

**Gate:** Research docs written. Patterns identified. Proceed to POC 2.

---

## Phase 1: POC 2 — The Live Agent (Day 1 afternoon)

Build on existing scaffold at `daemon/`. POC 1 is done (boots, serves HTML, finds CLIs).

- [ ] **1.1** Implement `ClaudeConnector::spawn()` — spawn `claude --input-format stream-json --output-format stream-json --session-id <uuid>` with piped stdin/stdout. Dedicated reader/writer tokio tasks. File: `daemon/crates/agentd/src/agent/claude.rs`. FEAT: FEAT-002
- [ ] **1.2** Implement JSONL parser for Claude stream-json events (init, message, tool_use, result, error). File: `daemon/crates/proto/src/claude_events.rs`. FEAT: FEAT-002
- [ ] **1.3** Wire Claude output → fabric → WebSocket → browser. Add `/ws` endpoint to axum server. Files: `daemon/crates/agentd/src/web/ws.rs`, update `server.rs`. FEAT: FEAT-014
- [ ] **1.4** Add `POST /api/message` endpoint — human message → fabric → Claude stdin. File: update `daemon/crates/agentd/src/web/server.rs`. FEAT: FEAT-008
- [ ] **1.5** Update `static/index.html` — connect WebSocket, display streaming Claude output in pane. File: `daemon/static/index.html`. FEAT: FEAT-014

**Gate:** Type a message in the browser, Claude responds in real-time. WebSocket carries the message.

---

## Phase 2: POC 3 — The Triplex (Day 2)

- [ ] **2.1** Implement `GeminiConnector::spawn()` — spawn `gemini --acp` with piped stdio. JSON-RPC request/response parsing. File: `daemon/crates/agentd/src/agent/gemini.rs`. FEAT: FEAT-003
- [ ] **2.2** Implement ACP JSON-RPC parser. File: `daemon/crates/proto/src/gemini_events.rs`. FEAT: FEAT-003
- [ ] **2.3** Implement `CodexConnector::spawn()` — spawn `codex mcp-server` with piped stdio. MCP JSON-RPC parsing. File: `daemon/crates/agentd/src/agent/codex.rs`. FEAT: FEAT-004
- [ ] **2.4** Implement MCP event parser. File: `daemon/crates/proto/src/codex_events.rs`. FEAT: FEAT-004
- [ ] **2.5** Implement message routing — `@claude`, `@gemini`, `@codex` directives + lead agent default (GR1-D3). File: `daemon/crates/agentd/src/routing.rs`. FEAT: FEAT-008
- [ ] **2.6** Implement digest system — daemon generates mechanical template from structured JSON for idle agents. File: `daemon/crates/agentd/src/digest.rs`. FEAT: FEAT-018
- [ ] **2.7** Update dashboard — 3 agent panes streaming simultaneously. File: `daemon/static/index.html`. FEAT: FEAT-016

**Gate:** Type a question, see Claude, Gemini, and Codex all respond in real-time. Digests visible in event log.

---

## Phase 3: Foundation (Day 3-4)

### 3A: Memory & Stenographer

- [ ] **3.1** Expand SQLite schema — add routing_log table, workflow tables. File: `daemon/crates/agentd/src/memory/store.rs`. FEAT: FEAT-012, FEAT-007
- [ ] **3.2** Implement stenographer — subscribe to firehose, capture agent messages, tool calls, file changes. Write structured JSON session log. File: `daemon/crates/agentd/src/steno/mod.rs`. FEAT: FEAT-011
- [ ] **3.3** Add `notify` crate — watch working directory for file changes, feed to stenographer. File: `daemon/crates/agentd/src/steno/watcher.rs`. FEAT: FEAT-011
- [ ] **3.4** Implement decision extraction — heuristic scan of agent JSON output, propose to dashboard. File: `daemon/crates/agentd/src/memory/extraction.rs`. FEAT: FEAT-013
- [ ] **3.5** Add memory injection — on each agent turn, prepend relevant memories to prompt. File: `daemon/crates/agentd/src/agent/context.rs`. FEAT: FEAT-012

### 3B: Workflow Engine

- [ ] **3.6** Create `crates/workflow/` crate — state machine core, SQLite persistence, event log. Informed by Temporal patterns from Phase 0. Files: `daemon/crates/workflow/src/{lib.rs, engine.rs, state.rs, persistence.rs}`. FEAT: FEAT-007
- [ ] **3.7** Implement ConversationWorkflow — human → route → agent → publish → digest. File: `daemon/crates/workflow/src/conversation.rs`. FEAT: FEAT-008
- [ ] **3.8** Implement retry with backoff — configurable exponential backoff, max 3 retries. File: `daemon/crates/workflow/src/retry.rs`. FEAT: FEAT-007
- [ ] **3.9** Implement human gate — workflow pauses, sends signal to dashboard via WebSocket, waits for approval. File: `daemon/crates/workflow/src/human_gate.rs`. FEAT: FEAT-007

### 3C: Health & Config

- [ ] **3.10** Formalize health monitor — watch channels per agent, publish transitions to fabric, expose via `/api/health`. File: `daemon/crates/agentd/src/agent/health.rs`. FEAT: FEAT-023
- [ ] **3.11** Implement auto-restart with jitter/backoff for dead agents. File: `daemon/crates/agentd/src/agent/supervisor.rs`. FEAT: FEAT-001
- [ ] **3.12** Implement quota tracking — count tokens per agent from structured JSON responses, store in routing_log. File: `daemon/crates/agentd/src/quota.rs`. FEAT: FEAT-017

**Gate:** Multi-turn conversation with memory. Session notes written. Workflow engine persists state. Agents auto-restart.

---

## Phase 4: Fleet (Day 5-6)

- [ ] **4.1** Implement agent pool — `AgentPool<T: AgentConnector>` with spawn N, track instances, route to specific instance. File: `daemon/crates/agentd/src/agent/pool.rs`. FEAT: FEAT-001
- [ ] **4.2** Implement worktree manager — `git worktree add` per fleet member, cleanup on teardown. File: `daemon/crates/agentd/src/fleet/worktree.rs`. FEAT: FEAT-019
- [ ] **4.3** Implement shared task list — SQLite fleet_tasks table, claim/complete/block operations. File: `daemon/crates/agentd/src/fleet/tasks.rs`. FEAT: FEAT-020
- [ ] **4.4** Implement FleetWorkflow — Wave 0 contracts, parallel fan-out, sequential merge. File: `daemon/crates/workflow/src/fleet.rs`. FEAT: FEAT-010
- [ ] **4.5** Implement sequential merge — merge branches one at a time, detect conflicts, surface to dashboard. File: `daemon/crates/agentd/src/fleet/merge.rs`. FEAT: FEAT-021
- [ ] **4.6** Implement peer messaging — fleet members send messages through fabric to specific other members. File: `daemon/crates/agentd/src/fleet/peer.rs`. FEAT: FEAT-010
- [ ] **4.7** Add `/api/fleet/*` endpoints — spawn, status, merge. File: update `daemon/crates/agentd/src/web/server.rs`. FEAT: FEAT-010
- [ ] **4.8** Implement `POST /api/fleet/spawn` — parse fleet spec, provision worktrees, spawn instances, create task list. FEAT: FEAT-010

**Gate:** Spawn 2 Claudes + 1 Codex on the same task. Worktrees created. Tasks claimed. Results merged. Visible in dashboard.

---

## Phase 5: Dashboard (Day 7-8)

- [ ] **5.1** Initialize Svelte 5 + Tailwind 4 + Vite 6 project in `daemon/frontend/`. Per FRONTEND_GUIDELINES.md file structure. FEAT: FEAT-014
- [ ] **5.2** Implement stores — `agents.svelte.ts`, `tasks.svelte.ts`, `fabric.svelte.ts`, `quota.svelte.ts`, `memory.svelte.ts`. Per FRONTEND_GUIDELINES.md. FEAT: FEAT-014
- [ ] **5.3** Build Tasks View — TaskList, TaskCard, FleetProgress components. Per DESIGN_SYSTEM.md tokens. FEAT: FEAT-015
- [ ] **5.4** Build Agents View — AgentGrid, AgentPane, StatusDot, streaming output. Dynamic grid layout. FEAT: FEAT-016
- [ ] **5.5** Build Input Area — message input, send/debate/interrupt buttons, keyboard shortcuts. FEAT: FEAT-014
- [ ] **5.6** Build Header — system status badge, view toggle, quota summary. FEAT: FEAT-014
- [ ] **5.7** Build Quota Dashboard — per-agent budget bars, routing log, summary/direct ratio. FEAT: FEAT-017
- [ ] **5.8** Build Memory Viewer — list memories, decision confirmation modal. FEAT: FEAT-012, FEAT-013
- [ ] **5.9** Build Workflow Panel — state machine visualization, step status, retry indicators. FEAT: FEAT-007
- [ ] **5.10** Build Merge Resolver — conflict diff display, human resolution controls. FEAT: FEAT-021
- [ ] **5.11** Wire `rust-embed` — build Svelte output, embed in Rust binary. Update Cargo.toml `#[folder]` path. FEAT: FEAT-014

**Gate:** Full Svelte dashboard replaces POC HTML. All views functional. Real-time streaming. Design system applied pixel-perfect.

---

## Phase 6: Debate & Governance (Day 9)

- [ ] **6.1** Implement DebateWorkflow — `/debate` trigger, proposal → challenge → vote → decision. File: `daemon/crates/workflow/src/debate.rs`. FEAT: FEAT-009
- [ ] **6.2** Add Cedar governance — load policies from `~/.triumvirate/policies/`, evaluate before destructive ops. File: `daemon/crates/agentd/src/governance.rs`. FEAT: FEAT-022
- [ ] **6.3** Write default Cedar policies — human approval for git push, file delete, db drop. File: `daemon/policies/default.cedar`. FEAT: FEAT-022
- [ ] **6.4** Implement provider abstraction — API backend for AgentConnector trait. File: `daemon/crates/agentd/src/agent/api_backend.rs`. FEAT: FEAT-005

**Gate:** Structured debate works. Cedar blocks destructive ops without approval. API backend compiles.

---

## Phase 7: Armor (Day 10-11)

- [ ] **7.1** Implement crash recovery — on boot, read workflow.db, list incomplete workflows, offer resume. File: `daemon/crates/workflow/src/recovery.rs`. FEAT: FEAT-024
- [ ] **7.2** Implement graceful shutdown — SIGTERM handler, signal all agents, wait for completion, close SQLite cleanly. File: `daemon/crates/agentd/src/shutdown.rs`. FEAT: FEAT-024
- [ ] **7.3** Build mock CLIs — `mock-claude`, `mock-gemini`, `mock-codex` with configurable responses and latency. Feature flag `--features mock`. Files: `daemon/crates/mock-claude/`, `daemon/crates/mock-gemini/`, `daemon/crates/mock-codex/`. FEAT: FEAT-027
- [ ] **7.4** Write integration tests with mock CLIs — conversation, debate, fleet, crash recovery. File: `daemon/tests/`. FEAT: FEAT-027
- [ ] **7.5** E2E test with real CLIs — single conversation turn with each agent. Acceptance gate. File: `daemon/tests/e2e/`. FEAT: GR1-D7
- [ ] **7.6** Add OpenTelemetry tracing — per-turn spans, token counting, cost calculation. File: `daemon/crates/agentd/src/telemetry.rs`. FEAT: FEAT-025
- [ ] **7.7** Chaos test — kill daemon mid-workflow, restart, verify resume. Document results. FEAT: FEAT-024
- [ ] **7.8** Process management — ensure all child processes die when daemon dies (process groups, Pdeathsig equivalent on macOS). File: `daemon/crates/agentd/src/agent/supervisor.rs`. FEAT: FEAT-001

**Gate:** Kill -9 the daemon, restart, conversation resumes. All mock CLI tests pass. Real CLI E2E passes. No orphan processes.

---

## Phase 8: Ship (Day 12)

- [ ] **8.1** Add `target/` to `.gitignore`. Clean up build artifacts.
- [ ] **8.2** Write `NOTICE.md` — attribution for Ruflo, Clash, swarms-rs, Temporal.
- [ ] **8.3** Update `README.md` — installation, usage, architecture overview.
- [ ] **8.4** `cargo clippy -- -D warnings` — zero warnings.
- [ ] **8.5** `cargo test` — all tests pass.
- [ ] **8.6** `cargo build --release` — optimized binary.
- [ ] **8.7** Run success criteria 1-7 from SPEC.md. Document results.
- [ ] **8.8** Commit everything. Tag `v0.1.0`.

**Gate:** All 7 success criteria from SPEC.md verified. Binary runs. Dashboard works. Fleet coordinates. Memory persists. Sessions resume.
