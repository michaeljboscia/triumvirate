# Retrospective — v3.2.0 Observability & Token Economics

**Date:** 2026-04-10
**Spec:** specs/OBSERVABILITY_TOKEN_ECONOMICS.md
**Tag:** 3.2.0
**Commits (3.1.0→3.2.0):** 20 (90% task-referenced, 0% snapshot noise)

## Completion Summary

| Metric | Value |
|--------|-------|
| REQs specified | 40 (23 observability + 17 token economics) |
| REQs passing | 36 |
| REQs partial | 2 (O1 span count, T13 needs runtime validation) |
| Completion rate | 90% |

## Deviation Summary

| Metric | Value |
|--------|-------|
| Planned tasks | 24 |
| Completed | 24 (including hotfix) |
| Deferred | 0 |
| Deviations | 8 (3 high-risk per twin review) |
| Process artifacts missing | 4 (TEST_PLAN, BUILD_MANIFEST, DEVIATION_LOG, DOC_AUDIT) |

## Twin Review Consensus

Both Gemini and Codex flagged deviation 6 (/uncompromising-executor skipped) as the root cause of multiple downstream issues. Gemini called the stale sentinel (DEV-3) "a violation of the no-silent-failures constitution." Codex rated deviations 2-6 as high-risk.

## Git Health

| Metric | Value |
|--------|-------|
| Total commits | 20 |
| Task-ID coverage | 90% |
| Snapshot noise | 0% (bypass file active) |

## Lessons Learned

1. **/uncompromising-executor cannot be skipped.** The missing TEST_PLAN directly caused: no acceptance criteria for the postrodeo, no test-to-REQ mapping, and no objective verification that metrics actually fire. "The code compiles" is not "the code works."

2. **Wave 0 contracts must define full API surface.** 4 integration errors from 3 workers inventing incompatible TokenDb APIs. The contract task (T-102) defined the struct but not the consumption interface. Downstream workers guessed differently.

3. **Stale sentinel is a critical bug.** Old TASK_COMPLETE.json files persist in worktree baselines and cause premature completion detection. Must delete during worktree setup.

4. **Watchdog must check sentinel before STUCK.** T-106 was marked STUCK despite having committed and written its sentinel. The watchdog should check if the sentinel exists before marking a task STUCK.

5. **Scanner must not block boot.** REQ-T17 explicitly says "does not block daemon boot." The initial implementation violated this. Hotfixed to tokio::spawn but should use spawn_blocking for CPU-heavy parsing.

6. **BUILD_MANIFEST must be produced during the build.** 5th consecutive sprint missing this artifact. The goatrodeo rule exists (Step 5.4 line 1171). Needs mechanical enforcement.

7. **Snapshot hook bypass should be DEFAULT during sprints.** v3.1.0 had 53% noise; v3.2.0 had 0% because bypass was active. The bypass file should be created at sprint start and removed at sprint end.

## Process Recommendations

1. **Never skip /uncompromising-executor.** If time-pressured, run it in abbreviated mode (TEST_PLAN + BACKEND_STRUCTURE only), but never skip entirely.
2. **Write BUILD_MANIFEST hook** that blocks next dispatch if current task lacks a manifest entry.
3. **Fix stale sentinel** — `rm -f .triumvirate/TASK_COMPLETE.json` in worktree_setup.rs.
4. **Fix watchdog race** — check sentinel existence before marking STUCK.
5. **Use spawn_blocking for scanner** — CPU-bound 536MB JSON parsing shouldn't run on async executor threads.
6. **Activate snapshot hook bypass at sprint start** as standard practice.
7. **Wave 0 must define pub fn signatures** for every function downstream tasks will call.
