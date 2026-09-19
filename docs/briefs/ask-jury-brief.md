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

**Shape.** `ask_jury` is a bridge-side fan-out over the existing `ask_agent` path, one strict call per seat. There is no daemon-side jury endpoint, so there is one implementation. The defence is two layers: every seat is dispatched `strict_agent: true`, and every reply is checked for who answered. The second layer is what survives version skew: a daemon older than `strict_agent` ignores the unknown field and substitutes, and that reply lands as `invalid` with its text withheld.

**Deviations.**
- `unanimous` means EVERY requested seat answered, cast a verdict, and agreed. Item 3 ("all valid seats") and the first acceptance line contradicted each other on two of three; acceptance wins. Two of three is `majority`.
- `outcome` is one of `unanimous`, `majority`, `split`, `no_quorum`, counted against seats REQUESTED. The draft's `split` ("all different") left two disagreeing seats and a lone answer with no name.
- `require_sight?: [...paths]` became `require_sight: bool` plus `required_sources: [paths]`, the same two fields with the same types as `ask_agent`. A non-empty `required_sources` implies the gate.
- `verdict_json_pointer` (RFC 6901) was added beside `verdict_regex` and wins over it. `verdict_source` in the result says which reader was used, so a false split from the first-line default is diagnosable.
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
- `model` is always absent. `ask_agent` does not report it, and it is not guessed. So a same-agent vote cast by a different MODEL (the gemini-cli faildown chain) is invisible to the jury. Closing it needs `AskAgentResponse` to carry the model.
- An absent `answered_by_agent` is read as "the asked agent answered", because that is what `ask_agent` sends on the normal path. The check catches substitution the daemon admits to.
- All seats share one `cwd`. Nothing stops seat A reading a file seat B already wrote. Blindness between seats is the caller's to arrange (separate directories, or outputs checked after all seats return). For the same reason the output flag is named `changed_during_call` and not `written_this_call`: it proves the file changed while the call ran, not which seat changed it.
- Verdict extraction reads a reply, and a reply can be written to mislead. The JSON pointer takes the last object that carries it and the regex takes its last match, so a preamble or an echoed prompt cannot cast the vote. Nothing defends against a seat whose first line contradicts its own conclusion under the `first_line` default: name a pointer or a regex when the verdicts matter.
- `approve` and `approved` are different verdicts. Normalization lowercases and strips wrapping and trailing punctuation, and does not stem. Two seats that agree in substance and differ in spelling read as a split, which is visible and wrong in the safe direction.
- A mandatory-peer-review rejection surfaces as `unavailable`. It does not rewrite a vote, but it can cost a quorum.
- The scheduled 300s health probe still does not close the breaker on success. Owner's call.

**Not yet deployed.** The running daemon is the old binary until `scripts/install.sh`. Until then `ask_jury` is not callable and live `ask_agent` still substitutes.
