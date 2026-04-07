# Codex Bugfix Handoff — Triumvirate v2.2 Post-Rodeo Fixes

**Branch:** `feat/mcp-first` at `0139710`
**Date:** 2026-04-07
**Source:** `daemon/docs/v2.2/FIX_REGISTER.md` + GitHub issues #1–#9

---

## Read First

1. `daemon/docs/v2.2/FIX_REGISTER.md` — full bug details, file paths, line numbers, proposed fixes
2. `daemon/docs/v2.2/SPEC.md` — the REQs each bug violates

## Fix Order (HIGH first, then MEDIUM)

### FIX-001 — Issue #1: Decouple compression from ingestion

**File:** `ledger/src/ingest.rs:41`
**Do:** Remove the `process_pending_events(store)?` call from `ingest_event`. Ingestion must return immediately after INSERT. Compression runs ONLY in the background Tokio task.
**Test:** Add test: ingest event while compression is deliberately broken (mock failure). Event must appear in DB. Current code fails this.
**Commit:** `fix(ledger): decouple compression from ingestion path (fixes #1)`

### FIX-002 — Issue #2: Review queue promotion

**File:** `peer-review/src/lib.rs:82`
**Do:** After `submit_review` marks a review `done`, query for the oldest `pending` review. If inflight < max_inflight, promote it to `in_progress`. Return the promoted review_id.
**Test:** Add test: submit 3 reviews with max_inflight=1. Complete first. Assert second auto-promotes to `in_progress`. Complete second. Assert third promotes.
**Commit:** `fix(peer-review): promote pending reviews on slot free (fixes #2)`

### FIX-003 — Issue #3: Reset tasks during crash recovery

**File:** `fleet/src/recovery.rs:35`
**Do:** Add after the fleet state update:
```sql
UPDATE tasks SET state = 'pending', assigned_agent = NULL
WHERE fleet_id = ? AND state IN ('claimed', 'in_progress')
```
**Test:** Update existing `recovery_marks_failed_cleans_worktrees_and_logs_event` test: create tasks in `claimed` and `in_progress` state before recovery. Assert they reset to `pending` with NULL agent. Assert `done` tasks are unchanged.
**Commit:** `fix(fleet): reset orphaned tasks during crash recovery (fixes #3)`

### FIX-004 — Issue #4: Subprocess env instead of global set_var

**File:** `fleet/src/orchestrator.rs:85`
**Do:** Remove `std::env::set_var("TRIUMVIRATE_PROJECT_ROOT", ...)`. Instead, when spawning each fleet member subprocess, use `Command::new(...).env("TRIUMVIRATE_PROJECT_ROOT", &req.project_root)`.
**Test:** Add test: spawn two fleet requests for different project roots concurrently. Assert each subprocess receives its own correct PROJECT_ROOT (not the other's). grep orchestrator.rs for `set_var` — must return zero matches.
**Commit:** `fix(fleet): use subprocess env for PROJECT_ROOT instead of global set_var (fixes #4)`

### FIX-005 — Issue #5: Fleet progress events

**Files:** `fleet/src/orchestrator.rs`, `fleet/src/tasks.rs`, `fleet/src/merge.rs`
**Do:** Add `ledger.record()` calls at each lifecycle point:
- `orchestrator.rs`: `agent_started` when subprocess spawns, `fleet_done` when all merges complete
- `tasks.rs`: `task_claimed` on successful claim, `task_completed` on task done
- `merge.rs`: `merge_started` before merge attempt, `merge_result` after (success or conflict)
**Test:** Add test: run fleet lifecycle. Assert Ledger contains all 6 event types.
**Commit:** `fix(fleet): emit all lifecycle progress events to ledger (fixes #5)`

### FIX-006 — Issue #6: Spool overflow health check

**File:** `ledger/src/health.rs:69`
**Do:** Add to the status determination logic:
```rust
if spool_size_bytes > 100_000_000 {
    status = "degraded".to_string();
}
```
**Test:** Add test: mock spool dir with >100MB total size. Assert health returns `degraded`.
**Commit:** `fix(ledger): trigger degraded health on spool overflow >100MB (fixes #6)`

### FIX-007 — Issue #7: Fleet task file fields

**File:** `fleet/src/orchestrator.rs:73`
**Do:** Add `depends_on` field to frontmatter. Add task description as prose body below the `---` frontmatter separator. Format:
```markdown
---
task_id: T-001
fleet_id: fleet-abc
assigned_agent: claude-1
depends_on: []
---

Build the authentication middleware. Handle JWT validation,
token refresh, and session management.
```
**Test:** Add test: spawn fleet. Read generated fleet-task.md. Assert frontmatter contains `depends_on`. Assert body is non-empty.
**Commit:** `fix(fleet): include depends_on and prose in fleet-task.md (fixes #7)`

### FIX-008 — Issue #8: fleet_spawn wait parameter

**File:** `fleet/src/orchestrator.rs:13`
**Do:** Add `wait: Option<bool>` to `FleetSpawnRequest`. When `wait` is `Some(true)`, block (await) until all worktrees are created and all agent subprocesses are running before returning the response.
**Test:** Add test: `fleet_spawn(wait=true)` returns only after agents are running. `fleet_spawn(wait=false)` returns immediately with `state=spawning`.
**Commit:** `fix(fleet): add wait parameter to fleet_spawn (fixes #8)`

### FIX-009 — Issue #9: Session event_count drift

**File:** `ledger/src/ingest.rs:18-31`
**Do:** After the INSERT OR IGNORE, check `conn.changes() == 1` (rusqlite). Only increment `sessions.event_count` if the row was actually inserted (changes == 1). If 0, the insert was a duplicate — don't increment.
**Test:** Add test: ingest same event twice. Assert `sessions.event_count` is 1, not 2.
**Commit:** `fix(ledger): only increment event_count on actual insert (fixes #9)`

---

## Rules

- Fix in the order listed (HIGHs 1-4 first)
- Each fix gets its own commit with `fixes #N` to auto-close the GitHub issue
- Run `cargo test --workspace` after EACH fix — all 121+ tests must pass
- Add new tests for each fix (the test column above)
- Do NOT modify the spec, implementation plan, or any doc file — code only

## When Done

Run `cargo test --workspace && cargo clippy --workspace`. All green = ready for re-audit.
