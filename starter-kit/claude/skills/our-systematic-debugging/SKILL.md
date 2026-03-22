---
name: our-systematic-debugging
description: Use when encountering any bug, test failure, or unexpected behavior, before proposing fixes — extends systematic-debugging with failure crystallization
---

# Our Systematic Debugging

**REQUIRED BACKGROUND:** You MUST understand superpowers:systematic-debugging before using this skill. This skill adds Phase 5 (Crystallization) to the standard 4-phase debugging process.

## Phases 1-4

Follow the standard `superpowers:systematic-debugging` skill exactly:
1. Root Cause Investigation
2. Pattern Analysis
3. Hypothesis and Testing
4. Implementation

### Phase 2.5: Dependency Error Search Gate

**Between Pattern Analysis and Hypothesis:** If the error originates from a third-party library, extension, or platform API (not code you wrote):

1. Run **at least 2** `mcp__gemini__gemini-search` queries with the exact error message + library name
2. Check the upstream issue tracker for known workarounds
3. Only THEN form a hypothesis

**Why:** Dependency errors often have non-obvious constraints (e.g., sqlite-vec requires CTE isolation for JOINed KNN queries — swapping `LIMIT` for `k=?` alone would still fail). The library author's fix is almost always better than your intuition. Skipping this step risks fixing the symptom while missing the real constraint.

## Phase 5: Crystallize (If Recurring)

**After completing Phase 4, check:**

1. Has this failure class occurred before? (Search lessons, iron laws, memory entries)
2. Do BOTH a lesson AND an iron law exist for this failure class?
3. Have we spent more than 3 iterations or significant time on this problem?

**If ANY of these are true:** Invoke the `crystallize` skill.

The crystallizer will:
- Capture the failure data (or use what Phases 1-4 already produced)
- Diagnose root cause via triumvirate
- Graduate to skill or matrix
- Build and validate the output

**If none are true:** The failure is handled. Move on.

## When NOT to Invoke Phase 5
- One-off bugs with clear fixes
- First encounter with a problem (no history of recurrence)
- Issues already covered by an existing skill or matrix
