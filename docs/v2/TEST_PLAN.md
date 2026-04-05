# TEST_PLAN — Triumvirate v2

**Version:** 1.0
**Date:** 2026-04-05
**Cross-refs:** PRD.md (FEAT-IDs), SPEC.md (REQ-IDs), IMPLEMENTATION_PLAN.md

Every REQ has a test. No orphan REQs.

---

| REQ-ID | Acceptance Criteria | Test Type | Pass Condition | FEAT |
|--------|-------------------|-----------|---------------|------|
| REQ-1 | N agents in browser, all talking, all live, structured JSON protocols | E2E | Send message, receive responses from all 3 agents in browser within 30s. WebSocket delivers streaming output. | FEAT-001-004, FEAT-008, FEAT-014 |
| REQ-1 | @-mention routing works | Integration | `@claude <msg>` goes only to Claude. `@gemini` only to Gemini. Verified via routing_log. | FEAT-008 |
| REQ-1 | Lead agent rotation | Integration | Architecture question → Claude. Research question → Gemini. Implementation → Codex. Verified via routing_log. | FEAT-008 |
| REQ-2 | Session notes capture what happened, no hallucination | E2E | Complete a 5-turn conversation. Session log contains: all agent messages (traceable to fabric IDs), git diff, tool calls. No claims without evidence. | FEAT-011 |
| REQ-2 | File changes tracked | Integration | Create a file during session. Stenographer log includes the file in `files_modified`. | FEAT-011 |
| REQ-3 | Memory shared across sessions | E2E | Session 1: make a decision. Session 2: new agent's prompt contains the decision. Verified by inspecting agent stdin. | FEAT-012 |
| REQ-3 | Memory write failure is loud | Integration | Simulate SQLite write failure (disk full mock). Dashboard shows red error banner. No silent skip. | FEAT-012 |
| REQ-3 | Decision extraction and confirmation | Integration | Agent outputs decision-like text. Dashboard shows confirmation prompt. Approve → written to decisions table. Reject → not persisted. | FEAT-013 |
| REQ-4 | Single binary, no external deps | Unit | Binary boots without NATS, Temporal, Docker, or any external process (besides agent CLIs). Verified by `lsof -p <pid>` showing no unexpected network connections. | FEAT-026 |
| REQ-4 | Startup completes in under 5 seconds | Unit | Time from process start to "Ready" log line < 5s (excluding agent spawn which depends on CLI availability). | FEAT-026 |
| REQ-4 | Failure modes visible | Integration | Kill an agent subprocess. Dashboard shows "restarting..." within 5s. Auto-restart occurs. | FEAT-023 |
| REQ-4 | Crash recovery | E2E | Start a workflow. Kill -9 the daemon. Restart. Incomplete workflow listed. Resume from last step. | FEAT-024 |
| REQ-5 | Idle agents receive digests | Integration | Claude responds. Gemini receives mechanical digest (template format, not LLM text). Verified via Gemini's stdin log. | FEAT-018 |
| REQ-5 | Quota auto-fallback | Integration | Simulate 80% quota usage. Verify digests stop for that agent. Only @-mentions reach it. Dashboard shows fallback indicator. | FEAT-017 |
| REQ-5 | Digest scoped to task | Integration | Fleet with 2 tasks. Agent on task A does NOT receive digest from task B agents. | FEAT-018 |
| REQ-6 | Dashboard serves at :8080 | Unit | `curl http://127.0.0.1:8080/` returns HTML. `curl /api/health` returns JSON with status "ok". | FEAT-014 |
| REQ-6 | Tasks view shows active tasks | E2E | Start a conversation. Tasks view shows the task with assigned agent. Status updates in real-time. | FEAT-015 |
| REQ-6 | Agents view shows dynamic grid | E2E | With 3 agents: 2×2 grid (3 agents + event log). With 7 agents: dynamic layout. | FEAT-016 |
| REQ-6 | Dashboard understandable in 30 seconds | Manual | Show dashboard to someone unfamiliar. They can describe what it does without prompting. | FEAT-014 |
| REQ-7 | Fleet spawns N instances | E2E | Request fleet: `{claude: 2, codex: 1}`. Three agent instances spawn with separate sessions. Verified via `/api/agents`. | FEAT-001, FEAT-010 |
| REQ-7 | Worktrees created per fleet member | Integration | Fleet spawns. `git worktree list` shows N+1 worktrees (main + 1 per member). Each agent's cwd is its worktree. | FEAT-019 |
| REQ-7 | Shared task list with dependencies | Integration | Create fleet with 3 tasks. Task B depends on Task A. Agent claims B before A completes → B stays blocked. A completes → B auto-unblocks. | FEAT-020 |
| REQ-7 | Sequential merge | Integration | Fleet completes. Merge first branch → success. Merge second → if conflict, surface to dashboard. Never parallel merge. | FEAT-021 |
| REQ-7 | Contracts first (Wave 0) | E2E | Fleet spawn triggers contract definition step before parallel fan-out. Agents implement against defined interfaces. | FEAT-010 |
| REQ-7 | Fleet composition is dynamic | Integration | First fleet: 3 claude + 2 codex. Second fleet: 5 codex + 1 claude. Both work. Pool scales up and down. | FEAT-001 |
| REQ-7 | Provider abstraction | Unit | Configure `backend = "api"` for Claude. Connector uses API crate instead of CLI subprocess. Same trait interface. | FEAT-005 |

---

## Test Infrastructure

### Mock CLIs (FEAT-027)

Three mock binaries that mimic the exact JSON protocols:

| Mock | Protocol | Configurable |
|------|----------|-------------|
| `mock-claude` | stream-json JSONL | Response content, latency, error injection, tool use simulation |
| `mock-gemini` | ACP JSON-RPC | Response content, latency, error injection |
| `mock-codex` | MCP JSON-RPC | Response content, latency, error injection |

Activated via `--features mock` cargo flag. CI always uses mocks. Real CLIs are the acceptance gate.

### Test Levels

| Level | What | When | Tool |
|-------|------|------|------|
| Unit | Individual functions, parsers, state machines | Every commit | `cargo test` |
| Integration | Multi-component (connector → fabric → WebSocket) | Every feature | `cargo test --features mock` |
| E2E | Full daemon + browser + agent CLIs | Before merge | Real CLIs + manual browser check |
| Manual | UX verification (30-second test) | Before ship | Human |

---

## REQ Traceability Gate

Before `finishing-branch`, run every test above. Produce the matrix:

```
═══════════════════════════════════════════
  REQ TRACEABILITY — Triumvirate v2
═══════════════════════════════════════════

  PASS: __/26 REQs verified
  FAIL: __ REQs
  SKIP: __ REQs (manual — flagged for user)

  ✅ REQ-001 — N agents in browser...
  ...
═══════════════════════════════════════════
```

Gate rule: `finishing-branch` CANNOT proceed if any REQ is FAIL. SKIP (manual) requires user sign-off.
