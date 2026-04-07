# Fix Register — Triumvirate v2.2 Post-Rodeo Findings

**Source:** Layer 6 Semantic Logic Check (Gemini + Codex twin review)
**Date:** 2026-04-07
**Tests at time of audit:** 121/121 passing

---

## HIGH (Blockers — fix before ship)

### FIX-001: Compression coupled to ingestion (REQ-013 violation)

**File:** `ledger/src/ingest.rs:41`
**Bug:** `ingest_event` calls `process_pending_events(store)?` synchronously. Compression failure can fail event capture. Mutex held during batch processing starves ingestion under load.
**Spec says:** REQ-013: "Worker failure MUST NOT block or lose raw event capture."
**Fix:** Remove `process_pending_events` call from `ingest_event`. Compression must run only in the background Tokio task, never in the ingestion path. Ingestion returns immediately after INSERT.
**Reality test:** Ingest event while compression worker is deliberately paused/failing. Event must appear in DB. Currently fails.
**Found by:** Both twins independently.

### FIX-002: Review queue never advances (REQ-024a violation)

**File:** `peer-review/src/lib.rs:82`
**Bug:** `submit_review` marks a review as `done` but never promotes the next `pending` review to `in_progress`. Reviews past the inflight cap starve permanently.
**Spec says:** REQ-024a: "Reviews exceeding the cap are queued FIFO."
**Fix:** After marking a review `done`, `submit_review` must check for `pending` reviews and promote the oldest one to `in_progress` (up to inflight cap).
**Reality test:** Submit 3 reviews with max_inflight=1. First completes. Second should auto-promote to in_progress. Currently stays pending forever.
**Found by:** Both twins independently.

### FIX-003: Crash recovery doesn't reset tasks (REQ-034a violation)

**File:** `fleet/src/recovery.rs:35`
**Bug:** `recover_crashed_fleets` marks fleets as `failed` and cleans worktrees but never resets `in_progress`/`claimed` tasks to `pending` in the tasks table.
**Spec says:** REQ-034a: "reset `in_progress` tasks to `pending`"
**Fix:** Add `UPDATE tasks SET state = 'pending', assigned_agent = NULL WHERE fleet_id = ? AND state IN ('claimed', 'in_progress')` to recovery.
**Reality test:** Create fleet with tasks in `claimed` state. Run recovery. Tasks must be `pending`. Currently stays `claimed` forever.
**Found by:** Both twins independently.

### FIX-004: Global env var mutation for PROJECT_ROOT (race condition)

**File:** `fleet/src/orchestrator.rs:85`
**Bug:** Uses `std::env::set_var("TRIUMVIRATE_PROJECT_ROOT", ...)` which mutates global process state. Two concurrent fleets corrupt each other's root. This is UB in Rust.
**Spec says:** REQ-036: "Fleet members MUST have `TRIUMVIRATE_PROJECT_ROOT` set"
**Fix:** Pass `TRIUMVIRATE_PROJECT_ROOT` via `Command::new(...).env("TRIUMVIRATE_PROJECT_ROOT", project_root)` on the subprocess, not via global env.
**Reality test:** Spawn two fleets for different projects concurrently. Each agent must see its own project root. Currently race condition.
**Found by:** Gemini.

---

## MEDIUM (Acknowledgments — fix before v2.3)

### FIX-005: Fleet progress events incomplete (REQ-036 violation)

**File:** `fleet/src/orchestrator.rs`, `tasks.rs`, `merge.rs`
**Bug:** Only `fleet_spawned` event emitted. Missing: `agent_started`, `task_claimed`, `task_completed`, `merge_started`, `merge_result`, `fleet_done`.
**Fix:** Add ledger event emission at each lifecycle point.
**Found by:** Codex.

### FIX-006: Spool overflow doesn't trigger degraded (REQ-011 violation)

**File:** `ledger/src/health.rs:67-69`
**Bug:** Health computes spool size but never checks 100MB threshold for degraded status.
**Fix:** Add `if spool_size_bytes > 100_000_000 { status = "degraded" }` to health computation.
**Found by:** Codex.

### FIX-007: Fleet task file missing fields (REQ-037 violation)

**File:** `fleet/src/orchestrator.rs:73`
**Bug:** fleet-task.md has task_id/fleet_id/agent but missing `depends_on` frontmatter and prose description body.
**Fix:** Add depends_on field and task description to the generated markdown.
**Found by:** Codex.

### FIX-008: fleet_spawn missing wait parameter (REQ-035 violation)

**File:** `fleet/src/orchestrator.rs:13`
**Bug:** FleetSpawnRequest has no `wait` field. No blocking path implemented.
**Fix:** Add `wait: Option<bool>` to FleetSpawnRequest. When true, block until all agents running.
**Found by:** Codex.

### FIX-009: Session event_count drifts (REQ-008 semantic)

**File:** `ledger/src/ingest.rs:18-31`
**Bug:** Duplicate events (ON CONFLICT DO NOTHING) still increment session event_count. Counter inflates.
**Fix:** Only increment event_count when INSERT actually inserts (check changes() == 1).
**Found by:** Codex.
