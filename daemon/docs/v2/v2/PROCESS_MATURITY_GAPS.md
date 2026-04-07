# Process Maturity Gaps — Enhancement Thread

**Date:** 2026-04-06
**Source:** Flow State goatrodeo session — post-mortem process analysis
**Status:** OPEN — needs goatrodeo review before implementation
**Scope:** Improvements to the development process itself, not to triumvirate features

---

## Context

During the Flow State v2.1 goatrodeo (4 rounds, 27 REQs), we identified 5 gaps in the development process that apply to ALL projects, not just triumvirate. These are process-level defects that the goatrodeo itself should prevent but currently doesn't.

The irony: we're building agent observability (Flow State) while lacking observability into our own process.

---

## Gap 1: Automated Regression (CI Pipeline)

**Current state:** `cargo test` is run manually. Nothing prevents merging code that breaks existing tests.

**Impact:** Every protection rule that says "all existing tests must pass" (REQ-021, CONVERSATIONAL_PARITY checklist) is honor-system only. A distracted session can skip verification and ship regressions.

**What "solved" looks like:**
- Every push to a feature branch triggers `cargo check` + `cargo test` + `cargo clippy`
- PR cannot merge if any check fails
- Results visible in the PR conversation (not buried in a separate dashboard)
- Runs in <5 minutes (fast feedback)

**Options to investigate:**
1. **GitHub Actions** — standard, free for public repos, YAML config
2. **Local pre-push hook** — `cargo test` runs before `git push` succeeds. Zero infrastructure. But only enforces on the machine with the hook installed.
3. **Claude Code hook** — hookify rule that runs `cargo test` after any Edit/Write to Rust files. Already have the hook infrastructure.

**Recommendation:** Start with option 3 (Claude Code hook) for immediate enforcement, add option 1 (GitHub Actions) for permanent CI. Option 2 as belt-and-suspenders.

---

## Gap 2: Golden Trace Mandate

**Current state:** First time we captured live CLI traces was during the Flow State session. No standard process exists for "before you write a parser for an external protocol, capture the actual wire format."

**Impact:** Parsers built against documentation or inferred schemas can be wrong. The Codex exec trace revealed that event types use dot notation (`thread.started`) not snake_case (`thread_started`), and `item.completed` carries the full payload inline — both different from what binary symbol analysis predicted.

**What "solved" looks like:**
- Goatrodeo Phase 0 or Pre-Round includes: "Does this spec integrate with an external protocol? If yes, golden trace capture is a BLOCKER before architecture rounds."
- Golden traces are committed as test fixtures
- Parsers are tested against real traces, not synthetic fixtures
- When CLI versions upgrade, traces are recaptured and parsers re-validated

**Where to implement:** Add to `/goatrodeo` skill as a Phase 0 sub-step:

```
Step 0.6: Protocol Integration Gate
  If the spec involves parsing external tool output, API responses, or wire protocols:
  1. Capture a live trace from the actual production binary
  2. Commit as test fixture
  3. Verify trace contains all event types the spec references
  BLOCKER if any referenced event type is absent from the trace.
```

---

## Gap 3: Process Compliance Enforcement

**Current state:** We almost skipped the Phase 3 quality gate. The user caught it ("we shouldn't have gone out of process"). Nothing in the tooling prevented the skip — the goatrodeo skill defines the steps but doesn't enforce ordering.

**Impact:** The goatrodeo's value comes from its state machine — each step depends on the previous step's output. Skipping steps defeats the purpose. If it can be skipped, it will be skipped under time pressure.

**What "solved" looks like:**
- The goatrodeo skill tracks which steps have been completed
- Attempting to run Step N without Step N-1 output produces a warning
- The Decision Ledger cannot be declared "done" without Phase 3 quality gate completing
- A checklist is printed at the end showing all gates passed/skipped

**Options to investigate:**
1. **State tracking in the skill** — goatrodeo maintains a `completed_steps` set and checks before each step
2. **Hookify rule** — hook that fires when goatrodeo outputs are written, validates all prior steps exist
3. **Mandatory checklist** — goatrodeo prints a final checklist that the user must acknowledge item-by-item

