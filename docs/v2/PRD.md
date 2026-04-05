# PRD — Triumvirate v2

**Version:** 1.0
**Date:** 2026-04-05
**Source:** SPEC.md (Final, Goat Rodeo R1-R6)
**Cross-refs:** APP_FLOW.md, TECH_STACK.md, BACKEND_STRUCTURE.md, IMPLEMENTATION_PLAN.md

---

## Product Summary

A Rust binary (`triumvirate-agentd`) that orchestrates Claude, Gemini, and Codex as a dynamic multi-agent fleet. The first cross-model multi-agent coordinator — any number of any agent type, working in parallel, coordinated by one daemon, visible in one dashboard.

---

## Feature List

### Agent Connectivity

**FEAT-001: Agent Connector Pool**
- REQ: REQ-1, REQ-7
- Priority: P0 (blocks everything)
- Description: Pool of persistent CLI subprocesses per agent type. Each type implements the `AgentConnector` trait. Pool scales from 1 to N per type at runtime.
- Acceptance:
  - Spawn N instances of any agent type via config or runtime command
  - Each instance has independent session, health status, and quota tracking
  - Pool auto-restarts dead instances with jitter/backoff
  - `CLAUDE_CONFIG_DIR` env var set per instance to isolate sessions

**FEAT-002: Claude Connector**
- REQ: REQ-1
- Priority: P0
- Description: Persistent subprocess using `claude --input-format stream-json --output-format stream-json --session-id <id>`. Bidirectional JSONL over stdin/stdout.
- Acceptance:
  - Send message via stdin, receive streaming JSONL on stdout
  - Session persists across turns (session-id continuity)
  - Parse all Claude stream-json event types (init, message, tool_use, result, error)
  - Health check: output within 30s timeout

**FEAT-003: Gemini Connector**
- REQ: REQ-1
- Priority: P0
- Description: Persistent subprocess using `gemini --acp`. JSON-RPC over stdio. Multi-turn sessions with save/load.
- Acceptance:
  - Send JSON-RPC requests via stdin, receive responses on stdout
  - Support cancellation via JSON-RPC cancel method
  - Parse ACP event types (init, message, tool_call, error)
  - Health check: response to ping within 30s

**FEAT-004: Codex Connector**
- REQ: REQ-1
- Priority: P0
- Description: Persistent subprocess using `codex mcp-server`. JSON-RPC/MCP over stdio. Multi-turn via `codex-reply` tool.
- Acceptance:
  - Send MCP requests via stdin, receive responses on stdout
  - Multi-turn via codex-reply tool invocation
  - Parse MCP event types (thread.started, turn.started, item.*, turn.completed)
  - Health check: response within 30s

**FEAT-005: Provider Abstraction**
- REQ: REQ-7, REQ-4
- Priority: P1
- Description: `AgentConnector` trait supports both CLI subprocess and API backends. Config switch: `backend = "cli"` or `backend = "api"`.
- Acceptance:
  - Same trait interface for CLI and API modes
  - Config-driven backend selection per agent type
  - API mode uses `anthropic`/`openai`/`google-ai` crates
  - Switching backend requires zero code changes

### Message Fabric

**FEAT-006: Tokio Channel Fabric**
- REQ: REQ-1, REQ-4
- Priority: P0
- Description: In-process message bus using Tokio broadcast/mpsc/watch channels. Topic enum maps to NATS subjects for future swap path.
- Acceptance:
  - Publish messages to typed topics
  - Subscribe to specific topics or firehose (all topics)
  - Broadcast channel capacity: 256 per topic, 1024 for firehose
  - Lagged subscribers receive warning, not crash
  - Zero external dependencies

### Workflow Engine

**FEAT-007: Workflow Engine Core**
- REQ: REQ-4
- Priority: P0
- Description: Purpose-built, SQLite-backed state machine with event sourcing. Informed by Temporal source (Apache 2.0). Handles retry, compensation, human-in-the-loop, crash recovery.
- Acceptance:
  - Workflows persist to SQLite WAL (workflow_id, step, state, payload)
  - Write-ahead log: intent written before execution, completion after
  - On crash: incomplete workflows resume from last completed step
  - Retry with configurable backoff (exponential, max 3 retries)
  - Human gate: workflow pauses, sends signal to dashboard, waits for approval

**FEAT-008: Conversation Workflow**
- REQ: REQ-1
- Priority: P0
- Description: N-way live collaboration. Human sends message, daemon routes to lead agent, response published to fabric, digests sent to idle agents.
- Acceptance:
  - Human message → routing decision → agent turn → fabric publish → digest to idle
  - Lead agent rotation per GR1-D3 (architecture→Claude, research→Gemini, implementation→Codex)
  - Human can override lead with @-mention
  - Conversation persists in SQLite for session resume

