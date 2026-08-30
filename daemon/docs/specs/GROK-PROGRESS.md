# Grok integration progress

**RESUME HERE. Authoritative state. Update after every slice, before starting the next.**
Goal: `GOAL-grok-integration.md` · Plan: `grok-integration-test-plan.md` · Evidence: `findings/grok-*.md`

## Slice status

| Slice | State | Commit | Peers reviewed |
|---|---|---|---|
| A identity + `supported_agent_names()` | **DONE** | `3893d27` | not reviewed (mechanical) |
| B invocation builder | CODE DONE, 14 tests green, awaiting peer review | | |
| C parser | | | |
| D spawn/dispatch + defects 2,3 | | | |
| E doctor/README/aliases + defect 1 | | | |
| F mock binary + integration | | | |
| G peer-review panel | | | |
| H fleet | | | |
| I ABE | | | |
| J token-economics | | | |
| K shared-types sweep | | | |
| L e2e | | | |

## Rulings already made, do not relitigate

- **Full Codex parity**, 30 files. Not just consult.
- **Grok is a DEFAULT peer-review reviewer.** Panel becomes `codex, gemini, grok, claude`.
- **MCP config is DONE and must not be touched.** `~/.grok/config.toml` has
  `[compat.claude] mcps/skills/hooks/agents/rules = false`, which is the real switch for the `~/.claude.json`
  inheritance. Active servers: graphiti-memory, gemini, github, triumvirate. gdrive dropped for the `gog` CLI.
- **The `HOME` override explored during investigation is OBSOLETE.** Do not implement it. `[compat.claude]` supersedes it.
- **No cost-control framing.** Flat subscription, so marginal dollar cost is zero. `total_cost_usd` is a usage signal.
- **"Peer review" = Codex + Antigravity + Grok.** DeepSeek only when explicitly asked.

## Facts established by live probing

- `streaming-json` is documented by the binary as "one ACP session update per line".
- `-s/--session-id` is NEW sessions only, must not already exist, never resumes.
- **`-r/--resume` takes an OPTIONAL argument**, so a bare `-r` means "most recent in cwd", the same cross-talk the
  spec bans for `--continue`. Builder must `bail` on an empty id.
- `--resume` also accepts a **title**, so passing a session *name* resolves silently wrong.
- `-s` and `-r` are mutually exclusive unless `--fork-session` is passed.
- Default `--output-format` is `plain`, so the flag must always be explicit.
- `end.sessionId` returns the `-s` uuid unchanged. REQ-GROK-007 confirmed.
- `input_tokens` and `cache_read_input_tokens` are SEPARATE counters. Total context is their sum.
- Fixtures committed: `agent-adapter/tests/fixtures/grok-streaming-{20260830,lean-20260830,isolated-20260830}.jsonl`.
- **Fixture gap:** none contain `tool_call` events. Tool mapping is spec-derived, not observed.

## Pre-existing defects in scope

1. `aliases.rs:166` maps only gemini and codex, so `spawn_daemon` is broken for deepseek and claude. Slice E.
2. `agent_exec.rs:2977` asserts against a closure it defines itself. Slice D.
3. `agent_exec.rs:336` claims subscription calls cost $0. True for dollars, false for quota. Slice D.
