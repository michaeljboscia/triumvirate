# Retrospective — Triumvirate v2.2 "The Accountability Release"

**Date:** 2026-04-07
**Spec:** `daemon/docs/v2.2/SPEC.md`
**Branch:** `feat/mcp-first`
**Commits:** 63 commits (0766a48..HEAD)
**Builder:** Codex (GPT-5.2-Codex, long-running CLI session)
**Auditor:** Claude (Opus 4.6) + Gemini + Codex (twin review)

---

## Completion Summary

| Metric | Value |
|--------|-------|
| REQs specified | 102 |
| REQs with passing tests | 34 |
| REQs partial coverage | 3 |
| REQs E2E/manual only | 5 (dashboard/frontend) |
| REQs failing | 0 |
| REQs unimplemented | 0 |
| Tasks planned | 71 |
| Tasks completed | 71 |
| Test suite | 121 tests, 0 failures |

## Deviation Summary

| Metric | Value |
|--------|-------|
| Planned tasks | 71 |
| Completed as planned | 70 |
| Wave ordering violations | 1 (T-201 before T-119 — no impact) |
| Documented deviations | 0 |
| Undocumented deviations | 1 (ordering) |
| Twin-validated deviations | 1 (both twins: noise, not risk) |
| BUILD_MANIFEST populated | No (reconstructed from git) |
| DEVIATION_LOG populated | No (empty) |

## Semantic Review Summary

| Metric | Value |
|--------|-------|
| Findings total | 9 |
| High confidence (both twins) | 4 |
| Medium confidence (one twin) | 5 |
| GitHub issues filed | 9 (michaeljboscia/triumvirate #1–#9) |

### High-Confidence Findings

1. **#1 — Compression coupled to ingestion** — `ingest_event` calls compression synchronously, violating REQ-013 decoupling. Data loss risk under load.
2. **#2 — Review queue starvation** — `submit_review` never promotes pending→in_progress. Fleet merges deadlock with >max_inflight agents.
3. **#3 — Crash recovery incomplete** — Tasks not reset to pending during fleet recovery. Orphaned claimed tasks.
4. **#4 — Global env var UB** — `set_var` in async daemon is undefined behavior. Concurrent fleets corrupt each other's PROJECT_ROOT.

### Medium-Confidence Findings

5. **#5 — Fleet progress events missing** — Only fleet_spawned emitted. Dashboard fleet view is blind.
6. **#6 — Spool overflow health gap** — 100MB threshold not checked in health computation.
7. **#7 — Fleet task file incomplete** — Missing depends_on + prose description.
8. **#8 — fleet_spawn wait param missing** — No blocking path for callers.
9. **#9 — Session event_count drift** — Idempotent inserts inflate counter.

## Git Health

| Metric | Value |
|--------|-------|
| Total commits | 63 |
| Task ID coverage | 98% (62/63) |
| Orphan commits | 1 (legitimate fixup: da7fabd) |
| New crates | 3 (ledger, fleet, peer-review) |
| Dashboard scaffold | 1 (Svelte project) |
| Lines added | 9,100 |
| Lines deleted | 162 |
| Files changed | 56 |

## Lessons Learned

### 1. Codex doesn't maintain meta-documents during build

**What:** BUILD_MANIFEST.md and DEVIATION_LOG.md were scaffolded but never populated by Codex during the 63-commit build.
**Why:** The handoff doc said "populated during build" but didn't make it a per-task checklist item. Codex optimizes for code output, not documentation side-effects.
**Rule:** Future Codex handoffs must include an explicit per-task step: "append to BUILD_MANIFEST.md after each commit." Consider a post-commit hook that auto-populates.

### 2. Unit tests pass but integration logic breaks

**What:** All 121 tests pass, yet 4 HIGH-confidence bugs exist in cross-component integration.
**Why:** Tests verify individual functions in isolation. Compression works alone. Ingestion works alone. But calling compression FROM ingestion violates the spec. The test for "decoupling" was a compilation check, not a behavioral check.
**Rule:** Reality tests must test the BOUNDARY between components, not just the components themselves. "Ingest while compression is failing" is the test that catches FIX-001.

### 3. The superpowers pipeline has gaps

**What:** The goat rodeo crystallized XML task blocks + reality tests (from Pythia v2). Claude acknowledged the rules, ran the rodeo, and then produced an IMPLEMENTATION_PLAN.md without XML blocks. When called out, invoked the wrong skill (/plan instead of superpowers:writing-plans).
**Why:** Context degradation over a long session. The crystallized rules were in the goat rodeo skill, were read, and were still ignored during Phase 4 production.
**Rule:** Superpowers skills (writing-plans, executing-plans) must be INVOKED, not just followed as guidelines. The skill invocation forces the format.

### 4. Layer 6 semantic review is essential

**What:** The twin review in Phase 5 found 4 HIGH bugs that 121 passing unit tests missed.
**Why:** Tests verify what was coded. Twins verify what was INTENDED. The spec says "decoupled" — the test says "function returns Ok." Only a reader who knows the spec intent can catch the gap.
**Rule:** Layer 6 is not optional. Every build must get twin semantic review before shipping.

### 5. Spool-first architecture validated

**What:** The spool directory design (atomic rename, no file contention) survived the rodeo without issues. No bugs found in the spool/drain pipeline.
**Why:** The design was pressure-tested through 8 rounds of interrogation. The atomic rename pattern has no contention by construction.
**Rule:** Goat rodeo rounds pay for themselves. The spool design was attacked 5 times (R1 Q1, R2 Q1, R4 Q1, R5 Q1, R6 Q2) and improved each time.

## Process Recommendations

1. **Codex handoff must include BUILD_MANIFEST as per-task checklist item** — not just "populated during build"
2. **Add Playwright E2E to Codex capabilities** — 5 dashboard REQs can only be verified with a browser
3. **Reality tests must cross component boundaries** — "works in isolation" is not "works as specced"
4. **Layer 6 twin review is mandatory gate** — catches what unit tests structurally cannot
5. **Superpowers skills must be invoked, not mentally followed** — skill invocation forces format compliance
6. **Fix the 4 HIGH issues before shipping** — these are not acknowledgments, they're blockers

## Process Metrics

| Model | Role | Cost |
|-------|------|------|
| Claude (Opus 4.6) | Orchestrator, spec, goat rodeo, postrodeo | Subscription (Max plan) |
| Gemini | Twin review (8 rodeo rounds + 2 postrodeo) | $0 (CLI subscription) |
| Codex | Twin review (8 rounds + 2 postrodeo) + full build (63 commits) | $0 (CLI subscription) |

Total out-of-pocket API cost: **$0** — all via CLI subscriptions.
