# Triumvirate recovery plan, 2026-09-13

Goal: the review path is trustworthy again. A failed review tells you which component failed
and why, a compliant reviewer is never rejected by the gate, and a real review has time to
finish. Everything else waits.

Evidence base: `docs/audits/2026-09-13-cli-paths/SUMMARY.md` and `progress.jsonl`.
Rerun after every step: `python3 scripts/audit-cli-paths.py --repo <throwaway repo> --rerun-all`.
Every step goes through the three-peer panel before commit, artifact frozen in `.review/`.

## What is already fixed
- Codex 0.154 argv on all four surfaces (commit 9843c7e, on the running daemon). Audit
  confirms consult, review, ABE plain, ABE worktree all spawn and run.
- Step 1 and Step 2, 2026-09-13 evening: failure chain, codex stdout/stderr in the error,
  Antigravity status on empty results, bridge body excerpt 2000 chars, sight-gated calls one
  attempt on `review_timeout()` 840s. Live-verified on a real Codex quota failure. Panel:
  antigravity and grok in; codex seat pending its quota reset (7:03 PM ET).
- Found during Step 1: Codex was over its usage limit all afternoon, hidden behind "exited
  with status 1" (D-013). Instant repeated failures with exit 1 are a quota or auth failure,
  not a timeout.
- Step 3, 2026-09-13 evening: a shell command that reads a file is a ReadFile in every
  adapter (grok, agy, claude, gemini) via one shared classifier, so the class is closed. The
  gate binds a shell read to its operand through the strict cat/nl parser: pipes, redirects,
  comments, subshells, second operands, output-suppressing flags, and a `description` naming
  the source all fail the whole-read check. Live: grok `cat` passes, `head -5` rejected as PART.
  Panel: antigravity (three bypasses, then `xxd -l0`) and grok (class across adapters, evidence
  strings, FileRead consumers) in; codex seat pending quota.

- Steps 4 and 5, 2026-09-13 evening: a quota/429 from Antigravity (non-zero exit, or an
  empty result with a quota line in the log, on stderr, or on stdout) is retried after a
  backoff (default 15s then 45s), the breaker sees every 429, the concurrency slot is held per
  attempt and not across the sleep, and the whole call ends by the connector deadline. The
  degraded route default is `codex`, and the primary Gemini backend defaults to agy (the
  gemini-cli default was the four-day-outage shape). Panel: antigravity (slot starvation,
  breaker regression, deadline overrun, all fixed) and grok (stderr dropped on zero exit,
  fixed; backend default, fixed; detectors over-match glog thread ids and doRefreshQuota
  lines, noted, not changed). Codex seat pending quota.

- Step 6, 2026-09-13 evening: four causes found. (1) Worker stdout/stderr were piped and
  never read, so a chatty worker deadlocked on a full pipe and `wait()` never returned; the
  non-agy path also had no timeout. Both pipes are now drained on their own tasks (8 KiB tail
  kept for the failure reason) and the wait is bounded by TRIUMVIRATE_FLEET_TASK_TIMEOUT_SECS
  (900s) with a kill. (2) `fleet_status` read an in-memory record written once at spawn; it
  now refreshes from the ledger (`fleets.state`, worktrees on disk). (3) `fleet_cancel`
  removed the record and killed nothing; a per-fleet pid registry lets it SIGTERM the workers
  and mark the ledger `cancelled`. (4) Task ids collide across fleets in one repo (D-015, not
  fixed here: it touches the merge queue and branch names). Live: a grok fleet reached `done`
  in 20s with its worktree reported, no worker survived cancel.

## Reorder after the panel review of this plan
Antigravity: Step 4 (Antigravity 429 under a burst) must precede any panel-gated step, or the
panel's three parallel calls trip the throttle. Grok: the panel path is sight-gated and so
already gets the review timeout; `query_gemini_review`, `ask_daemon`, `send_message` and
ungated `ask_session` stay on 180s x3 and are a later step. Also from Antigravity: the
`dispatch_codex` audit probe must let the worker commit, and "Done when" must include the
long-review test and the grok parser tests, not only the fast audit.

## Step 1. Errors carry the chain (D-012)
**Problem.** `agent_exec.rs:1489` formats the final error from the LAST hop. A Gemini request
that died in Antigravity reports "codex connector timed out". Two sessions misdiagnosed it.
**Change.**
- Collect every hop's failure into an ordered list as the route runs: primary attempt(s), then
  each degraded hop. Format oldest first: `gemini/agy: empty response (status=<x>) x2 ->
  degraded codex: connector timed out (180s)`.
- On the empty-response path in `agy.rs:344`, log the parsed `result.status` and any
  `RequestUserInput` tool event at WARN, and put the status into the error.
- Keep agy's `--log-file` when the run fails (rename to `<dead-drop id>-agy.log`), delete on
  success as now.
