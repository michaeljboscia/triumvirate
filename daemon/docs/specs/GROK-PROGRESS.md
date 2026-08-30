# Grok integration progress

**RESUME HERE. Authoritative state. Update after every slice, before starting the next.**
Goal: `GOAL-grok-integration.md` · Plan: `grok-integration-test-plan.md` · Evidence: `findings/grok-*.md`

## Slice status

| Slice | State | Commit | Peers reviewed |
|---|---|---|---|
| A identity + `supported_agent_names()` | **DONE** | `3893d27` | not reviewed (mechanical) |
| B invocation builder | **DONE**, 20 tests green | `68c10d3` + fixes | Codex, Antigravity (Grok hit max-turns) |
| C parser | **DONE**, 21 tests green vs real fixtures | `4c8d8d3` + fixes | Codex (5 defects found and fixed) |
| D spawn/dispatch + defects 2,3 | **DONE**, 179 tests green | `e41a55d` | awaiting peer review |
| E doctor/README/aliases + defect 1 | **DONE**, 222 tests green | | |
| F mock binary + integration | NEXT | | |
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

## Peer findings fixed in Slice B (do not reintroduce)

- **Flag injection via session id / cwd** (Codex). A value starting with `-` is parsed by clap as the next FLAG, so
  `--resume -c` becomes a bare `--resume` plus `--continue`. `validate_argv_value` rejects it. Tests u_b_15, u_b_16.
- **Clustered short flags** (Codex). `split('=')` catches `--model=x` but never `-rp` or `-mfoo`.
  `FORBIDDEN_SHORT_FLAGS` inspects every char of a short cluster. Test u_b_17.
- **20 missing forbidden flags** (Codex, from live `grok --help`): `--system-prompt-override`, `--leader-socket`,
  `--worktree`, `--agents`, `--allowedTools`, `--rules`, `--restore-code` and more.
- **No write containment** (Antigravity). agy denies workspace writes on the consult path via sandbox-exec; grok had
  nothing, while grok's own config sets `permission_mode = "always-approve"`. Now emits `--sandbox read-only` by
  default. Test u_b_18.
- **Duplicate managed flags** (Codex). `value_after` returns the first match; grok takes the LAST, so a duplicate
  silently wins. Test u_b_19 asserts each managed flag appears exactly once.
- **Sampled rather than exhaustive forbidden-flag coverage** (Codex). Test u_b_20 now iterates the whole list.

## Peer findings fixed in Slice C

- **`finish(self)` was an ordering trap** (Codex). Facts were exposed as getters that had to be read BEFORE `finish`
  consumed the parser, but `agent_exec.rs` calls every sibling as a bare `parser.finish()`, so they would never have
  been read. Now `finish_full() -> GrokParsed` returns everything in one value.
- **Substring tool matching was wrong** (Codex). It had already misfired once (`glob_file_search` contains "search").
  Codex named more waiting to happen: `thread` contains "read", `rewrite` contains "write", `research` contains
  "search". Now exact matching, with the ACP `kind` field authoritative. Test u_c_19.
- **Text after `end`** was appended, mutating an answer the runner may already have reported. Test u_c_15.
- **A second `end`** emitted a second TurnCompleted. Now idempotent. Test u_c_16.
- **`usage` after `end`** left the recorded TurnCompleted carrying stale usage. Now backfilled. Test u_c_17.
- **Duplicate `toolCallId`** attached every update to the first call, starving later ones. Now targets the last
  still-open call. Test u_c_18.

## Known cosmetic issue

`triumvirate doctor` prints a tracing INFO line for the `grok_command` span into operator-facing stdout. Pre-existing
behavior of `#[instrument(skip_all)]` on the connector resolvers, not introduced by grok. Not fixed.

## Verified sandbox behavior (matters for Slice D)

Profiles on grok 1.0.13: `workspace`, `read-only`, `strict`, `off`. **An unknown profile does NOT fail.** It warns
`sandbox could not be applied` on stderr and runs with NO containment. **The Slice D runner must treat that string as
a hard error**, or a consult silently runs uncontained.

## Open from peer review, not yet addressed

- **Antigravity B:** validation lives in the builder; they argue the trust boundary is `agent_exec.rs`. Kept in the
  builder as defense in depth since it is the single source of truth for argv. Revisit in Slice D.
- **Antigravity E:** `prompt` is baked into argv, coupling the module to single-turn use. Fleet workers (Slice H) may
  need prompt delivery via stdin. Revisit there rather than speculatively generalizing now.

## Pre-existing defects in scope

1. `aliases.rs:166` mapped only gemini and codex. **RESOLVED in Slice E, but NOT the way it was first framed.**
   An existing test (`u_al_03_spawn_daemon_claude_rejected`) asserts claude IS rejected, so the exclusion was
   deliberate rather than drift. Daemon targets are now an explicit `daemon_target_agents()` list of the spawnable
   CLI peers (gemini, codex, grok). claude is excluded because it is the orchestrator, deepseek because it is HTTP
   with no CLI to hold open. Both exclusions now carry a documented reason and a test.
2. `agent_exec.rs` retry test asserted against a closure it defined itself. **FIXED in Slice D:** the schedule is now
   `attempt_schedule_for()`, a real function, and the test calls it.
3. `agent_exec.rs:336` claimed subscription calls cost $0. **FIXED in Slice D:** corrected to say dollars, and to
   record that a flat plan still has finite quota burned invisibly at 14K to 67K per consult.