**FEAT-009: Debate Workflow**
- REQ: REQ-1
- Priority: P1
- Description: Structured Toulmin debate. One agent proposes (claim + data + warrant), another challenges (rebuttal), vote to decide.
- Acceptance:
  - `/debate <topic>` triggers workflow
  - 3-phase: proposal → challenge → vote
  - Each agent's position captured as structured JSON
  - Winner determined by majority vote
  - Decision auto-extracted to memory (per FEAT-017)

**FEAT-010: Fleet Workflow**
- REQ: REQ-7
- Priority: P0
- Description: Fan-out N agents across worktrees. Contract definition (Wave 0), parallel execution, sequential merge.
- Acceptance:
  - User defines fleet composition: `{"claude": 3, "codex": 2, "gemini": 1}`
  - Daemon provisions worktrees, spawns instances, assigns tasks
  - Wave 0: contracts defined before fan-out
  - Parallel execution with shared task list
  - Sequential merge: one branch at a time
  - Workflow state visible in dashboard

### Stenographer

**FEAT-011: Mechanical Session Notes**
- REQ: REQ-2
- Priority: P0
- Description: Fabric consumer that mechanically extracts facts from structured JSON events. No LLM summarization. Verified against git and tool results.
- Acceptance:
  - Subscribes to fabric firehose
  - Captures: agent messages, tool calls, file changes (notify crate), decisions
  - Writes structured JSON session log on session end
  - Every claim traceable to a fabric message ID or git commit
  - Git diff between session start and end included
  - Resume command included in session log

### Memory

**FEAT-012: SQLite Memory Store**
- REQ: REQ-3
- Priority: P0
- Description: SQLite WAL database with memories, sessions, and decisions tables. No hot cache. Concurrent reads via WAL mode.
- Acceptance:
  - CRUD operations on memories (key, value, type, agent, timestamp)
  - Memory types: user, feedback, project, reference
  - Session tracking: start, end, agents involved, summary
  - Decision tracking: outcome, evidence, proposed_by, validated_by
  - Loud failure on write errors (dashboard banner, not silent skip)

**FEAT-013: Decision Extraction**
- REQ: REQ-3
- Priority: P1
- Description: Daemon parses structured JSON from agents, detects decision-like content, proposes memory writes to dashboard. Human or second agent confirms.
- Acceptance:
  - Daemon scans agent output for decision patterns (heuristic)
  - Proposed decisions appear in dashboard for confirmation
  - Confirmed decisions written to SQLite decisions table
  - Rejected proposals logged but not persisted
  - No Markdown keyword protocol — JSON is native transport

### Dashboard

