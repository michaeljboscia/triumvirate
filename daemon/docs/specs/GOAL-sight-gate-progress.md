# Sight gate: round log

## Round 1, 2026-09-01. Three peers, three lenses, all dispatched before any was read.

Receipts, via `~/.claude/scripts/peer-review/receipt.sh`:

| Peer | Sources | Tool markers | Receipt |
|---|---|---|---|
| Codex | 81 | 236 | ACCEPT |
| Grok | 26 | 18 | ACCEPT |
| Antigravity, first run | 0 | 0 | REJECT |
| Antigravity, after harness fix | 8 | 10 | ACCEPT |

**The Antigravity zero was MY bug, not the model's.** `dispatch.sh` dropped
`--dangerously-skip-permissions`, so headless agy auto-denied every tool it tried to use and
emitted one line: "no output produced". My first read of that blamed the model. A zero from a
blocked instrument is not evidence about the agent, which is the exact distinction the gate
itself draws. Fixed in `dispatch.sh` with the reasoning recorded there.

### What the peers found, and what changed because of it

**Codex, buildability lens.** Six findings, all real.

1. `codex-app-server-jsonrpc` declares `tool_calls` and never pushes. I had found this
   independently while checking all four parsers. FIXED.
2. The gate returns `Err` inside the `Ok(parsed)` arm, bypassing retry, faildown, degrade and
   dead-drop. OPEN.
3. Success side effects fire BEFORE the gate: `tel.success`, DONE lifecycle, DONE outbox,
   worker-session update, progress "responded". The ledger records DONE for a rejected turn.
   OPEN, and the sharpest of the six.
4. DeepSeek is remote and metered, has no tools, so `require_sight` spends the call and then
   always rejects. Should refuse before dispatch. OPEN.
5. `grok-batch-json` also returns empty `tool_calls`, reachable at `agent_exec.rs:2843`.
   Verified independently, then FIXED.
6. `tool_calls_made` not set by `AskAgentResponse::direct()` itself. Minor, set on the
   production path. OPEN.

Route survey: only one-shot `ask_agent` can carry `require_sight` today. `ask_session`,
`ask_daemon`, `send_message`, HTTP `/session/ask`, `query_antigravity*` all build
`AskAgentRequest { ..Default::default() }` and cannot set it. OPEN, and it is the two-surface
split again.

**Grok, is-this-the-same-mistake lens. Verdict: yes, and it was right.**

> Counting tool calls is the `\dt` problem reimplemented in Rust. It demands that a look be
> cited, as a number, once a look has occurred. It does not force the method.

Named the cheapest defeats against this exact stack: one `todo_write` (classified `Unknown`,
still counted), `list_dir .`, `pwd`, or a `read_file` of a path that does not exist because
the gate never checked `success`. It noted it nearly took the first of those before opening
any files.

**And it found the blocker.** `agy-pipe-plain-text` and `agy-pty-plain-text`
(`crates/triumvirate/src/agy.rs`, `build_result`) hardcode `tool_calls: Vec::new()`. So does
`deepseek-sse`. **Antigravity, the agent this gate was built for, was structurally invisible
to it** and would have been rejected on every review whether it looked or not. My own
`sight_06` test could not catch this: its hardcoded list covered only `agent-adapter`, while
the agy parser lives in `crates/triumvirate`. The test's own method could not see the thing it
guarded. Same defect, three levels deep.

### Changes made in response

- **Denylist inverted to an allowlist.** `PARSER_MODES_WITH_TOOL_RECORDS` names only the three
  parsers verified to push on a real tool event. A denylist fails OPEN for anything unlisted,
  so every new parser silently becomes a false rejection. An allowlist fails CLOSED.
- **Named-sources check added.** `required_sources` on the request. A source is satisfied only
  by a SUCCESSFUL read-shaped call whose recorded args mention it, by full path or basename.
  This is Grok's proposal and it is the difference between a fig leaf and a gate. It proves the
  method requested the named thing. It does NOT prove the contents were used, and the field
  docs say so rather than implying otherwise.
- **Write detection added,** answering the owner's question about looking without touching.
  A review that calls WriteFile or EditFile is rejected. Honest limit pinned in
  `sight_10`: `codex-exec-json` stamps every call `ToolKind::Bash`, so a write inside a shell
  command is invisible here and only the read-only sandbox actually prevents it.