**Recommendation:** Option 1 — state tracking built into the skill. The skill already prints `[ROUND N — STEP M]` markers. Add a set that tracks which markers have been printed. Refuse to print Step M if Step M-1 hasn't been printed.

---

## Gap 4: Token/Cost Tracking for the Process Itself

**Current state:** We don't know how many tokens this session consumed across Claude (Opus 4.6), Gemini (via daemon sessions), and Codex (via daemon sessions). We're building token visibility for agents but don't have it for ourselves.

**Impact:** Can't answer "how much does a goatrodeo cost?" or "is 4 rounds worth it vs 2 rounds?" Can't optimize the process because we can't measure it.

**What "solved" looks like:**
- Each goatrodeo round logs: tokens consumed by Claude, tokens consumed by Gemini (from daemon), tokens consumed by Codex (from daemon)
- Final ledger includes a cost summary: "4 rounds consumed ~X tokens across 3 models"
- Over time, build a baseline: "a typical goatrodeo costs X tokens and catches Y issues"

**Where the data lives today:**
- Claude: usage shown in `/usage` command (but not programmatically accessible mid-session)
- Gemini: `result.stats` in stream-json (we just proved this with the golden trace)
- Codex: `turn.completed.usage` (we just proved this with the golden trace)

**Irony:** Flow State (the feature we just specced) gives us Gemini+Codex token tracking. But Claude's own token usage isn't exposed to us mid-session.

**Options to investigate:**
1. **Manual capture** — at end of each goatrodeo, run `/usage` and log the delta
2. **Daemon-side tracking** — triumvirate daemon already logs outbox events with `tool` and `status`. Add `token_usage` to outbox events (REQ-017, deferred to v2.2). When v2.2 ships, the daemon tracks twin token usage automatically.
3. **Session log enrichment** — stenographer (when implemented) captures all events including token counts

**Recommendation:** Option 1 now (manual), option 2 when v2.2 ships. The goatrodeo skill's final output should include a "Process Metrics" section with whatever token data is available.

---

## Gap 5: Post-Ship REQ Traceability (Never Been Run)

**Current state:** The REQ Traceability Gate exists in the goatrodeo skill, the superpowers verification rules, and the TEST_PLAN.md template. But it has never been executed on a shipped feature. It's a spec, not a proven process.

**Impact:** We don't know if the gate actually works. Does `cargo test` output map cleanly to REQ-IDs? Can we actually produce the traceability matrix? Are the test types (unit/integration/E2E/manual) correct, or do some REQs need different test types than specced?

**What "solved" looks like:**
- Flow State v2.1 becomes the FIRST feature to run the full traceability gate
- After Phase 6, before declaring v2.1 shipped:
  1. Run every test in TEST_PLAN.md
  2. Map results to REQ-IDs
  3. Produce the traceability matrix
  4. Any FAIL = not shipped
  5. Any SKIP = user sign-off required
- Document what worked and what didn't about the gate process
- Feed lessons back into the goatrodeo skill

**Recommendation:** Treat v2.1 as the test run for the traceability gate. Expect friction. Capture lessons. Improve the gate before v2.2.

---

## Proposed Process: How to Address These Gaps

These are process improvements, not features. They should go through a lightweight version of the goatrodeo — not 4 full rounds, but at least:

1. **Write this doc** (done)
2. **Send to twins for review** — "are these the right gaps? are the solutions reasonable?"
3. **User prioritizes** — which gaps to fix first
4. **Implement in order** — each gap is a small, independent improvement

**Suggested priority:**
1. Gap 3 (compliance enforcement) — prevents process decay, cheapest to fix
2. Gap 2 (golden trace mandate) — add one paragraph to goatrodeo skill
3. Gap 1 (CI) — highest impact but most infrastructure work
4. Gap 5 (traceability gate) — validated by shipping v2.1
5. Gap 4 (token tracking) — blocked on Flow State v2.2 for full solution

---

## Action Items

- [ ] Send this doc to twins for review
- [ ] User prioritizes gaps
- [ ] Update `/goatrodeo` skill with Gap 2 (golden trace gate) and Gap 3 (state tracking)
- [ ] Set up CI for triumvirate (GitHub Actions or pre-push hook)
- [ ] Run traceability gate on v2.1 after implementation
- [ ] Add "Process Metrics" section to goatrodeo final output
