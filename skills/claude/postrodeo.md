# Post Rodeo — Full-Weight Build Retrospective

**Skill:** `/postrodeo`

**Purpose:** After a build completes, audit what was built against what was specced. Produce the COMPLETION_MATRIX, analyze deviations, run twin review on code diffs, execute Layer 6 semantic logic check, and generate a retrospective report with lessons and process metrics.

**Philosophy:** The goatrodeo asks "is the spec any good?" The postrodeo asks "did we build what the spec said?" Same rigor, opposite direction. Evidence over assertions.

**Pipeline position:** Runs AFTER `superpowers:verification-before-completion` (L1-L3) and BEFORE `superpowers:finishing-a-development-branch`. This is the L4 gate — no shipping without it.

```
/goatrodeo → spec (REQ-IDs)
/uncompromising-executor → 8 canonical docs + TEST_PLAN.md
superpowers:writing-plans → implementation plan (FEAT-IDs, task XML)
superpowers:executing-plans → build (produces BUILD_MANIFEST.md, DEVIATION_LOG.md)
superpowers:verification-before-completion → L1-L3 (tests pass, build compiles)
  ↓
/postrodeo → L4 audit (this skill)
  ↓
superpowers:finishing-a-development-branch → merge/PR (gated by postrodeo)
```

---

## Required Input Artifacts

The postrodeo consumes artifacts produced by earlier stages. If any are missing, it says so and tells you which skill produces them.

| Artifact | Produced by | Required? |
|----------|-------------|-----------|
| `TEST_PLAN.md` | `/uncompromising-executor` | **YES** — cannot run without it |
| `IMPLEMENTATION_PLAN.md` | `/uncompromising-executor` | **YES** — cannot run without it |
| `BUILD_MANIFEST.md` | `superpowers:executing-plans` | Recommended — degraded mode without it |
| `DEVIATION_LOG.md` | `superpowers:executing-plans` | Recommended — degraded mode without it |
| `policy-rules.yml` | `/uncompromising-executor` | Optional — policy scan skipped without it |
| Git history | Git | **YES** — always available |

**Degraded mode:** If BUILD_MANIFEST or DEVIATION_LOG don't exist, the postrodeo reconstructs what it can from git history. It flags the gap: "BUILD_MANIFEST not found — reconstructing from git log. Fidelity reduced."

---

## Activation

When this skill is invoked:

1. Locate the project's doc suite (search for `TEST_PLAN.md`, `IMPLEMENTATION_PLAN.md` in the project)
2. Locate build artifacts (`BUILD_MANIFEST.md`, `DEVIATION_LOG.md`)
3. Verify L1-L3 verification has been run (tests pass, build compiles)
4. Print status:

```
═══════════════════════════════════════════
  POST RODEO — LOADING
═══════════════════════════════════════════
  Project: [name]
  TEST_PLAN.md: ✅ found ([N] REQ-IDs)
  IMPLEMENTATION_PLAN.md: ✅ found ([N] phases, [M] tasks)
  BUILD_MANIFEST.md: ✅ found ([N] task entries)
  DEVIATION_LOG.md: ✅ found ([N] deviations)
  policy-rules.yml: ✅ found ([N] rules)
  Git history: [N] commits on branch

  Running Phase 1...
═══════════════════════════════════════════
```

If TEST_PLAN.md or IMPLEMENTATION_PLAN.md is missing: **STOP.** Print what's missing and which skill to run. Do not proceed.

**Argument:** `/postrodeo [path-to-doc-suite]` — if provided, look for docs at that path. If omitted, search the current project for canonical docs.

---

## Phase 1: Completion Matrix

**Purpose:** For every REQ-ID in TEST_PLAN.md, determine if it was implemented and if the test passes.

### Step 1.1: Run Tests

Execute the project's test suite. Capture full output including individual test results.

```bash
cargo test 2>&1  # or npm test, pytest, go test ./...
```

### Step 1.2: Map Results to REQ-IDs

For each row in TEST_PLAN.md:

1. **Find the test:** Look for a test function containing the REQ-ID in its name (e.g., `test_req_003_*`). If no test exists with the REQ-ID in its name, search for the test described in the "Pass Condition" column.
2. **Check the result:** Did the test pass, fail, or not exist?
3. **Check implementation:** Does the code for this REQ exist? (Search for REQ-ID in code comments, or check BUILD_MANIFEST for the task that implemented it.)

