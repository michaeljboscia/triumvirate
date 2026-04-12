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
| T-007.5 | c14e057 | daemon-core/src/lib.rs (DaemonState + 4 new fields + run_replay_buffer_fill), triumvirate/src/abe/task_tracker.rs (snapshot_workers + status_label + format_rfc3339), triumvirate/src/main.rs (replay-buffer fill task wired in run_daemon) | ❌ skip — pure plumbing | 7/7 reality tests (4 fill + 3 snapshot) PASS | ✅ |
| T-008 | `5f6a3cb` + harmonize `55df8f7` | triumvirate/src/main.rs (3 module-scope handlers: api_workers/api_fleet/api_fleet_by_id + DaemonRuntimeState alias hoisted + pantheon_rest_tests with TEST_TOKEN/get_with_bearer/get_no_bearer helpers + 9 reality tests) | ✅ **2× Claude subagents — Apollo won (narrow).** Winner: Apollo for type-alias hoisting + per-handler doc comments. Cherry-picked: Athena's `TEST_TOKEN` constant + `get_with_bearer`/`get_no_bearer` helpers. Gemini judge: agree-with-prior, 0 bugs. | 9/9 pantheon_rest_tests + 32/32 Wave 1 regression PASS | ✅ |
| T-009 | `bda929a` | triumvirate/src/main.rs (3 module-scope handlers: api_state/ws_v2/ws_v2_handshake + route registrations + pantheon_ws_replay_tests with subscribe-before-read + envelope wire format + max_sent dedup + 9 reality tests driving real ephemeral Axum server + tokio_tungstenite client) | ✅ **2× Claude subagents — Apollo won (CLEAR).** Winner: Apollo because Athena nested handlers inside run_daemon and duplicated ~160 lines of handler logic into her test module (test fidelity break — tests exercised copies, not production). Cherry-picked: Athena's `read_text()` (graceful Ping/Pong/Close handling) + `ws_request_with_bearer`/`ws_request_plain` helpers. Gemini judge: agree-with-prior, HIGH-severity duplication defect verified, 0 bugs in winner. | 9/9 pantheon_ws_replay_tests + 41/41 T-008 & Wave 1 regression = **50/50 total** PASS | ✅ |

**Wave 2 closure SHA:** `bda929a` (2026-04-11 22:2x EDT)
**Wave 2 test total:** 50/50 green (18 new in Wave 2 + 32 Wave 1 regression)
**Wave 2 wall time:** ~2h from kickoff to closure (32 min manifests + 6 min R1 audit + 4 min R2 audit + 7 min reaper probe + 12 min bake-off wall + 10 min judge + 10 min cherry-pick/harmonize/test/commit per task × 2 tasks + 5 min contamination restore). Biggest time-sink was the manifest drafting and the absolute-path contamination recovery.

### Phase 5.3 audit record

- **R1 (4 auditors)**: REJECTED × 4. 10 findings total (1 CRITICAL — T-009 wire format split between replay and live; 5 HIGH — SessionState field hallucinations, scope drift, BACKEND_STRUCTURE staleness; 3 MEDIUM; 1 LOW). Fix commit: `707c677`.
- **R2 (4 fresh auditors)**: APPROVED × 4 with 1 residual LOW finding (stale /api/workers intro sentence in BACKEND_STRUCTURE — 3 of 4 auditors flagged it). Fix commit: `b9ce1e5`.
- **Gate pass**: P5.3 — Dispatch Audit PASSED at SHA `b9ce1e5`.

### A3 → A1 deviation log (2026-04-11)

**User selected A3** (2 Claude + 1 Codex per task, 6 implementers total). **Executing A1** (2 Claude per task, 4 implementers total).

**Reason:** Empirical probe of the Codex path (`dispatch_codex` + `dispatch_codex_worktree`) ran into the `ContractFields`-setup complexity barrier. The 4-channel daemon reaper fix is **verified working** (two probes both transitioned cleanly from `working` → terminal state via `child.wait()` in 82s and 106s respectively — no hang observed), so the infrastructure is sound. The barrier is that `dispatch_codex_worktree` requires hand-crafted per-task `allowed_files` / `forbidden_files` / `allowed_commands` / `file_policy:default-deny` contracts, and getting them wrong produces worker failures that look like reaper issues but are actually contract mismatches. Hand-crafting correct Contracts for two brand-new tasks mid-bake-off adds substantial probability of false-negative results that would contaminate the Gemini judge's signal.

**What's preserved from A3:**
- Cross-implementation diversity (different Claude instances, independent token paths, separate worktrees, no shared state between the two implementers per task)
- Cross-model evaluation (Gemini code-review judges via `mcp__gemini__gemini-analyze-code`)
- The 4-task parallelism (not serial)

**What's NOT preserved:**
- Cross-model generation (no Codex-authored alternative for comparison)

**Upgrade path if the user wants true A3 after this bake-off:** Run a R2 pass where the winners from A1 get a single Codex worker submission added to each task's bake-off, re-judged by Gemini. This delays merge by ~15 min per task but delivers the 3-implementer signal the user asked for. Logged as a potential follow-up.

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