- The dead-drop `reason:` field carries the same chain.
**Verify.** Force an agy failure (set `TRIUMVIRATE_AGY_BIN` to a script that exits 1), call
`ask_agent gemini`, read the 502 body and the dead drop: agy is named first, codex second.
Existing negative control: the 2026-07-28 "timeout misreported as dead daemon" test pattern.
**Files.** `daemon/crates/triumvirate/src/agent_exec.rs`, `daemon/crates/triumvirate/src/agy.rs`.

## Step 2. A review timeout separate from the consult timeout (B6)
**Problem.** `connector_timeout()` at `agent_exec.rs:3089` is 180s for everything. A
sight-gated review reads files and runs commands and routinely needs longer. Three attempts of
180s then failure is the signature on 2026-09-11 and 2026-09-13.
**Change.** When `sight_required(req)` is true, use `TRIUMVIRATE_REVIEW_TIMEOUT_SECS`
(default 900s) and ONE attempt, not three. A review that fails once should return its error,
not be retried into the degraded route where the first failure gets buried. Keep 180s x3 for
consults.
**Verify.** `ask_agent codex` with `require_sight` on the masterFFL review brief returns a
verdict instead of "connector timed out". Audit `review_sight` probes still pass.
**Files.** `daemon/crates/triumvirate/src/agent_exec.rs`.

## Step 3. The gate sees a grok shell read (D-010)
**Problem.** `agent-adapter/src/grok.rs:104` maps every `run_terminal_command` to
`ToolKind::Bash`. The gate proves a named source only through `ToolKind::ReadFile`. Its own
rejection text tells the reviewer to `cat` the file. Three grok reviews were rejected after
doing exactly that.
**Change.** Reuse `agent_adapter::codex::command_read_range` for grok's
`run_terminal_command`: if the command is a whole-file `cat` or a `sed -n` window of a named
source, classify it `ReadFile` with the same args shape codex records, so the existing union
logic covers it. Then make the rejection text per adapter: only recommend what that adapter's
classifier can see.
**Verify.** `ask_agent grok` with `required_sources=[f]` and "cat f" passes; with "head -5 f"
is rejected as PART. Both as unit tests on the parser, and one live run.
**Files.** `daemon/crates/agent-adapter/src/grok.rs`, `daemon/crates/triumvirate/src/agent_exec.rs`.

## Step 4. Antigravity under load (B3)
**Problem.** Bursts of Antigravity calls hit Google 429 capacity. The daemon retries four times
with no backoff, then the empty result rides the degraded route into a misleading error.
**Change.** On a 429 or capacity match (`agy.rs:182` already detects it) back off 15s, 30s, 60s
before retrying. Treat an empty result with a non-SUCCESS status as a hard failure that names
the status; do not route it to codex, because a codex fallback cannot answer a Gemini-seat
review anyway. Serialize Antigravity dispatches through one semaphore so the panel's three
parallel calls do not become three parallel 429s.
**Verify.** Six `ask_agent gemini` calls in one minute all return, slower but returned. The
`query_antigravity` audit probe passes.
**Files.** `daemon/crates/triumvirate/src/agy.rs`.

## Step 5. Drop the dead first hop (B4)
**Problem.** Default `TRIUMVIRATE_GEMINI_DEGRADED_ROUTE` is `gemini-cli,codex`
(`agy.rs:231`). The Gemini CLI now fails auth every time (IneligibleTierError). The hop costs a
spawn and a failure before codex gets its turn.
**Change.** Default to `codex`. Leave the env var for operators who still have a tier.
**Verify.** Force an agy failure; the lifecycle shows one DEGRADED hop, not two.
**Files.** `daemon/crates/triumvirate/src/agy.rs`, `.claude.json` env if it overrides.

## Step 6. Fleet state tracking and cancel (B1)
**Problem.** Fleet codex worker ran 13 minutes; `fleet_status` said `spawning` with no
worktrees the whole time; `fleet_cancel` did not stop it. The worktree existed on disk.
**Change.** Not yet diagnosed. First find why the orchestrator never records the worktree it
created, then why cancel does not reach the child. Likely the same completion-detection gap as
the codex `--message` era, when workers died instantly and nobody noticed.
**Verify.** Audit `fleet_spawn.codex` reaches a terminal state within the task timeout and
`fleet_cancel` kills the process.
**Files.** `daemon/crates/fleet/src/orchestrator.rs`.

## Known and accepted, not in scope
- Antigravity session resume does not carry state. The agy connector forbids `--resume` and
  `--conversation` by design (`mcp-bridge/src/agy.rs:81`, REQ-012). Use one-shot for Gemini.
- Gemini CLI at the account level. Google's decision. Anything that shells out to `gemini`
  needs to shell out to `agy`.
- The 87 dead-drop tickets are an inbox. `fallback_gc` clears them; they block nothing.

## Order and dependencies
1 first: every later failure becomes diagnosable. 2 and 3 next, independent of each other:
together they stop rejecting or killing reviews that did the work. 4 and 5 after: Gemini seat
reliability. 6 last: fleet is not on the honesty path.

## Done when
`audit-cli-paths.py --rerun-all` passes every probe except `cli.gemini.print`, and a forced
Antigravity failure returns an error that names Antigravity first.
