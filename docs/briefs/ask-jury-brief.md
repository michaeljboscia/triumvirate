# Brief: `ask_jury`, a fan-out tool that never substitutes a seat

Owner's proposal, 2026-09-19 morning ("Maybe add 'ask jury' instead?"), after the overnight mneme fill.

## The failure this fixes
On 2026-09-19 the mneme jury (three blind labelers: Codex, Grok, Gemini) ran 98 parts through `ask_agent`. When the Gemini seat's backend (Antigravity) hit its per-user quota, the bridge did what it is built to do for Q&A: it opened a circuit breaker and routed the Gemini calls to Codex, returning success with a warning prefix and `answered_by_agent: "codex"` in the metadata. For a jury that is a silent corruption: a "unanimous" vote would have been Codex agreeing with itself. Seven parts were caught only because the caller inspected the metadata; the quota reset did not close the breaker, and `query_gemini` is an alias of the same backend, so there was no honest route to a Gemini vote for the rest of the night. HNT entry: "Gemini seat silently answered by Codex under quota degradation" (2026-09-19).

## What `ask_jury` does
One call, one brief, N seats (default codex, grok, gemini). Each seat runs the same message with the same `cwd` and sight requirements. The tool:
1. Never substitutes. If a seat's backend is unavailable (quota, breaker, timeout), that seat is returned as `{status: "unavailable", reason}`; the tool does not route to another agent and does not retry on a different backend. Degradation is a caller decision, not a bridge decision.
2. Returns provenance per seat: `agent`, `answered_by_agent`, `answered_by_backend`, `model` when known, `request_id`, and the response. `answered_by_agent` must equal `agent` or the seat is marked `invalid`.
3. Tallies: `seats_answered`, `unanimous` (all valid seats gave the same normalized verdict), `majority` (verdict and count), `split` (all different). The verdict is extracted by an optional caller-supplied regex or JSON path; default is the first line of the reply.
4. Optional per-seat write targets (`outputs: {codex: path, ...}`) so labelers can write files as today; the tool verifies each file exists and parses as JSON when `expect_json: true`, and reports row counts, never contents.
5. Runs seats in parallel with a per-seat timeout; a slow seat does not block the others' provenance from being reported.
6. Records one ledger entry with the tally and the seat provenance, and one PostHog event per seat with `answered_by_backend` and `status`, so degradation is visible on a dashboard instead of in a warning prefix.

## Interface (draft)
```
ask_jury({
  message: string,
  cwd?: string,
  seats?: ["codex","grok","gemini"],
  require_sight?: [...paths],
  grok_depth?: "quick"|"deep",
  outputs?: {codex?: path, grok?: path, gemini?: path},
  expect_json?: boolean,
  verdict_regex?: string,
  timeout_s?: number,
  context?: string
}) -> {
  seats: {codex: {status, answered_by_agent, answered_by_backend, model, request_id, response, output_rows?}, ...},
  seats_answered: n, unanimous: bool, majority?: {verdict, count}, split: bool,
  ledger_id
}
```

## Also
- `ask_agent` gets a `strict_agent: true` option that turns the same no-substitution behavior on for single calls; the default stays as today for Q&A.
- The circuit breaker should expose a `probe` so a caller can ask the bridge to re-test a backend after a quota reset instead of waiting for the breaker's own timer.

## Acceptance
- A jury run with one seat's backend forced unavailable returns that seat as `unavailable`, never as answered by another agent; `unanimous` is false; the other two seats' provenance is intact.
- `answered_by_agent != agent` cannot occur in an `ask_jury` result (test).
- The mneme scripts (`ops/jury_resolve.py`) can consume the result without reading label contents (row counts only).

## Where the code lives
Degradation and the circuit breaker: `daemon/crates/triumvirate/src/agent_exec.rs` (fields `answered_by_agent`, `degraded_from_backend`, breaker states); shared result types in `daemon/crates/shared-types/src/lib.rs`. The Antigravity backend integration notes are in `daemon/docs/specs/agy-integration-HANDOFF.md`.

## As built (2026-09-19, branch feat/ask-jury)