### Tests: 14, all mutation-verified

Two mutations were run against the suite to prove the tests can fail:
- `tool_calls > 0` to `>= 0` turned `sight_02`, `04`, `05` red.
- collapsing the two-zero branch turned `sight_04`, `05` red.

Each test carries a `RED IF:` line naming the breaking change that turns it red.

## Still open, in priority order

1. Success side effects fire before the gate (Codex 3). The ledger says DONE for a rejected
   turn. Fix by moving the gate ahead of the side effects.
2. DeepSeek refused before dispatch, not after spending a metered call (Codex 4).
   `AGENTS_WITHOUT_TOOLS` exists and is asserted by `sight_11`, but is not yet wired into a
   pre-dispatch check.
3. Rejection discards the reviewer's text entirely (Grok 4). The only demonstrated catcher on
   2026-09-01 was a human reading the output and noticing it had no links. Discarding the text
   destroys that artifact. Return it somewhere that is not `.response`.
4. `require_sight` is opt-in and defaults false, and the rejection message names dropping the
   flag as the way through. Grok: "a skip-catcher that is off unless remembered is not a
   skip-catcher." The durable answer is a distinct reviewer surface with sight always on.
5. Routes that cannot set the flag at all (Codex route survey).
6. Integration and end-to-end tests. Only unit tests exist so far.
7. Containment: reviewers should be dispatched read-only. Detection cannot cover Bash.

---

## Round 2, 2026-09-01. Same three peers, same lens split, artifact re-reviewed.

| Peer | Receipt | Outcome |
|---|---|---|
| Codex | 65 sources, 129 markers, ACCEPT | 7 findings, 5 real and acted on |
| Antigravity | 4 sources, 4 markers, ACCEPT | ran the mutation itself, found 3 real test defects |
| Grok | REJECT, twice | **invalidated by my error, see below** |

### Grok's round 2 was destroyed by me, twice, and that is a process finding

Both attempts returned narration and no verdict. Its own words: *"the matcher changed under
me"*, and on the retry, *"the live gate already diverges from the diff"*. **I was editing the
code while three reviewers were reading it.** A review of a moving artifact is not a review.

Rule: FREEZE the artifact before dispatching, and do not touch it until every reviewer returns.
I froze it before the retry and then edited again anyway. This cost two full Grok reviews.

Second, related: `receipt.sh` FALSE-REJECTED Grok's first attempt. Grok's plain `-p` output
narrates its reads in prose rather than emitting parseable tool markers, so the grep-based
checker saw nothing while the agent had demonstrably looked. The in-daemon gate reads
structured `ToolCallRecord`s and does not have this weakness. The bash checker is the weaker
instrument and should not be trusted to fail an agent whose output format it cannot parse.

### Codex round 2, acted on

1. **Degraded arm still had the round 1 bug.** It persisted tokens, pushed DONE, appended a
   DONE outbox entry, emitted "responded" and recorded `degraded_success` BEFORE the gate.
   Fixing one of two surfaces, for the third time in this single change. FIXED.
2. **Branch order was wrong.** Named sources were checked before parser capability, so a blind
   parser with named sources reported "never successfully opened" and blamed the agent for the
   instrument's blindness. FIXED, capability now first.
3. **The allowlist did not fail closed.** An unvetted parser passed simply by recording one
   call, which is the denylist behaviour the allowlist replaced. FIXED, `sight_18` pins it.
4. Stale header comment still read "Parser modes that do NOT record" above an allowlist. FIXED.
5. Degraded rejection discarded the reviewer's text. FIXED.
6. Its basename false-positive finding was already stale: I had replaced fuzzy suffix matching
   with cwd-relative matching while it was reading. That is the moving-artifact problem again,
   this time making a reviewer's correct finding look wrong.

### Antigravity round 2, acted on. A complete reversal from round 1.

It ran `cargo test --workspace`, then mutated the gate itself and reported which tests caught
it. That is the assignment done properly.

1. **`sight_03`'s RED IF was FALSE.** It claimed red under `>= 0`, but one call passes under
   `>= 0` too, so the test could not fail for the reason it advertised. A test whose RED IF is
   wrong is the same defect as a test that cannot fail. FIXED: the boundary is now pinned from
   both sides.