### Step 1.3: Produce the Matrix

```
═══════════════════════════════════════════
  COMPLETION MATRIX — [Feature Name]
═══════════════════════════════════════════

  PASS: 14/17 REQs verified
  FAIL: 2 REQs
  SKIP: 1 REQ (manual test — flagged for user)
  UNIMPLEMENTED: 0 REQs

  ✅ REQ-001 — Postgres schema created
     Test: test_req_001_schema_exists — PASS
     Files: src/db/schema.rs (T-001, Wave 1)

  ✅ REQ-002 — REST endpoints responding
     Test: test_req_002_endpoints — PASS
     Files: src/api/routes.rs (T-003, Wave 2)

  ❌ REQ-007 — CSV export missing 2 columns
     Test: test_req_007_csv_export — FAIL
     Expected: 12 columns matching display
     Actual: 10 columns (missing: created_at, updated_at)
     Files: src/export/csv.rs (T-008, Wave 3)

  ❌ REQ-011 — Alert not firing after 3 missed refreshes
     Test: test_req_011_alert_trigger — FAIL
     Expected: Alert after 3 missed refreshes
     Actual: No alert fired after 5 missed refreshes
     Files: src/monitor/alerts.rs (T-012, Wave 4)

  ⏭️  REQ-015 — Requires manual OAuth flow test
     Test type: manual
     Action required: User must test OAuth flow manually

═══════════════════════════════════════════
```

**Gate rule:** If ANY REQ shows ❌ FAIL, print:
```
⛔ COMPLETION MATRIX HAS FAILURES — cannot proceed to Phase 2 until resolved.
Fix the failing tests, then re-run /postrodeo.
```

User can override with "continue anyway" — this is logged as a waiver.

If ANY REQ shows ⚠️ UNIMPLEMENTED, this is a **BLOCKER**. The spec said to build it. It wasn't built. This cannot be waived without explicit justification.

---

## Phase 2: Deviation Analysis

**Purpose:** Compare what was planned vs what was actually done. Identify where the build diverged from the implementation plan and whether those divergences were documented.

### Step 2.1: Plan-to-Build Diff

For each phase/task in IMPLEMENTATION_PLAN.md:

1. **Was it completed?** Check BUILD_MANIFEST or git history for matching work.
2. **Was it completed as specified?** Compare the planned files/approach with the actual files/approach.
3. **Was it completed in order?** Check if wave ordering was respected.

### Step 2.2: Deviation Log Audit

If DEVIATION_LOG.md exists:

1. **Are all deviations documented?** Compare the plan-to-build diff (Step 2.1) with the logged deviations. Any divergence NOT in the log is an **undocumented deviation**.
2. **Are deviations justified?** Each deviation should have a "Why" entry. Flag any without justification.
3. **Were affected REQs updated?** Each deviation should list affected REQ-IDs. Cross-reference with COMPLETION_MATRIX — did those REQs still pass?

### Step 2.3: Produce Deviation Report

```
═══════════════════════════════════════════
  DEVIATION ANALYSIS
═══════════════════════════════════════════

  Planned tasks: 24
  Completed as planned: 20
  Documented deviations: 3
  Undocumented deviations: 1 ⚠️

  DOCUMENTED:
  DEV-001: T-005 changed from REST to WebSocket
    Why: Latency requirement incompatible with polling
    REQs affected: REQ-007, REQ-012
    REQ status: REQ-007 ❌ FAIL, REQ-012 ✅ PASS
    ⚠️ Deviation introduced a failure — REQ-007 needs attention

  DEV-002: T-011 added (not in original plan)
    Why: Discovered missing error handling during Wave 3
    REQs affected: REQ-019 (new)
    REQ status: REQ-019 ✅ PASS

  DEV-003: T-015 approach changed
    Why: Upstream API schema different from spec
    REQs affected: REQ-003
    REQ status: REQ-003 ✅ PASS

  UNDOCUMENTED:
  ⚠️ T-009 was skipped — no DEVIATION_LOG entry
    Planned: "Implement rate limiting middleware"
    Actual: Not present in BUILD_MANIFEST or git history
    REQ affected: REQ-014
    REQ status: REQ-014 ⚠️ UNIMPLEMENTED

═══════════════════════════════════════════
```

Undocumented deviations are **warnings**, not blockers. But they're surfaced prominently.