Where the build departs from the draft above, and why. The draft is left as written.

**Shape.** `ask_jury` is a bridge-side fan-out over the existing `ask_agent` path, one strict call per seat. There is no daemon-side jury endpoint, so there is one implementation.

The defence is three layers, and the third exists because the second failed open. Every seat is dispatched `strict_agent: true`. Every reply is checked for who answered. And the daemon must ACKNOWLEDGE that it took the strict path, by returning `strict_agent_honored: true`. Without the third, the whole guarantee rested on an absence: no `answered_by_agent` was read as "the asked agent answered". Codex put it exactly right in review: absence must be treated as unverifiable. A daemon that never heard of `strict_agent` ignores the unknown field, substitutes, and if it also omits the provenance fields the corrupted vote reads as clean. A seat that cannot be verified is now `invalid`, which is a different answer from `answered`, and the tally treats it as such.

**This makes installing the daemon mandatory, not optional.** Run against an older daemon, every seat comes back `invalid` with "did not acknowledge strict_agent" and the outcome is `no_quorum`. That is the intended behavior: refusing to count a vote it cannot verify is the entire point of the tool.

**Deviations.**
- `unanimous` means EVERY requested seat answered, cast a verdict, and agreed. Item 3 ("all valid seats") and the first acceptance line contradicted each other on two of three; acceptance wins. Two of three is `majority`.
- `outcome` is one of `unanimous`, `majority`, `split`, `no_quorum`, counted against seats REQUESTED. The draft's `split` ("all different") left two disagreeing seats and a lone answer with no name.
- `require_sight?: [...paths]` became `require_sight: bool` plus `required_sources: [paths]`, the same two fields with the same types as `ask_agent`. A non-empty `required_sources` implies the gate.
- `verdict_json_pointer` (RFC 6901) was added beside `verdict_regex` and wins over it. `verdict_source` in the result says which reader was used, so a false split from the first-line default is diagnosable.
- `model` is reported when the dispatch named one, and under `strict_agent` the gemini-cli faildown chain collapses to a single attempt on the primary model. The draft asked for "model when known"; an earlier version of this section wrote it off as permanently absent. It is not: `gemini` could answer on any of four models with nothing in the reply to say which, so a seat asked for one voter could get another (Grok).
- `ledger_id` became `jury_id`. `ledger_record` returns the string "ok", not an id. The id is carried in the record; find a run with `ledger_query`, not `ledger_session`.
- `context` is not a field. The bridge injects it into every tool and strips it before dispatch.
- `grok_depth` is `fast|deep`, the existing enum, not `quick|deep`.
- Status `timeout` was added beside `unavailable` and `invalid`.
- The ledger record and the `tv_jury_seat` PostHog event carry provenance and counts only. No reply text and no verdict text: for a labelling jury the verdict IS the label. A seat's failure `reason` is the daemon's own error text, and the daemon quotes what it rejected, so the journal and the ledger get the first line only, cut at any quoting marker and capped. The caller still gets the whole reason.
- `split` is "two or more verdicts cast and none has a majority", not the draft's "all different". Two against two is a split, and calling it anything else would be calling a tie agreement.
- Outputs report `format` (`json`, `jsonl`, `embedded_json`) beside `rows`, so a caller can insist on strict JSON instead of accepting an array fished out of prose. `rows` counts JSON values and is never a claim that a value is a well-formed anything.
- Two seats told to write one file is rejected before anything is spent, by file identity rather than by spelling. So is one seat named twice through an alias.
- Every seat is dispatched `own_lane: true`, a new `ask_agent` field. The daemon serializes `ask_agent` per project, and all seats share a `cwd`, so without it the "parallel" seats ran one after another and the last seat's timeout was spent waiting in the queue.

**The probe.** `breaker_probe` (MCP tool, `POST /agy/breaker/probe`) closes the agy breaker only on the exact expected answer, and a failed probe changes nothing in any phase. Building it surfaced the real cause of "the quota reset did not close the breaker": see the 2026-09-19 entry in `daemon/docs/bugs/OPEN.md`.