2. **`sight_06` had the identical shape to the version that missed the agy parser.** A
   hardcoded blind list cannot catch a NEW blind parser added to the allowlist. FIXED: the
   allowlist is now pinned exactly, so any addition turns the test red.
3. **The REJECTED lifecycle event was dropped.** The gate pushed it, the function returned
   `Err`, and `lifecycle` went out of scope. The ledger had no record that a rejection ever
   happened, making "how often are reviews rejected for having looked at nothing" unanswerable,
   which is the exact question the gate exists to answer. FIXED on both arms.

## Final state

18 unit tests, 5 integration tests against the mock, 2 live tests behind
`TRIUMVIRATE_LIVE_GROK=1`. Full workspace suite green, 38 groups, zero failures. Clippy clean
for the touched crates; one pre-existing `redundant closure` warning remains in `pantheon` and
was deliberately not touched.

## Still open, honestly

1. **`require_sight` is opt-in and defaults false**, and the rejection message names dropping
   the flag as the way through. Grok round 1: "a skip-catcher that is off unless remembered is
   not a skip-catcher." The durable answer is a distinct reviewer surface with sight always on.
2. **Only one-shot `ask_agent` can set it.** `ask_session`, `ask_daemon`, `send_message`,
   HTTP `/session/ask` and `query_antigravity*` all build `AskAgentRequest { ..Default }`.
3. **No test drives the degraded arm's gate.** Codex flagged this in round 2 and it is still
   true: the degraded sight code has production logic and no test. It is where a test should
   have failed and did not.
4. **A write inside a shell command is undetectable.** `codex-exec-json` stamps every call
   `ToolKind::Bash`. Only the read-only sandbox prevents it, and agy is NOT contained.
5. **Opening a source is not reading it.** An agent can open every named source and answer from
   memory. Entailment against the opened text, or a human, is the next layer and is not built.
6. **Grok has not reviewed the final state.** Two attempts were invalidated by my edits.

---

## Post-commit correction, 2026-09-01

**Commit `8f9f6c2` claimed 18 unit tests and shipped 11.** `sight_12` through `sight_18` were
destroyed by my own block edit when rewriting `sight_06`: I sliced the source between two
anchors and everything between them went with it. The full suite stayed green at 38 groups
throughout, because a deleted test is silently absent rather than red.

That is the same class as a test that cannot fail, and it is worse: a suite cannot tell you
about a test that is not there. The only thing that caught it was noticing `cargo test`
reported 11 where 18 were expected.

Restored, plus `sight_19` added to close the degraded-arm gap Codex flagged: a structural
ordering test asserting BOTH success arms gate before writing DONE, following the
`persist_deepseek_err_tokens` precedent already in this file. Mutation-verified by deleting the
degraded arm's gate block, which turns it red with "found 1" where 2 are required. The first
attempt at that mutation did not compile, so it proved nothing; a mutation that does not build
is not a mutation test.

Final: 18 unit, 5 integration, 2 live. 38 groups green. Clippy clean on touched crates.

**Rule: after any block-level source edit, count the test functions before claiming a number.**

---

## Round 3, 2026-09-01. Grok, on the frozen commit `4f84b3a`. The one that landed.

Grok verified blob hashes against HEAD before reading, then reviewed. Receipt: it opened the
gate, both success arms, all 18 unit tests, the integration tests, three parsers, `agy.rs`,
`inter_agent.rs` and `streaming.rs`.

### BLOCKER, verified independently: Antigravity can never pass the gate

The allowlist trusts `gemini-stream-json`. The LIVE antigravity path does not emit that. It
emits `agy-pipe-plain-text` or `agy-pty-plain-text` (`agy.rs:320`), and neither is on the
allowlist, and `build_result` hardcodes `tool_calls: Vec::new()` regardless.

**So a `require_sight` review dispatched to Antigravity is rejected 100% of the time, however
carefully it looks.** The gate is permanently closed against the exact agent whose zero tool
calls motivated building it. `sight_17` asserts this behaviour as CORRECT, so the lockout is
encoded as intended rather than flagged as a defect.

This fails the GOAL's own non-negotiable: "a false rejection is worse than the hole."

Root cause is not the parser, it is the dispatch. agy is invoked in plain-text mode, so there
are no structured tool events to record. `agy --output-format stream-json` exists. The fix is
to dispatch agy structured and parse tool events, which is real work in `agy.rs` and out of
scope for this commit. Until it lands, `require_sight` must not be used with antigravity.

