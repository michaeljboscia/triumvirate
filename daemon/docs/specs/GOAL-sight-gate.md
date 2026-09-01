# GOAL: the sight gate, reviewed and tested to completion

**Set by the owner 2026-09-01.** Loop with the three CLI peers. Unit, integration and
end-to-end tests. Finish the change.

`/goal` does not mean stop and wait. Work it through and report at the end of each round.

---

## The goal in one sentence

**A dispatch declared to be a review cannot come back from any route with an answer the
agent produced without looking at anything, and that guarantee is proven by tests that can
fail.**

## Why this exists

2026-09-01. Three peers were dispatched with filesystem access to review one brief.

| Peer | Tool calls | Outcome |
|---|---|---|
| Codex | 35 | proved the reviewed design's core claim false, with three route citations |
| Grok | 4 primary sources | found the only error that had actually reached a client |
| Antigravity | **0** | graded nine research citations from memory, called it "rigorous sourcing", two grades wrong |

Nothing in Triumvirate noticed. A human noticed the output had no links in it.

This is ISO/IEC 27042's validation step pointed at the reviewer instead of the evidence:
before accepting a finding, establish the method could have seen the thing.

## Scope: IN

1. `require_sight` on `AskAgentRequest`. A turn completing with zero tool calls is REJECTED,
   not returned with a caveat. A generated objection that does not stop the caller gets
   quoted approvingly and the wrong conclusion ships anyway, which is the documented
   2026-08-31 failure.
2. `tool_calls_made` on `AskAgentResponse`, always populated. The receipt, not just the gate.
3. **Every route** that can return text to a caller. The primary success arm and the
   degraded hop are done. Session paths, faildown, prewarm, HTTP, `ask_session` are open
   until proven. A fix that lands on one of two surfaces is the local idiom for a whole
   class of defect here, and it is how the session-leak fix passed its own verification
   while surviving on the other surface.
4. The full pyramid: unit, integration against a mock binary, end-to-end opt-in.
5. Peer review of the change itself by all three CLI peers, each on a different lens.

## Scope: OUT, named so it is a decision

- Proving the reviewer read the RIGHT things. The gate proves it looked at something.
  Naming primary sources lives in the review brief, which is prose and bypassable.
- The bash harness in `~/.claude/scripts/peer-review/` stays as the cross-project tool.
  This is the in-daemon version, not a replacement for it.

## Non-negotiables

- **No test that cannot fail.** This repo has produced three: one asserting against a
  closure the test defined itself, one documenting a bug as `"documented limitation"`, one
  scanning source text with an assertion string that matched itself. For every test, state
  what breaking change turns it red.
- **`cargo test` passes with no network and no API keys.** End-to-end is opt-in behind an
  env var.
- **No regressions.** Every existing caller that does not set `require_sight` behaves
  exactly as before.
- **A false rejection is worse than the hole.** If any parser under-reports `tool_calls`,
  this gate breaks that agent. Verify all four parsers before shipping.
- State what is not verified rather than letting silence imply it was.

## DONE WHEN

Each is a command whose failure is observable.

1. `cargo test` green across the workspace, no network, no keys.
2. `cargo clippy --all-targets` clean.
3. A test proves zero tool calls plus `require_sight` returns Err.
4. A test proves zero tool calls WITHOUT `require_sight` still returns Ok. This is the
   regression guard and it is the one most likely to be missing.
5. A test covers the degraded-hop arm, not only the primary arm.
6. Every route named in scope item 3 is either gated or listed as deferred with the reason.
7. All four parsers confirmed to populate `tool_calls` for a tool-using turn, by test or by
   fixture, so the gate cannot false-reject.
8. Three peer reviews of the final state, each passing `receipt.sh`, with findings either
   fixed or recorded as declined with a reason.
9. Installed via `scripts/install.sh`. Never run from `target/`.
10. Committed, with the open items listed FIRST in the report, or the word "done" not used.

## Round log

Checkpoints are appended to `GOAL-sight-gate-progress.md` at each synthesis. Do not keep
round state only in conversation context.
