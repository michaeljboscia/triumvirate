# Autonomous Build Enforcement — Orchestration Flow

**Version:** Triumvirate v3.0
**Spec:** `specs/AUTONOMOUS_BUILD_ENFORCEMENT.md`
**PRD:** `docs/abe/PRD.md`

---

## Entry Points

There are exactly three ways a human interacts with ABE:

### 1. Start a Build

```
Human: "Build this" (provides IMPLEMENTATION_PLAN.md)
  │
  ├─ Claude reads IMPLEMENTATION_PLAN.md
  ├─ Claude reads Execution Contract appendix
  ├─ Claude creates BUILD_STATE.json (build_id, plan_path, build_started_at)
  ├─ Claude validates wave structure:
  │   ├─ No-overlap invariant (no shared allowed_files within a wave)
  │   ├─ All task XML blocks have 8 mandatory fields
  │   ├─ All <reality_test> fields are present
  │   └─ Wave 0 is contracts/interfaces only
  │
  ├─ Claude says: "Build starting. {N} tasks across {M} waves. max_parallel: {P}."
  └─ Enters Orchestration Loop (see below)
```

### 2. Resume a Build

```
Human: "Resume" (after session crash, rate limit, or manual stop)
  │
  ├─ Claude reads BUILD_STATE.json
  │   ├─ Identifies current_wave, tasks_completed, tasks_running, tasks_remaining
  │   └─ Reads build_started_at for elapsed wall-clock
  │
  ├─ Claude calls get_task_status for each task in tasks_running
  │   ├─ If completed → move to tasks_completed, process result
  │   ├─ If failed → move to tasks_failed, enter failure handling
  │   └─ If no process alive → crash recovery:
  │       ├─ Save .triumvirate/interrupted.patch
  │       ├─ Quarantine dirty worktree
  │       └─ Redispatch fresh from last stable SHA
  │
  ├─ Claude reads BUILD_MANIFEST.md + DEVIATION_LOG.md for full context
  └─ Continues Orchestration Loop at first incomplete task
```

### 3. Review Results

```
Human returns after build completes (or escalation halted it)
  │
  ├─ Reads BUILD_MANIFEST.md — what was built, commit SHAs, validation status
  ├─ Reads DEVIATION_LOG.md — every failure, repair, Gemini concern
  ├─ Reads BUILD_STATE.json — final tallies
  │
  ├─ If build completed:
  │   └─ End-of-execution report in contract format
  │       backlog_status: 0 remaining
  │       completed_tasks: [T-001, T-002, ...]
  │       total_commits: {N}
  │       collateral_fixes: {N}
  │       validation: {N}/{N} tasks passed
  │       test_suite: npm test → {pass/fail}
  │
  └─ If build halted (escalation):
      └─ Escalation payload with:
          - Which task failed and why
          - All briefings Claude generated
          - Gemini's diagnosis
          - Failure class and retry count
```

---

## The Orchestration Loop

This is the core runtime. It runs after "Start" or "Resume."

```
FOR EACH WAVE (sequential):
│
├─ PRE-WAVE: Validate no-overlap invariant
│
├─ FOR EACH TASK IN WAVE (concurrent, max_parallel cap):
│   │
│   ├─ STEP 1: BRIEFING + CONTRACT GENERATION
│   │   Claude generates:
│   │   - briefing_content (markdown string)
│   │   - contract_fields (JSON object)
│   │   Claude does NOT write files to disk.
│   │
│   ├─ STEP 2: DISPATCH (single MCP call)
│   │   dispatch_codex_worktree(sha, briefing_content, contract_fields)
│   │   Returns: task_id (non-blocking)
│   │   Daemon atomically: worktree → .triumvirate/ → files → hooks → spawn
│   │   On setup failure: daemon rolls back, returns clean error
│   │
│   ├─ STEP 3: MONITOR
│   │   Poll get_task_status(task_id) until:
│   │   - completed → proceed to STEP 4
│   │   - failed → proceed to STEP 4
│   │   - timeout (task_timeout_sec exceeded) → daemon SIGTERMs worker
│   │
│   ├─ STEP 4: VALIDATE
│   │   Worker ran validate-task.sh post-commit
│   │   Claude reads .triumvirate/VALIDATION_LOG.md from worktree
│   │
│   ├─ STEP 5: IF BLOCKED → Failure Handling
│   │   │
│   │   ├─ 5a. MECHANICAL CLASSIFICATION
│   │   │   Evidence from validate-task.sh output:
│   │   │   - Stub markers / test failures → worker-error
│   │   │   - Hook blocked needed file → contract-error
│   │   │   - Missing binary / sandbox error → environment-error
│   │   │   - Everything else → orchestrator-briefing-error (→ Gemini)
│   │   │
│   │   ├─ 5b. Log to DEVIATION_LOG.md
│   │   │
│   │   ├─ 5c. Query Gemini (with briefing + contract on failure)
│   │   │
│   │   ├─ 5d. Generate repair briefing (rigid template, size-capped)
│   │   │
│   │   ├─ 5e. Redispatch with repair briefing
│   │   │
│   │   └─ RETRY LIMITS:
│   │       worker-error: max 3
│   │       contract-error: max 2
│   │       orchestrator-briefing-error: max 2
│   │       total across all classes: max 5
│   │       environment-error: HALT immediately (0 retries)
│   │       On cap breach → ESCALATE TO HUMAN
│   │
│   ├─ STEP 6: IF PASS → Record Success
│   │   ├─ 6a. Write AFTER_ACTION.md
│   │   ├─ 6b. Append to BUILD_MANIFEST.md
│   │   ├─ 6c. Append to DEVIATION_LOG.md (even if clean)
│   │   ├─ 6d. Query Gemini blind review (no briefing on pass)
│   │   └─ 6e. Update BUILD_STATE.json
│   │
│   └─ STEP 7: CONTEXT DIES
│       Codex session gone. Worktree kept or deleted.
│       Only surviving artifacts: git commits + orchestrator records.
│
├─ POST-WAVE: Wave Boundary Gate
│   ├─ Collect all validation results
│   ├─ Gemini reviews all wave code
│   ├─ Full test suite on merged state
│   ├─ If any failure → HALT, escalate
│   ├─ Write wave summary to BUILD_MANIFEST.md
│   └─ Update BUILD_STATE.json (advance current_wave)
│
└─ NEXT WAVE (or END OF BUILD)
```