### Also found, all verified

- **Dead commented-out copy of the counting fallback** left above the live one, residue from a
  mutation-test revert. REMOVED.
- **`args.contains` is substring, not path equality.** A read of `agent_exec.rs.bak` satisfies
  a source named `agent_exec.rs`. So does `ls <path>` or a grep whose PATTERN is the path.
  `sight_15` closed the suffix hole and not this one.
- **`success: None` counts as success**, and grok records `success: None` on `tool_call` until
  a later `tool_call_update`. An incomplete call reads as a completed look.
- **`i_sight_01..05` never call `enforce_reviewer_sight`.** They prove the parser records and
  the unit tests prove the gate against hand-built records. The chain the comments describe as
  closed, live CLI to parser to record to gate, is not actually tested end to end. A gate that
  ignored `args_json` entirely would leave `i_sight_04` green.
- **`chars().take(600)` can drop the artifact it exists to preserve.** A long unsighted review
  opens with throat-clearing, and the thing a human needs to see is what is missing from the
  whole text.
- **DONE WHEN item 4 is STILL UNMET.** There is no test that zero tool calls WITHOUT
  `require_sight` still returns Ok. My own goal doc named it the regression guard most likely
  to be missing, and it is missing: every `sight_*` test calls the inner function directly,
  which is only reached once the flag is on.

Grok's summary of the shape: "the counting fallback, the escape hatch in the error string, and
the dead commented branch are the same voice: the gate is still willing to describe itself as
optional."

### Verdict

> It is still a fig leaf on the path that will actually run.

On a sighted hedge being unfixable at this layer, its verdict is that the function never
receives tool RESULTS, only the record that a call happened, so "could have seen" is the
honest ceiling of a dispatch gate. Closing "did use" needs the read bytes and an entailment
check, which is a different feature.

---

## Round 4, 2026-09-01. The agy lockout is FIXED and proven live.

Grok's blocker was real: the gate rejected every Antigravity review because the live agy path
records no tool calls. Root cause was dispatch, not the parser.

### The stale requirement

`agy-integration-spec.md` REQ-012 forbade `--output-format`, and REQ-060 recorded exactly why:
**verified 2026-05-24 against agy v1.0.1, which had no such flag.** The installed binary is
**v1.1.23**. The flag now exists with a `stream-json` value. The prohibition was correct when
written and had simply gone stale, which is why "do not delete and hand-wave" matters: the
reason was written down, so it could be checked instead of guessed at.

### Verified live before writing any code

`agy --output-format stream-json` emits, captured in
`crates/agent-adapter/tests/fixtures/agy-stream-*.jsonl`:

- `result.response` carries the whole final answer, replacing ANSI-stripped terminal scraping
- `step_update` with `step_type:"tool"` carries `tool_name`, `tool_info.parameters` and output
- `usage` carries real token counts, replacing log-file parsing
- `result.status` gives a terminal verdict, so a truncated stream is detectable

Strictly more information than the plain-text path. Also observed: agy's `run_command` reports
`pwd` as the HOME directory even though `init` declares the dispatch cwd. Noted, not yet acted
on, and it matters for cwd-relative source matching.

### What changed

- **`crates/agent-adapter/src/agy_stream.rs`**, new. Parses the stream into a real
  `ParsedAgentResult` with populated `tool_calls`, emitting `agy-stream-json`.
- **agy is dispatched with `--output-format stream-json` by default.**
  `TRIUMVIRATE_AGY_OUTPUT=text` reverts, and the revert re-locks Antigravity out of the gate,
  which is stated at the flag.
- **`agy-stream-json` added to the sight gate allowlist.** Antigravity can now pass.
- **A truncated stream is treated like the REQ-024 empty-output canary**: retried, not returned
  as a confident short answer.
- **`session_id` is deliberately NOT published** even though the stream carries a
  `conversation_id`. agy is single-turn (REQ-040/042) and publishing an id would let the worker
  registry cache something no dispatcher will resume, which is the shape of the cross-session
  leak this project already fixed once. An existing test caught this bug as I introduced it.