**FEAT-014: Web Dashboard Server**
- REQ: REQ-6
- Priority: P0
- Description: axum HTTP server on :8080 with `rust-embed` static assets. WebSocket for real-time streaming.
- Acceptance:
  - Serves Svelte+Tailwind build at /
  - REST API at /api/* for health, agents, memory, tasks
  - WebSocket at /ws for real-time fabric event streaming
  - CORS enabled for local development
  - Binds to 127.0.0.1 only (localhost)

**FEAT-015: Tasks View**
- REQ: REQ-6, REQ-7
- Priority: P0
- Description: Default dashboard view. Tasks grouped by work item. Shows assigned agents, progress, completion status.
- Acceptance:
  - Each task shows: name, status, assigned agents, progress
  - Click into task to see agent output for that task
  - Fleet tasks show worktree status and merge progress
  - Real-time updates via WebSocket

**FEAT-016: Agents View**
- REQ: REQ-6, REQ-7
- Priority: P1
- Description: Debug view. Dynamic grid with one pane per running agent. Streaming output, health indicators, quota meters.
- Acceptance:
  - Grid auto-layouts based on fleet size (1 agent = full, 4 = 2x2, 7 = dynamic)
  - Each pane: agent name, model, status dot, streaming output
  - Color-coded per agent type (Claude purple, Gemini blue, Codex green)
  - Toggle between Tasks and Agents view

**FEAT-017: Quota Management Dashboard**
- REQ: REQ-5, REQ-7
- Priority: P0
- Description: Per-agent and per-model-type quota meters. Routing log showing every message, target, token cost.
- Acceptance:
  - Budget bar per agent (tokens used / window remaining)
  - Auto-fallback indicator when agent hits 80% threshold
  - Routing log: timestamp, target agent, type (direct/summary/background), tokens
  - Summary vs direct ratio per agent

### Fleet Coordination

**FEAT-018: Digest System**
- REQ: REQ-5
- Priority: P1
- Description: Daemon generates mechanical digests from structured JSON for idle agents on the same task. Template-based, not LLM-generated. Includes raw event IDs for drill-down.
- Acceptance:
  - Digest format: "{Agent} {action}. [{N} tool calls, {M} files modified]. {Idle agent}, anything to add?"
  - Digests scoped to task, not fleet (GR5-D2)
  - Raw fabric event IDs attached for source verification
  - Digest generation skipped when agent at 80%+ quota

**FEAT-019: Worktree Manager**
- REQ: REQ-7
- Priority: P0
- Description: Creates and manages git worktrees for fleet members. One worktree per agent instance. Cleanup on fleet teardown.
- Acceptance:
  - `git worktree add` per fleet member with unique branch name
  - Agent's working directory set to its worktree
  - Worktree cleanup on agent shutdown
  - Conflict detection between worktrees (informed by Clash patterns)

**FEAT-020: Shared Task List**
- REQ: REQ-7
- Priority: P0
- Description: SQLite-backed task list with dependency tracking. Fleet members claim tasks. Completed tasks unblock dependents.
- Acceptance:
  - Task states: pending, claimed, in_progress, completed, blocked, failed
  - Dependency tracking: task B blocked until task A completes
  - Claim mechanism: atomic SQLite update (no double-claiming)
  - Dashboard shows task list with status and assignee

**FEAT-021: Sequential Merge**
- REQ: REQ-7
- Priority: P0
- Description: Fleet branches merged one at a time into main. Each merge gets full context of previous merges.
- Acceptance:
  - Merge order determined by task dependency graph
  - Each merge: checkout main, merge branch, run tests
  - On conflict: present to dashboard for human resolution
  - Merge results logged to stenographer

### Governance & Observability

**FEAT-022: Cedar Governance**
- REQ: REQ-4
- Priority: P2 (Week 2)
- Description: `cedar-policy` crate for authorization policies. Controls what each agent can/can't do. Human approval gates for destructive operations.
- Acceptance:
  - Cedar policies loaded from `~/.triumvirate/policies/`
  - Policy evaluation before destructive ops (file delete, git push, db drop)
  - Denied actions logged and surfaced in dashboard
  - Human override via dashboard approval

**FEAT-023: Health Monitor**
- REQ: REQ-4
- Priority: P0
- Description: Watches agent health via `watch::Receiver<HealthStatus>`. Publishes health changes to fabric. Dashboard displays status.
- Acceptance:
  - Health states: Starting, Ready, Busy, Unresponsive, Restarting, Dead
  - Transition events published to fabric
  - Dashboard status dots update in real-time
  - Unresponsive after 30s without output

**FEAT-024: Crash Recovery**
- REQ: REQ-4
- Priority: P1
- Description: On daemon restart, load SQLite workflow state, resume incomplete workflows from last completed step.
- Acceptance:
  - Daemon reads workflow.db on boot
  - Incomplete workflows listed with option to resume or abandon
  - Resume replays from last completed step (write-ahead log)
  - Agent subprocesses re-spawned with session-id continuity

**FEAT-025: OpenTelemetry Tracing**
- REQ: REQ-4
- Priority: P2
- Description: Distributed tracing with GenAI semantic conventions. Per-agent spans for latency, tokens, cost.
- Acceptance:
  - Trace per conversation turn (session_id as trace context)
  - Span per agent invocation (agent type, tokens in/out, latency)
  - Export to stdout (dev) or OTLP endpoint (prod)
  - Cost calculation per turn based on token counts

### Infrastructure

**FEAT-026: Config System**
- REQ: REQ-4
- Priority: P0
- Description: TOML config from `~/.triumvirate/config.toml`. Defaults for everything. Override per agent type.
- Acceptance:
  - Falls back to defaults if no config file
  - Agent-specific config: backend, instances, enabled/disabled
  - Web port configurable (default 8080)
  - DB path configurable (default ~/.triumvirate/memory.db)

**FEAT-027: Mock CLIs**
- REQ: GR1-D7
- Priority: P1
- Description: Deterministic mock binaries that mimic Claude/Gemini/Codex JSON protocols. For development and testing without burning CLI quota.
- Acceptance:
  - `mock-claude`: emits stream-json JSONL with configurable responses
  - `mock-gemini`: emits ACP JSON-RPC responses
  - `mock-codex`: emits MCP JSON-RPC responses
  - Configurable latency, error injection, response content
  - Used by CI and dev builds (feature flag `--features mock`)

---

## Priority Summary

| Priority | Features | Description |
|----------|----------|-------------|
| P0 | FEAT-001 through FEAT-008, FEAT-010 through FEAT-012, FEAT-014, FEAT-015, FEAT-017, FEAT-019 through FEAT-021, FEAT-023, FEAT-026 | Core daemon: agents, fabric, workflows, memory, dashboard, fleet |
| P1 | FEAT-005, FEAT-009, FEAT-013, FEAT-016, FEAT-018, FEAT-024, FEAT-027 | Provider abstraction, debate, extraction, debug view, digests, crash recovery, mocks |
| P2 | FEAT-022, FEAT-025 | Cedar governance, OpenTelemetry |
