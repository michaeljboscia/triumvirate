# CLI path audit, 2026-09-13

## Final matrix, 2026-09-13 23:5x ET, daemon at commit 0647a20 plus the residual fix

Full 23-probe run on the final daemon (`progress-final.jsonl`), fleet probes each in their own
repo (D-015), plus a Codex fleet cancelled while running.

| Path | Codex | Gemini (Antigravity) | Grok |
|---|---|---|---|
| CLI run directly | OK | OK | not probed |
| Gemini CLI (`gemini`) | | DEAD, Google retired the tier | |
| `ask_agent` consult | OK | OK | OK |
| `ask_agent` with `required_sources` | OK | OK | OK |
| `review_agent` | OK | OK | OK |
| Session, two turns, state carried | OK | not carried, by design (no resume on agy) | OK (one flaky run: the model chose a ledger tool over answering; passed on rerun) |
| `query_antigravity` and alias | | OK | |
| `dispatch_codex` plain | spawned, wrote the file (probe prompt forbids the commit ABE requires) | | |
| `dispatch_codex_worktree` | OK, committed, 65s | | |
| `fleet_spawn` to ledger `done` | OK, 70s | OK, 30s | OK, 20s |
| `fleet_cancel` on a running worker | kills wrapper and binary, ledger `cancelled` | | kills worker |
| Codex quota failure | error names "You've hit your usage limit" on the 502, outbox, dead drop | | |

Still open: D-014 (agy quota detectors over-match log noise), D-015 (task ids collide across
fleets in one repo), D-016 (fleet_status after a daemon restart).

## Original run, 2026-09-13 afternoon

Every Triumvirate path that spawns an agent CLI, probed end to end through a fresh
`triumvirate mcp` on the installed binary (3.9.0, commit 9843c7e), plus the CLIs run
directly as a baseline. Raw results: `progress.jsonl`, one line per probe. Harness:
`scripts/audit-cli-paths.py`. Stopped before the gemini and grok fleet probes ran.

## Matrix

| Path | Codex | Gemini (Antigravity) | Grok |
|---|---|---|---|
| CLI run directly | OK 5s | OK 5s | not probed |
| Gemini CLI (`gemini`) run directly | | DEAD: IneligibleTierError, Google retired the tier | |
| `ask_agent` consult | OK 7s | OK 5s | OK 8s |
| `ask_agent` with `required_sources` (review sandbox) | OK 12s | OK 10s | OK 15s |
| `review_agent` | OK 12s | OK 8s | OK 9s |
| `spawn_session` then two `ask_session` turns | OK, state carried | FAIL, state not carried | OK, state carried |
| `query_antigravity` and `query_gemini` alias | | FAIL, 429 capacity after 4 retries | |
| `dispatch_codex` (ABE, plain) | spawned and wrote the file (see note) | | |
| `dispatch_codex_worktree` (ABE, worktree) | OK, committed, 70s | | |
| `fleet_spawn` | FAIL, see below | not run | not run |

Note on `dispatch_codex`: the worker ran on the fixed argv and wrote the file. ABE then marked
the task failed because the probe prompt said "do not commit" and ABE requires a commit. That is
the probe's fault, not the path's.

## What the audit proves

1. The codex 0.154 argv fix (commit 9843c7e) holds on every surface it touched. Before it,
   `dispatch_codex`, `dispatch_codex_worktree` and the fleet codex worker died at argv parse.
   Now all three spawn and run.
2. Antigravity works as the Gemini seat for consult, review and review_agent.
3. Grok works on every probed path.

## What is broken, with the cause where known

### B1. Fleet codex spawn never leaves `spawning`
`fleet_spawn` with `agents=["codex"]`, `dry_run=false`, `wait=false` created the worktree
`.triumvirate/worktrees/fleet-<id>-T-001-codex` and a `codex exec -- ...` worker that stayed
alive for 13 minutes on a task that should take one. `fleet_status` reported
`{"state":"spawning","worktree_paths":[]}` for the whole 600s budget. `fleet_cancel` did not
stop the worker; it was killed by hand. Cause not established. The worker was alive, so this
is a state-tracking or completion-detection gap in `fleet/src/orchestrator.rs`, not the argv.

### B2. Antigravity session resume does not carry state
Turn one: "Remember PELICAN. Reply OK" was answered. Turn two: "what word?" produced a ledger
lookup instead of the word. Codex and Grok both recalled it on the same probe. Either the
resume handle is not passed on the agy path or each turn starts a fresh conversation.

### B3. Antigravity 429 capacity under a burst
`query_antigravity` failed after four retries with "Antigravity 429 capacity", following a run
of six Antigravity calls in under two minutes. Google-side throttling. Likeliest cause of the
empty responses recorded as "stream-json result carried no response text" (D-012), which
were also preceded by bursts. The daemon does not back off and does not log the status.

### B4. Gemini CLI is dead at the account level
`gemini -p` fails auth with `IneligibleTierError`. Triumvirate's primary Gemini route is agy,
so only the degraded route's first hop (`gemini-cli`) is affected, and it now fails every time
before the codex hop gets its turn. Anything else on this machine that shells out to `gemini`
is broken.

### B5. Failures report the last fallback hop, not the first failure (D-012)
Not reproduced here (no fallback fired on these probes) but established from the daemon log for
the masterFFL request on 2026-09-13T18:03Z. Gemini failed in agy; the caller was told
"codex connector timed out". Two sessions misdiagnosed it two different ways.

### B6. The 180s connector timeout is too short for a sight-gated review
The masterFFL codex review ran three attempts of 180s and failed at 543s. The same signature
appears on 2026-09-11 before any change. The sight gate (mandatory since 2026-09-01) makes a
review read files and run commands; the timeout was sized for one-line consults.

## Not broken, despite what other sessions reported

- "Gemini alias mis-routes to Codex": no. The alias resolves to agy. See B5.
- "84 stuck jobs": no. That is the daemon's list of unacknowledged dead-drop fallback
  tickets (87 files, 50 from 2026-09-02). Nothing is queued behind them.
- "100 active sessions": named session records in the registry, not processes.

## Fix order

1. B5, so every later failure is diagnosable from its own error.
2. B6, a review timeout separate from the consult timeout.
3. B1, fleet state tracking and cancel.
4. B2, agy resume.
5. B3, backoff on 429 and treat an empty result as a failure with its status attached.
6. B4, drop `gemini-cli` from `TRIUMVIRATE_GEMINI_DEGRADED_ROUTE`'s default.