---

## Phase 3: Git Forensics

**Purpose:** Use git history as an independent record of what actually happened during the build. Cross-reference with BUILD_MANIFEST and IMPLEMENTATION_PLAN.

### Step 3.1: Commit Analysis

```bash
git log --oneline --no-merges base-branch..HEAD
```

For each commit:
1. **Does it reference a FEAT-ID?** Commits should reference FEAT-IDs per project rules.
2. **Does the changed file set match a planned task?** Map commits to tasks in the implementation plan.
3. **Are there orphan commits?** Work done that doesn't map to any task or REQ-ID.

### Step 3.2: File Coverage

```bash
git diff --stat base-branch..HEAD
```

1. **Files planned but not touched:** Files listed in IMPLEMENTATION_PLAN that have no commits.
2. **Files touched but not planned:** Files modified that aren't in any task. Could be legitimate (bug fixes, refactoring) or scope creep.

### Step 3.3: Produce Git Forensics Report

```
═══════════════════════════════════════════
  GIT FORENSICS
═══════════════════════════════════════════

  Total commits: 47
  Commits with FEAT-ID: 39 (83%)
  Orphan commits: 8 (no FEAT-ID or task reference)
  
  Orphan commits:
    abc1234 — "fix typo in README"
    def5678 — "update Cargo.lock"
    ghi9012 — "refactor error handling" ← ⚠️ no task
    ...

  Files planned but not touched: 2
    src/middleware/rate_limit.rs (T-009, skipped)
    src/export/pdf.rs (T-018, skipped)

  Files touched but not planned: 5
    src/utils/retry.rs ← ⚠️ not in any task
    Cargo.lock (expected — dependency changes)
    .gitignore (expected — config)
    ...

═══════════════════════════════════════════
```

---

## Phase 4: Twin Review of Deviations

**Purpose:** Send the deviations and their justifications to both twins for independent review. Are the deviations reasonable? Did they introduce risk?

### Step 4.1: Package for Twins

Dispatch to both twins via daemon pattern (`spawn_session` → `ask_session`):

**Package:**
- DEVIATION_LOG.md (or reconstructed deviations from Phase 2)
- The original spec sections for affected REQ-IDs
- The COMPLETION_MATRIX results for affected REQ-IDs
- Git diff for the deviated tasks

**Prompt:**
> Review these deviations from the implementation plan. For each deviation:
> 1. Was the justification reasonable given the constraints?
> 2. Did the deviation introduce risk to other REQ-IDs?
> 3. Are there downstream consequences the team should watch for?
> 4. Would you have made the same call? If not, what would you have done?
>
> Be specific. Reference REQ-IDs and file paths.

### Step 4.2: Synthesize Twin Responses

For each deviation:
- **Both twins agree the deviation was reasonable:** Log as validated.
- **One or both twins flag risk:** Surface to user with the twin's reasoning.
- **Twins disagree:** Surface both perspectives.

Present results inline — do not require user action unless risk was flagged.

---

## Phase 5: Layer 6 Semantic Logic Check

**Purpose:** After all tests pass and deviations are reviewed, have a separate model review the actual code diff for logical correctness. Code can compile, pass tests, and still not do what the spec intended.

### Step 5.1: Prepare the Diff

```bash
git diff base-branch..HEAD -- '*.rs' '*.ts' '*.py' '*.go'  # Source files only
```

Exclude: lock files, generated code, test fixtures, config files.

### Step 5.2: Dispatch Semantic Review

Send to BOTH twins (this is a separate review from Phase 4 — Phase 4 reviews deviations, Phase 5 reviews the actual code):

**Package:**
- The full source code diff
- The original spec (REQ-IDs and acceptance criteria)
- COMPLETION_MATRIX (so they know what passed/failed)

**Prompt:**
> You are performing a Layer 6 semantic logic check. All tests pass. The code compiles. Your job is to find logical errors that tests didn't catch.
>
> Review this code diff against the spec. For each changed file, answer:
> 1. Does the implementation match the spec's INTENT, not just its letter?
> 2. Are there edge cases the tests don't cover but the spec implies?
> 3. Are there race conditions, ordering issues, or state management bugs?
> 4. Does the error handling actually handle the errors the spec describes?
> 5. Are there security implications the spec didn't anticipate?
>
> Only flag issues where you have specific evidence. "This might be a problem" is not a finding. "Line 42 of foo.rs handles timeout but doesn't retry, contradicting REQ-007 which says 'retry up to 3 times'" IS a finding.