- **`--output-format` stays forbidden to OPERATORS** via `FORBIDDEN_EXTRA_FLAGS`. Triumvirate
  now sets it; an operator still cannot override it. Those are different claims, and conflating
  them is what kept agy in plain-text mode after the flag existed.

### Tests

11 parser tests against LIVE captures, not hand-written fixtures. A hand-written fixture only
proves the parser matches my belief about the format, which is the mistake that left agy unable
to report tool calls for months.

`sight_20_antigravity_can_actually_pass_the_gate` is the regression guard: an antigravity
review that opened the named source must PASS. `sight_17` was rewritten, since it previously
asserted the LOCKOUT as correct behaviour.

**`e_agy_sight_01` ran LIVE and passed**: a real agy turn read a real file, recorded the read,
preserved the path in its arguments, and returned the marker. Full chain proven from CLI to
gate.

Three legacy mock tests were converted from plain text to stream-json, because a test that
pins a path production no longer takes is worse than no test.

Full suite: 38 groups green. Clippy clean except one pre-existing `pantheon` warning.

---

## Round 4, 2026-09-01. Codex found the agy fix did not reach production.

**The fix shipped broken, and worse than "unfixed".**

`build_agy_invocation` has TWO branches. The sandbox branch delegates to `agy_args`. The
default branch, yolo, builds its argv INLINE. I added `--output-format stream-json` to
`agy_args` only, which is the branch production does not take.

So the default path still emitted plain text while the dispatcher had already switched to
parsing NDJSON: no result event, treated as truncated, retried, failed. That does not merely
fail to close the lockout, it breaks every agy call in the default configuration.

**Why no test caught it.** `agy_args_never_include_forbidden_flags` exercises `agy_args`
directly, the helper the default path does not use. And `e_agy_sight_01` spawns `agy` with
hardcoded flags, so it proved the parser works and proved nothing about what the daemon spawns.
Verifying through the path I patched rather than the path production takes, again.

**This is the two-surface split for the FIFTH time in this work**: primary vs degraded arm,
rejected-text preservation, the outbox event, and now two argv builders for one invocation.

### Then my own fix-test could not fail

The first `default_invocation_carries_stream_json` survived a mutation that deleted the exact
line it guards. Cause: the test I paired it with set `TRIUMVIRATE_AGY_SANDBOX` as a
process-global, and cargo runs tests in parallel, so the "default" test could take the SANDBOX
branch and pass on a path it was not testing.

Fixed two ways: the default test now pins `inv.program == "agy"`, so a leaked env var is a
visible failure rather than a false pass; and the sandbox test no longer mutates the
environment at all, asserting against `agy_args` directly since that IS the sandbox branch's
builder.

Re-verified with two independent mutations. Removing the yolo flag reds ONLY the default test.
Removing the `agy_args` flag reds ONLY the sandbox test. Each builder is now independently
guarded.

### New live test

`e_agy_sight_02` drives `build_agy_invocation`, the function production calls, and runs the
resulting argv against the real binary. **Ran live and passed.** This is the test that would
have caught the shipped bug.

### Also verified live this round

agy's `run_command` reports `pwd` as `/Users/michaelboscia/.gemini/antigravity-cli/scratch`,
an agy-managed scratch directory. Not the dispatch cwd, and not home as first assumed. agy then
resorted to `find / -name evidence.txt` to locate a relative path.

Consequence for the gate: **cwd-relative source matching is inert for agy.** Absolute paths
still match, and the review brief template already mandates absolute paths, so `required_sources`
works. But the relative candidate contributes nothing on this backend and should not be
described as if it does.

Full suite: 39 groups green. Clippy clean except the pre-existing `pantheon` warning.

---

## Round 4 continued: Grok and Antigravity on the agy fix. Six real defects.

Both reviewed `dded41b`, so both predate the default-path fix.

### From my own fixture, which I collected and did not read

**The parser never recorded a failure.** The shipped capture
`agy-stream-tools-20260901.jsonl` contains
`"state":"ERROR","error":{"type":"TOOL_ERROR","message":"context canceled"}`. My code comment
beside it asserted "agy does not emit an explicit error flag in the captures". False, and
contradicted by the file next to it. ERROR fell through to `success: None`, the gate reads
`success.unwrap_or(true)`, so a FAILED agy read counted as a successful look and `sight_14`
could not fail on the agy path. Grok found it by reading the evidence I had gathered.
FIXED, with `agy_12` and `agy_13`, the latter pinning that the fixture still exercises a real
error so a happy-path recapture cannot silently remove the coverage.