**Known limits, not closed.**
- **A timeout ends the wait, not the work.** The daemon does not cancel on client disconnect, so a seat reported `timeout` may still be running and may write its output file afterwards (Codex). Its output check says so rather than claiming the seat wrote nothing. A caller that re-runs a timed-out seat should expect the first one's write to still land.
- **Seats in one project no longer exclude each other.** `own_lane` is what makes the fan-out parallel, and parallel seats sharing a `cwd` can race on a file neither declared (Codex). Worth stating precisely: the project lane was never a system-wide "one agent per repo" rule, because the fleet has always spawned members in parallel in one repo without taking it. The rule this removes is narrower than it looks, and it is removed deliberately: a jury that runs its seats one at a time spends the last seat's timeout waiting in a queue. Give seats separate directories when they write.
- **The queue registry is never pruned**, and a lane per agent multiplies its entries by the number of agents. The growth is bounded by the agent list, which is why `own_lane` is a bool and not a caller-supplied lane name, but the underlying leak (one entry per distinct `cwd`, forever) predates this and is not fixed here.
- All seats share one `cwd`. Nothing stops seat A reading a file seat B already wrote. Blindness between seats is the caller's to arrange (separate directories, or outputs checked after all seats return). For the same reason the output flag is named `changed_during_call` and not `written_this_call`: it proves the file changed while the call ran, not which seat changed it.
- Verdict extraction reads a reply, and a reply can be written to mislead. The JSON pointer takes the last object that carries it and the regex takes its last match, so a preamble or an echoed prompt cannot cast the vote. Nothing defends against a seat whose first line contradicts its own conclusion under the `first_line` default: name a pointer or a regex when the verdicts matter.
- `approve` and `approved` are different verdicts. Normalization lowercases and strips wrapping and trailing punctuation, and does not stem. Two seats that agree in substance and differ in spelling read as a split, which is visible and wrong in the safe direction.
- A mandatory-peer-review rejection surfaces as `unavailable`. It does not rewrite a vote, but it can cost a quorum.
- The scheduled 300s health probe still does not close the breaker on success. Owner's call.

**Not yet deployed.** The running daemon is the old binary until `scripts/install.sh`. Until then `ask_jury` is not callable and live `ask_agent` still substitutes.

## Addendum 2026-09-19: the breaker outlived the outage
Observed after the brief was written: the agy breaker kept the Gemini seat unavailable for hours after the quota had reset, because it closes on its own timer rather than on a probe. Calling `agy --print` directly worked immediately while the bridge still refused. So the probe in the "Also" section is not a nicety; without it the bridge reports a capability as gone when it is available, and `ask_jury` would return `unavailable` seats that are in fact reachable. Acceptance addition: after a breaker opens, a caller-invoked probe must be able to close it on a single successful call, and `ask_jury` must attempt that probe once before marking a seat unavailable.

**As built, answering the addendum.** Both halves are implemented and tested.

1. *A caller-invoked probe closes the breaker on a single successful call.* `breaker_probe` does, and only on the exact expected answer: the health classification calls any non-empty reply healthy, so a quota sentence printed on a zero exit would otherwise have reopened the gate on a message saying the gate was shut.
2. *`ask_jury` attempts that probe once before marking a seat unavailable.* Once per RUN, not once per seat: three seats failing on one breaker is one outage, and three probes would spend three live calls to learn the same fact. The probe is spent only on a failure a probe could plausibly fix, which means the agy-backed `gemini` seat failing on the breaker or on quota. A probe that closes the breaker buys a retry of exactly those seats; a probe that does not close it, or that cannot run at all, leaves them `unavailable` and the jury still returns. The result carries `breaker_probe` and `seats_retried_after_probe`, so a recovered seat is never silently indistinguishable from one that answered first time.

Diagnosis of the addendum's observation, from the code: the breaker was not merely waiting out a timer. The 300s health probe shared the request runner, so its quota exits fed the breaker and its retry claimed the single half-open slot, re-tripping and doubling the cooldown toward the five-hour cap. The monitor was holding the gate shut. Fixed by `agy::BreakerRole`; see the 2026-09-19 entry in `daemon/docs/bugs/OPEN.md`.
