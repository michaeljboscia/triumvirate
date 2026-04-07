# Product Requirements Document — Triumvirate v2.2

**Version:** 2.2 — The Accountability Release
**Date:** 2026-04-07
**Source:** SPEC.md (8-round goat rodeo, Phase 3 CLEAN)

---

## Features

### FEAT-001: Triumvirate Ledger
**REQs:** REQ-001–013b, REQ-015–018a
**Acceptance:** Events captured in SQLite within 2s (p95). `triumvirate doctor` reports green. Zero events lost on daemon crash + restart.

The Ledger replaces the broken stenographer with SQLite-backed session persistence. Hooks write to a spool directory (atomic rename, <1ms). The daemon drains the spool into SQLite asynchronously. No mutable coordination flags. No JSON state files. Health is always visible.

### FEAT-002: Lessons Ledger
**REQs:** REQ-019–022
**Acceptance:** Lessons searchable via FTS5. Confidence decays correctly (< 0.5 after 50 days at lambda=0.01). Auto-created from error_resolution/bug_fix/architecture_decision summaries.

Machine-readable lessons with exponential confidence decay. Calculated at query time. Auto-proposed by compression worker for high-signal summaries.

### FEAT-003: Cross-Model Peer Review
**REQs:** REQ-023–027b, REQ-024a
**Acceptance:** Agent cannot approve its own work. Review spawns full non-author agent on demand. Bounded queue (max 2 inflight, 120s timeout).

No self-approval. Reviews use full agent sessions. Round-robin assignment among non-author agent types. Optional for standalone work, mandatory for fleet merges.

### FEAT-004: Fleet/Swarm Execution
**REQs:** REQ-028–037, REQ-034a
**Acceptance:** 3-agent fleet spawns worktrees, claims tasks atomically, produces output, merges sequentially with peer review gate. Crash recovery on daemon restart.

Parallel agent execution with git worktrees, SQLite task lists, atomic claiming, sequential merge with review gates, and daemon crash recovery.

### FEAT-005: Dashboard
**REQs:** REQ-041–048, REQ-044a, REQ-045a–045e
**Acceptance:** Dashboard loads at localhost:8080. All 6 views functional. Ledger health indicator green/yellow/red. Live agent state streaming.

Svelte 5 + Tailwind 4 SPA embedded via rust-embed. 6 views: Sessions, Fleet, Ledger, Lessons, Reviews, Metrics. WebSocket real-time updates. Design system document required before development.

### FEAT-006: OutboxEvent Enrichment
**REQs:** REQ-049–050, REQ-049a, REQ-049b
**Acceptance:** OutboxEvent contains working_state, token_usage, tool_name from agent output.

Backward-compatible struct extension. Fields populated from ParsedAgentResult in agent_exec.

### FEAT-007: Codex App-Server Protocol
**REQs:** REQ-051–055, REQ-054a
**Acceptance:** Codex connects via app-server JSON-RPC 2.0 when configured. Auto-approve via --full-auto flag. Capability probe at startup.

New CodexAppServerParser in agent-adapter. Falls back to exec protocol if approval channel is broken. All auto-approved actions logged to Ledger.

### FEAT-008: Outbox Rotation/GC
**REQs:** REQ-056–058
**Acceptance:** Events >30 days without summaries deleted. GC runs on startup if >24h since last. GC blocked during active fleets.

Retention policy on events table. MCP tool + automatic startup trigger. Dead-drop ticket cleanup.

---

## Feature Dependencies

```
FEAT-001 (Ledger) ← FEAT-002 (Lessons)
FEAT-001 (Ledger) ← FEAT-004 (Fleet) ← FEAT-003 (Peer Review)
FEAT-001 (Ledger) ← FEAT-005 (Dashboard)
FEAT-001 (Ledger) ← FEAT-008 (GC)
FEAT-006 (Enrichment) — independent
FEAT-007 (Codex Protocol) — independent
```

## MCP Tool Inventory

| Tool | Feature | Phase |
|------|---------|-------|
| `ledger_health()` | FEAT-001 | 1 |
| `ledger_query()` | FEAT-001 | 2 |
| `ledger_session()` | FEAT-001 | 2 |
| `ledger_record()` | FEAT-001 | 2 |
| `lesson_add()` | FEAT-002 | 2 |
| `lesson_query()` | FEAT-002 | 2 |
| `lesson_validate()` | FEAT-002 | 2 |
| `lesson_list()` | FEAT-002 | 2 |
| `review_request()` | FEAT-003 | 4 |
| `review_submit()` | FEAT-003 | 4 |
| `review_status()` | FEAT-003 | 4 |
| `fleet_spawn()` | FEAT-004 | 3 |
| `fleet_status()` | FEAT-004 | 3 |
| `fleet_task_list()` | FEAT-004 | 3 |
| `fleet_claim_task()` | FEAT-004 | 3 |
| `fleet_cancel()` | FEAT-004 | 3 |
| `ledger_gc()` | FEAT-008 | 7 |

17 new MCP tools total.
