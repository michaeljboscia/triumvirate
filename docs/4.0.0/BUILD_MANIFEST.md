# Pantheon v3.9.0 + v4.0.0 — BUILD MANIFEST

**Branch:** `v3.9.0`
**Base SHA (Wave 1 closure):** `f5396b9` (T-004 stdio-half closure commit)
**Pipeline:** `/goatrodeo` Phase 5 — Build Execution
**Bake-off pattern:** ON (per user mandate). Two parallel implementers per code task, Gemini code-review judges, winner cherry-picked, loser's standout bits backported. Skipped for trivial / docs-only / pure-plumbing tasks.

This manifest is **append-only** per the goatrodeo skill Phase 5.4 step 14. Every committed task adds a row to its wave's table. Do NOT rewrite history.

---

## Wave 0 — Decision Ledger / Contracts (already complete, prior session)

| Task | Commit | Files | Status |
|---|---|---|---|
| T-000 | (multiple) | specs/PANTHEON_V4.md, docs/4.0.0/* (11 canonical docs) | ✅ DONE — Phase 4.4 audited APPROVED by twins |

---

## Wave 1 — daemon-core primitives (v3.9.0) — CLOSED

| Task | Commit | Files | Tests | Status |
|---|---|---|---|---|
| T-001 | (Wave 1 batch) | shared-types/src/streaming.rs (WorkerLifecycle variant + WorkerLifecycleType) | streaming::tests 18/18 | ✅ |
| T-002 | (Wave 1 batch) | shared-types/src/api.rs (NEW — WorkersResponse, WorkerInfo, FleetResponse, FleetBuild, FleetTask, StateResponse, ReplayRequest, ReplayResponse) | api::tests 5/5 | ✅ |
| T-003 | (Wave 1 batch) | shared-types/src/lib.rs (SessionState gains parent/root/pantheon lineage fields) | session_state lineage tests 2/2 | ✅ |
| T-005 | (Wave 1 batch) | daemon-core/src/pid.rs (NEW — PidFile via libc::flock LOCK_EX\|LOCK_NB) | pid::tests 5/5 | ✅ |
| T-006 | (Wave 1 batch) | daemon-core/src/replay.rs (NEW — EventReplayBuffer + ReplayResult) | replay::tests 9/9 | ✅ |
| T-007 | (Wave 1 batch) | triumvirate/src/abe/task_tracker.rs (TaskTracker.with_pantheon_observability + emit_worker_lifecycle on register/completed/failed) | task_tracker::tests 13/13 | ✅ |
| T-004 (HTTP half) | (Wave 1 batch) | triumvirate/src/http_mcp.rs, daemon-core/src/pantheon_session.rs (NEW), mcp-tools/src/abe.rs (AbeTaskTracker.register signature widened with parent/root) | http_mcp::tests 6/6, pantheon_session::tests 5/5, lineage propagation tests 2/2 | ✅ |
| T-004 (stdio half) | f5396b9 | triumvirate/src/main.rs (replaced #[tool_handler] with hand-rolled ServerHandler impl + extract_pantheon_scope_from_meta + 10 reality tests) | pantheon_stdio_meta_tests 10/10 | ✅ |
| T-010 | — | (none — absorbed into T-004 dual-transport scope) | n/a | ✅ ABSORBED |

**Wave 1 cumulative test count:** 75/75 reality tests pass. `cargo check --workspace --tests` clean.

---

## Wave 2 — Daemon HTTP (v3.9.0) — IN PROGRESS

**Phase 4.4 misses caught by Phase 5.3 empirical verification (2026-04-11):**

1. T-006 built `EventReplayBuffer` but no task wired it into `ws_events`. Fix: T-007.5 added.
2. `TaskTracker` had no enumerator; T-008 needs `Vec<WorkerInfo>`. Fix: T-007.5 adds `snapshot_workers()`.
3. `fleet_states` lives only on `McpBridge`, not `DaemonState`; T-008 cannot read it. Fix: T-007.5 adds `fleet_v2_states` to `DaemonState`.
4. IMPLEMENTATION_PLAN.md spelled `/api/fleet/:build_id` (axum 0.7 syntax). axum 0.8 panics on `:id`. Fix: docs/4.0.0/IMPLEMENTATION_PLAN.md updated to `/api/fleet/{build_id}` inline.
5. T-009's WS handshake requires client→server message handling that the existing `daemon-http::ws_route` doesn't support. New route `/ws/v2` will be added by T-009 alongside the legacy `/ws` (zero regressions for `triumvirate watch`).

| Task | Commit | Files | Bake-off | Tests | Status |
|---|---|---|---|---|---|
| T-007.5 | _pending_ | daemon-core/src/lib.rs (DaemonState + new fields), triumvirate/src/abe/task_tracker.rs (snapshot_workers), triumvirate/src/main.rs (replay-buffer fill task) | ❌ skip — pure plumbing | _pending_ | 🟡 in progress |
| T-008 | _pending_ | triumvirate/src/main.rs (3 new GET routes + 3 handlers + 3 reality tests) | ✅ 2x Claude subagents | _pending_ | ⏸ blocked on T-007.5 |
| T-009 | _pending_ | triumvirate/src/main.rs (1 new GET route, 1 new WS route, replay-aware handshake) | ✅ 2x Claude subagents | _pending_ | ⏸ blocked on T-007.5 |

---

## Wave 3+ — not yet started

| Wave | Tasks | Status |
|---|---|---|
| Wave 3 | T-011 (integration test), T-012 (v3.9.0 release) | not started |
| Wave 4 | T-013 (Tauri scaffold) | not started |
| Wave 5 | T-014–T-018 (Tauri features) | not started |
| Wave 6 | T-019–T-023 | not started |
| Wave 7 | T-024–T-026 | not started |
| Wave 8 | T-027–T-029 | not started |

---

## Bake-off result schema (per task, when applicable)

```
Task: T-NNN
  Implementer A: <name>           (worktree: <path>)
  Implementer B: <name>           (worktree: <path>)
  Gemini judge:  <verdict + 1-line rationale>
  Winner:        <A|B>
  Loser bits backported: <none|list>
  Final SHA:     <hash>
```
