# D-027 peer panel, seat 2: Grok deep (class lens)
Artifact frozen at 9408370. 73 tool calls, third attempt (1st: I edited the file mid-review; 2nd: sight gate false-reject, now D-028).
Verdict: the three instances are closed, the CLASS is open.
Against THIS diff (all fixed in the follow-up commit):
- cancel-before-launch return never called finalize -> same stranding hole on the other exit.
- a panicking worker reaches no exit at all; `let _ = jh.await` swallowed it; task stays in_progress.
- terminal count `.unwrap_or(1)` fails closed SILENTLY.
- the new pin had no negative control; `assert_ne!(state,"running")` passed on a fleet stuck in `merging`.
- `cargo test <filter> --ignored` exits 0 when the filter matches NOTHING: rename the test and the script prints PASS.
- OPEN.md in the same diff still claimed the breaker-open path had no test.
Class instances NOT fixed here (filed):
- D-029 complete_fleet stamps its own review gate Approved then merges (fleet/src/orchestrator.rs:923, merge.rs:82).
- D-030 review verdicts cannot name who answered (gemini_query.rs:61/76, agent_exec.rs:2966/2981/1611, peer-review/src/lib.rs:50/339, main.rs:1238/1248).
- Fleet reduces the route to "contains codex" while the ask path walks an ordered chain; with `gemini-cli,codex` fleet launches codex immediately and `tasks.assigned_agent` still says gemini (orchestrator.rs:287, 1162).
- Model faildown across 4 gemini models is invisible to non-strict callers (agent_exec.rs:4358).
- Session reuse drops identity: ask_session returns only response.response (inter_agent.rs:203-232).
- agy_resilience.rs:9-12 and :363 module docs still instruct the next editor to route to codex.
