# D-027 peer panel, seat 1: Codex (coverage lens)
Artifact: D027_REVIEW_DIFF.patch frozen at a70dd60. 59 tool calls. Verdict: REJECT.
Findings summary (full text in session transcript):
1. CRITICAL fleet early return (orchestrator.rs:391/428) bypasses terminal-count + complete_fleet() at :794. Fleet stays `running`, no fleet_failed event, merge/completion never runs. SQLite update error discarded so row may stay in_progress.
2. HIGH peer-review aliases not canonicalized (peer-review/src/lib.rs:50, :339) -> antigravity vs gemini lets an agent review itself.
3. HIGH query_antigravity_review (mcp-tools/src/gemini_query.rs:61,:71) drops provenance; a substituted codex APPROVE becomes verdict Clean.
4. HIGH legacy code_review hardcodes author_agent "codex" (main.rs:1238/1248).
5. MED mandatory peer review does not set strict_agent (agent_exec.rs:2966,:2979); agy->gemini-cli hop has no prefix (:1611).
6. MED named sessions erase provenance (inter_agent.rs:231, main.rs:2634, aliases.rs:221).
7. MED streaming TurnCompleted reports requested agent (streaming.rs:49).
8. MED new ledger event semantics: agent="gemini" though no gemini ran (orchestrator.rs:419).
Clean: ask_agent, review_agent, ask_jury, /ask-agent, alias normalization itself.
Shared reader: safe, fresh env read, no cache. Notes policy divergence (ask = ordered chain, fleet = contains-codex) and that the test fixture removes the var instead of restoring it (orchestrator.rs:1312).