### Step 5.3: Triage Findings

For each finding from either twin:
- **Both twins found the same issue:** High confidence — surface as a finding.
- **One twin found it:** Medium confidence — surface with caveat.
- **Finding contradicts a passing test:** Low confidence — note but don't block.

Present findings:

```
═══════════════════════════════════════════
  LAYER 6 — SEMANTIC LOGIC CHECK
═══════════════════════════════════════════

  Findings: 3 (2 high confidence, 1 medium)

  🔴 HIGH: REQ-007 retry logic incomplete
     src/client/retry.rs:42 — handles timeout but doesn't
     retry. REQ-007 specifies "retry up to 3 times with
     exponential backoff."
     Both twins flagged this independently.

  🔴 HIGH: REQ-012 race condition in concurrent updates
     src/db/writes.rs:88 — two concurrent writes to the same
     row can interleave. No transaction or lock.
     Both twins flagged this independently.

  🟡 MEDIUM: REQ-019 error message doesn't match spec
     src/api/errors.rs:15 — returns "Internal Server Error"
     but REQ-019 specifies "descriptive error with retry hint."
     Flagged by Gemini only. Codex didn't flag.

  VERDICT: 2 high-confidence findings require attention.
═══════════════════════════════════════════
```

**Gate rule:** High-confidence findings are **warnings**, not blockers. The user decides whether to fix before shipping. But they must acknowledge each one — no silent pass-through.

---

## Phase 6: Retrospective Report

**Purpose:** Synthesize everything into a single retrospective document. This is the artifact that enables process improvement.

### Step 6.1: Generate Report

Write `RETROSPECTIVE.md` to the project's doc suite:

```markdown
# Retrospective — [Feature Name]

**Date:** [date]
**Spec:** [path to spec]
**Branch:** [branch name]
**Commits:** [N] commits, [date range]

## Completion Summary

| Metric | Value |
|--------|-------|
| REQs specified | [N] |
| REQs passing | [N] |
| REQs failing | [N] |
| REQs skipped (manual) | [N] |
| REQs unimplemented | [N] |
| Completion rate | [%] |

## Deviation Summary

| Metric | Value |
|--------|-------|
| Planned tasks | [N] |
| Completed as planned | [N] |
| Documented deviations | [N] |
| Undocumented deviations | [N] |
| Twin-validated deviations | [N] |
| Twin-flagged deviations | [N] |

## Semantic Review Summary

| Metric | Value |
|--------|-------|
| Findings total | [N] |
| High confidence | [N] |
| Medium confidence | [N] |
| Acknowledged by user | [N] |

## Git Health

| Metric | Value |
|--------|-------|
| Total commits | [N] |
| FEAT-ID coverage | [%] |
| Orphan commits | [N] |
| Planned files not touched | [N] |
| Unplanned files touched | [N] |

## Lessons Learned

[For each deviation, finding, or failure — what caused it and what would prevent it next time]

## Process Recommendations

[Based on the data — what should change about the process for the next build]
```

### Step 6.2: Policy Scan (if policy-rules.yml exists)

Run the policy scanner against the full diff:

```bash
policy-scanner --layer ci --files $(git diff --name-only base-branch..HEAD) --rules .claude/policy-rules.yml
```

Append results to RETROSPECTIVE.md.

### Step 6.3: Update LESSONS.md

If the project has a LESSONS.md, append lessons from the retrospective. Each lesson includes:
- What went wrong (or right)
- Why it happened
- The rule that prevents it (or reinforces it) next time

### Step 6.4: Process Metrics (if available)

If token usage data is available (from Flow State daemon, `/usage` command, or twin session stats):

```markdown
## Process Metrics

| Model | Tokens | Cost Estimate |
|-------|--------|---------------|
| Claude (Opus 4.6) | ~[N] | ~$[X] |
| Gemini (via daemon) | ~[N] | $0 (CLI) |
| Codex (via daemon) | ~[N] | $0 (CLI) |
| Total | ~[N] | ~$[X] |
```

If not available, print: "Token tracking not available. See Gap 4 in PROCESS_MATURITY_GAPS.md."

---

## Final Verdict

After all 6 phases complete, print the final verdict:

```
═══════════════════════════════════════════
  POST RODEO — FINAL VERDICT
═══════════════════════════════════════════

  Phase 1 — Completion Matrix:    ✅ 14/17 REQs pass
  Phase 2 — Deviation Analysis:   ⚠️ 1 undocumented deviation
  Phase 3 — Git Forensics:        ✅ 83% FEAT-ID coverage
  Phase 4 — Twin Review:          ✅ All deviations validated
  Phase 5 — Semantic Logic Check: ⚠️ 2 findings acknowledged
  Phase 6 — Retrospective:        ✅ Report written

  VERDICT: SHIP WITH ACKNOWLEDGMENTS
  
  Acknowledged items:
  - REQ-015: manual test required (user sign-off needed)
  - 2 semantic findings acknowledged but not fixed
  - 1 undocumented deviation logged

  RETROSPECTIVE.md written to [path]

  Ready for superpowers:finishing-a-development-branch.
═══════════════════════════════════════════
```

**Verdict levels:**
- **SHIP** — all REQs pass, no findings, no undocumented deviations
- **SHIP WITH ACKNOWLEDGMENTS** — all REQs pass, findings acknowledged, manual tests signed off
- **BLOCKED** — REQs failing, unimplemented REQs, or unacknowledged findings

Only SHIP and SHIP WITH ACKNOWLEDGMENTS allow proceeding to `finishing-a-development-branch`.

---

## Constraints

- **All inline.** No background tasks. Everything runs in main chat. Twin dispatches are synchronous — wait for responses.
- **No WebSearch.** This skill doesn't need external research. It's auditing internal artifacts.
- **Append-only artifacts.** BUILD_MANIFEST and DEVIATION_LOG are append-only during the build. The postrodeo READS them, never writes to them.
- **RETROSPECTIVE.md is the output.** It gets written to disk. It's the permanent record.
- **Gate, not ceremony.** This skill produces actionable findings, not participation trophies. If everything passes cleanly, the report is short. Length correlates with issues found, not with thoroughness theater.
- **Twin review is mandatory.** Phases 4 and 5 both dispatch to twins. This is the multi-agent advantage — use it. Skipping twin review requires explicit user waiver.
- **Policy scan is optional.** Only runs if `policy-rules.yml` exists. Projects without structured policy rules still get all other phases.
- **Print phase markers.** `[POSTRODEO — PHASE N]` before every phase. User should be able to scroll back and see exactly where things happened.

---

## Integration with Upstream Skills

### BUILD_MANIFEST.md — Produced by `superpowers:executing-plans`

After each task completes during plan execution, append an entry:

```markdown
## T-001 (Wave 1) — REQ-001, REQ-002 → FEAT-001
- **Status:** complete
- **Files created:** src/services/user.rs, src/models/user.rs
- **Files modified:** src/lib.rs
- **Tests added:** tests/user_service_test.rs (3 tests)
- **Tests passing:** 3/3 ✅
- **Deviations:** none
- **Completed:** 2026-04-06T14:32:00
```

### DEVIATION_LOG.md — Produced by `superpowers:executing-plans`

When the GSD deviation protocol fires (plan change during execution), append:

```markdown
## DEV-001 — Task T-005 approach changed
- **Original plan:** REST endpoint with JSON polling
- **Actual approach:** WebSocket with binary frames
- **Why:** Discovered latency requirement (REQ-007: <100ms) incompatible with HTTP polling overhead
- **Plan updated:** yes (IMPLEMENTATION_PLAN.md, Phase 3, Step 2)
- **REQs affected:** REQ-007, REQ-012
- **Logged:** 2026-04-06T16:45:00
```

### Test Stubs — Produced by `/uncompromising-executor`

When generating TEST_PLAN.md, also generate test stub files with REQ-IDs in function names:

```rust
// tests/req_tests.rs — generated by /uncompromising-executor
// DO NOT DELETE — postrodeo uses these function names for REQ traceability

#[tokio::test]
async fn test_req_001_schema_exists() {
    todo!("Implement: Verify Postgres schema created with all tables")
}

#[tokio::test]
async fn test_req_003_stale_data_banner() {
    todo!("Implement: Verify banner visible when data timestamp > 1hr old")
}
```

The `todo!()` stubs are replaced during implementation. The function NAMES are permanent — they're how the postrodeo maps test results to REQ-IDs.
