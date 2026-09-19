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