### Searching for a file is not reading it

`tool_call_touched_source` accepted `ReadFile | Grep | Glob | Bash`. The live capture runs
`find_by_name` with `Pattern: "evidence.txt"` and several `run_command` (`find -name`, `ls`,
`mdfind`) and NEVER opens the file. Every one of those satisfied the source, and `agy_02`
asserted that as proof the check works. The substring hole written down as a passing test.

FIXED: only `ToolKind::ReadFile` satisfies a named source. Consequence, stated rather than
hidden: `codex-exec-json` stamps every call `Bash`, so `required_sources` cannot be enforced
there at all. Rather than fake it, the gate now REFUSES named sources on parsers that cannot
classify reads (`PARSER_MODES_THAT_CLASSIFY_READS`), which is the fail-closed rule applied one
level finer. `sight_21`, `sight_22`, `sight_23`.

### `notebook_execution` was classified as a read

It EXECUTES. Doubly wrong: it could run side effects while looking like a read, and it could
satisfy a required source without opening it. Now `Bash`. Grok found it.

### Three of my RED IF comments were false. Antigravity found all three.

1. **`agy_04`**: claimed red if `result.response` stopped being preferred. The fixture's
   `text_delta` also spells "pong", so dropping the result event left it green. Now the two
   are made to DISAGREE, and both preference and fallback are asserted.
2. **`sight_19`**: claimed red if a third ungated success arm appeared. It would not: the gate
   count stays 2 and the new DONE still sits after the first gate. Now each DONE must have a
   gate within a bounded 9000-char window, following the `persist_deepseek_err_tokens`
   precedent. Measured distances are 6562 and 2865.
3. **`sight_15`**: claimed to guard against fuzzy matching, and only passed because I picked a
   deeply nested source. With source `/repo/src/lib.rs` the relative form `src/lib.rs` IS a
   substring of `crates/unrelated/src/lib.rs`. It hid the exact bug it named.
   FIXED both the test (shallow source, the case that actually breaks) and the code: matching
   is now on the quote-delimited JSON value `"<path>"`, not a bare substring. That also closes
   Grok's `<path>.bak` hole. Mutation-verified: restoring bare `contains` reds `sight_15`.

### The security hole, and my comment that lied about it

`sight_10` said a shell write is undetectable but "the read-only sandbox is what actually
prevents it". True for codex (`codex exec` without --full-auto is read-only). **FALSE for agy**:
`agy_sandbox_enabled()` defaults to false and dispatch passes
`--dangerously-skip-permissions`, so an agy reviewer can overwrite the repository through
`run_command`, invisible to the no-touch check and unrestrained by the OS. Antigravity found
this in its own backend.

Recorded as `AGENTS_WITH_NO_WRITE_CONTAINMENT` with `sight_24`. Documentation with a test
attached, not enforcement: refusing agy reviews outright is the wrong trade while it is the
only Gemini backend. Closing it properly needs a read-only profile for review dispatches, or
denying agy's write tools at spawn. NEITHER IS BUILT.

### `sight_20` did not prove what it claimed

It builds a `ToolCallRecord` by hand, so it proves the gate accepts a well-formed record and
nothing about whether Antigravity can produce one. Antigravity said so about its own
regression guard.

`sight_25` closes it: spawn agy through `build_agy_invocation`, parse the real stdout, hand the
result to the real `enforce_reviewer_sight`. It lives in the unit module because the gate is
private to the binary. **Ran live and passed.**

### Still open

- Plain-text fallback (`TRIUMVIRATE_AGY_OUTPUT=text`) is no longer exercised by subprocess
  tests. Antigravity flagged the coverage loss and it is real.
- `call_mcp_tool`, `invoke_subagent`, `browser_subagent` are `Unknown`: they can write and
  escape the no-touch check. Unclassifiable without guessing; containment is the answer.
- Fixtures go stale. If Google changes the wire format the offline suite stays green while
  production breaks. `e_agy_sight_02` and `sight_25` are the live guards, and they are opt-in.
- `require_sight` is still opt-in, and only one-shot `ask_agent` can set it.

39 groups green. Clippy clean except the pre-existing `pantheon` warning.
