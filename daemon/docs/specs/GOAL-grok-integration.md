# GOAL: Grok as a first-class peer

**ANSWERED by the owner 2026-08-30.** Scope is FULL Codex parity (30 files), all three pre-existing defects are in
scope, and context control is a deliberately owned MCP set rather than inheritance or blanket isolation.

**Draft for the owner to correct.** Replaces the first attempt, which said "works the same way Codex does" without
saying which Codex. Codex touches 30 files, Antigravity touches 9, and the gap between them is fleet swarms, ABE, and
token economics. That ambiguity is what this rewrite fixes.

**Read first:** `grok-integration-spec.md` (the vendor guide, verified), `grok-integration-test-plan.md` (the pyramid),
and `findings/grok-*.md` (four evidence docs from live probing).

---

## The goal in one sentence

**Grok is a peer I can consult, name in a peer review, and drive from a named session, with its cost visible and its
context under Triumvirate's control.**

## Scope: IN

1. **Consult path.** `ask_agent` with `grok` or any alias returns a parsed answer with tokens and session id.
2. **Named sessions.** `spawn_session` / `ask_session` resume correctly across turns, using the parser's
   `end.sessionId` and never a bare `--resume`.
3. **Peer review membership.** Grok is a first-class reviewer. **"Peer review" means all three CLI peers: Codex,
   Antigravity, Grok.** DeepSeek joins only when explicitly asked for.
4. **Alias/daemon surface.** `spawn_daemon target:"grok"` works, which means fixing `aliases.rs:166`.
5. **Context config: ALREADY DONE, do not redo.** Grok owns its own MCP set in `~/.grok/config.toml`, with
   `[compat.claude] mcps/skills/hooks/agents/rules = false` switching off the `~/.claude.json` inheritance. Active
   servers: graphiti-memory, gemini, github, triumvirate. gdrive disabled in favour of the `gog` CLI. **Do not
   reconfigure this.** The daemon spawns `grok` and inherits that config.
6. **Telemetry.** PostHog records grok generations with tokens **and cost**, captured from `end.total_cost_usd`.
7. **Doctor and docs.** `triumvirate doctor` reports binary, version, and which auth is in use, spending no tokens.
   README documents every `TRIUMVIRATE_GROK_*` variable.
8. **The full test pyramid** in `grok-integration-test-plan.md`: 26 unit, 15 integration against a mock binary,
   6 end-to-end behind `TRIUMVIRATE_LIVE_GROK=1`.

9. **Fleet worktree swarms.** `fleet/orchestrator.rs`, `fleet/tasks.rs`. Grok is a spawnable fleet worker.
10. **ABE worker type.** `abe/task_tracker.rs`.
11. **Token-economics.** All five files, including a `~/.grok` session scanner. Grok is the best candidate here
    because it self-reports `total_cost_usd` per turn.
12. **Shared types and transport surfaces.** `shared-types/{api,lib,streaming}.rs`, `daemon-core/observability.rs`,
    `daemon-http/lib.rs`, `agent-adapter/stuck.rs`.

**Owner's ruling: full Codex parity.** Codex appears in 30 files. Grok should appear in the equivalent set. Track
the delta explicitly with `rg '"codex"' --include='*.rs' crates/` against the same query for `"grok"`.

## Scope: OUT, deliberately, and named so it is a decision

- ACP transport via `grok agent stdio`. `streaming-json` is already ACP session updates over stdout, so this is a
  transport change, not a protocol adoption. Revisit if the leader socket proves cheaper than per-turn spawn.
- Driving Triumvirate *from* Grok. The orchestrator inversion stays parked.

## Pre-existing defects to fix on the way through

These are not Grok's fault. They block or corrupt Grok's integration, so they are in scope.

1. **`aliases.rs:166`** maps only `gemini` and `codex`. `spawn_daemon` is broken for **DeepSeek and Claude** today.
   Fix it from `supported_agent_names()` so it cannot drift again.
2. **`agent_exec.rs:2977`** asserts against a `schedule_len_for` closure the test defines itself, so it passes
   regardless of the real scheduler. Replace with an assertion against `execute_ask_agent`.
3. **`agent_exec.rs:336`** comments that subscription siblings "cost exactly $0". True for marginal dollars, false
   for quota. Correct it, because it is the reasoning that would justify not tracking Grok spend.

## Non-negotiables

- **No test that cannot fail.** Every test must break for the reason it claims to guard. No reconstructions.
- **`cargo test` passes with no network and no `XAI_API_KEY`.** E2E is opt-in.
- **No regressions.** Gemini, Codex, DeepSeek and Claude behavior unchanged except where a shared list must grow.
- **Slice by slice**, each compiling with tests, committed separately. No single unreviewable diff.
- **State what is not verified** rather than letting silence imply it was.

## DONE WHEN

Each of these is a command whose failure is observable.

1. `cargo test` green across the workspace, no network, `XAI_API_KEY` unset.
2. `curl -s localhost:PORT/status | jq -r '.supported_agents[]'` includes `grok`, and equals
   `supported_agent_names()` exactly.
3. `ask_agent {agent: "supergrok", message: "reply with pong"}` returns `pong`.
4. `spawn_session` plus two `ask_session` calls: turn 2 demonstrably recalls turn 1, and the second invocation passed
   turn 1's parsed `end.sessionId` to `--resume`.
5. `spawn_daemon target:"grok"` succeeds, and the same call succeeds for `deepseek` and `claude`.
6. A peer review names Grok as a reviewer alongside Codex and Antigravity.
7. PostHog shows a grok generation carrying input tokens, output tokens, **and cost in USD**.
8. A live consult reports the **measured** context for the owned MCP set (github, triumvirate, graphiti, plus native)
   and it is materially below the inherited 67K. The number is recorded, not predicted.
9. `gog` is reachable from a Grok consult via `run_terminal_command`, proving Drive access survives dropping the
   gdrive MCP and its 116 schemas.
10. Fleet can spawn a Grok worker; ABE recognizes the worker type; token-economics attributes Grok spend.
11. `rg '"grok"'` covers the same surface set as `rg '"codex"'`, with any remaining gap listed as deferred.
9. `triumvirate doctor` reports grok's binary, version and auth kind, and spends zero tokens.
10. A **tool-using** fixture is committed, closing the gap that the current fixtures contain no `tool_call` events.
11. Every deferred item in "Scope: OUT" is written down as deferred, with what it was for and what replaces it.

## Answered

- **Scope:** full Codex parity, 30 files.
- **Defects:** fix all three.
- **Context:** owned MCP set (github, triumvirate, graphiti-memory) plus `gog` for Drive. Not inheritance, not
  blanket isolation.
- **`triumvirate` stays in Grok's set**, so Grok can consult the other peers. Once this lands Grok can reach itself
  recursively; guard against unbounded self-dispatch rather than removing the capability.

## Still open

- **ANSWERED: Grok is a DEFAULT peer-review reviewer.** `peer-review/src/lib.rs:42` becomes
  `["codex", "gemini", "grok", "claude"]`.
- The earlier "curation leak" was not a leak. `[compat.claude] mcps = false` is the real switch and it is already
  set. The `HOME` override explored during investigation is obsolete; do not implement it.