---

## Escalation Priority Order

When multiple conditions are true simultaneously, highest priority wins:

```
1. environment-error / daemon unreachable     → HALT immediately
2. Wave boundary gate fails                   → HALT immediately
3. Hard retry cap reached (5 per task)        → HALT immediately
4. orchestrator-briefing-error                → Fix briefing, redispatch
5. contract-error                             → Fix contract, redispatch
6. worker-error                               → Repair briefing, redispatch
7. Gemini advisory concern                    → Log, incorporate into next briefing
8. Normal pass                                → Continue
```

---

## Data Flow

```
IMPLEMENTATION_PLAN.md (input — read-only)
       │
       ▼
Claude (orchestrator)
       │
       ├─── generates ──→ briefing_content + contract_fields
       │                         │
       │                         ▼
       ├─── MCP call ───→ dispatch_codex_worktree (daemon)
       │                         │
       │                         ├─── creates ──→ .triumvirate/BRIEFING.md
       │                         ├─── creates ──→ .triumvirate/contract.json
       │                         ├─── copies  ──→ .triumvirate/validate-task.sh
       │                         ├─── installs ─→ .triumvirate/hooks/pre-commit
       │                         └─── spawns  ──→ Codex worker
       │                                              │
       │                                              ├─── reads BRIEFING.md
       │                                              ├─── reads files from worktree
       │                                              ├─── writes code (allowed_files only)
       │                                              ├─── commits (pre-commit hook validates)
       │                                              └─── validate-task.sh runs post-commit
       │                                                        │
       │                                                        ▼
       │                                              .triumvirate/VALIDATION_LOG.md
       │                                                        │
       ▼                                                        │
Claude reads VALIDATION_LOG.md ◄────────────────────────────────┘
       │
       ├─── writes ──→ BUILD_MANIFEST.md (append-only)
       ├─── writes ──→ DEVIATION_LOG.md (append-only)
       ├─── writes ──→ BUILD_STATE.json (checkpoint)
       ├─── writes ──→ AFTER_ACTION.md (per-task)
       └─── queries ─→ query_gemini_review (Gemini auditor)
                              │
                              └─── returns: concerns / clean
```

---

## Error States

| State | What Happened | What the Human Sees |
|-------|--------------|-------------------|
| **DAEMON_UNAVAILABLE** | Daemon process is down | "Daemon unreachable. Attempted auto-restart. Still down. Build halted." |
| **SETUP_FAILED** | Worktree creation or hook install failed | "dispatch_codex_worktree failed: {error}. Worktree rolled back." |
| **TASK_TIMEOUT** | Worker exceeded task_timeout_sec | "T-003 timed out after 600s. Worker killed. Retrying." |
| **VALIDATION_BLOCKED** | validate-task.sh exit 1 | "T-003 BLOCKED: {stub markers / test failures}. Classified as {class}. Retrying." |
| **RETRY_EXHAUSTED** | 5 attempts across all failure classes | "T-003 failed 5 times. Escalating. All briefings attached." |
| **WAVE_GATE_FAILED** | Merged wave state fails tests | "Wave 2 boundary gate FAILED. Build halted. Wave summary in manifest." |
| **SESSION_DIED** | Claude rate-limited or crashed | BUILD_STATE.json on disk. Human says "resume" in new session. |
| **BUILD_COMPLETE** | All tasks done, all validations pass | End-of-execution report. /postrodeo runs. |
