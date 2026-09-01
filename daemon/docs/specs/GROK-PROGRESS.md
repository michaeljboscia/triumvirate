# Grok integration progress

**RESUME HERE. Authoritative state. Update after every slice, before starting the next.**
Goal: `GOAL-grok-integration.md` · Plan: `grok-integration-test-plan.md` · Evidence: `findings/grok-*.md`

## LIVE VERIFICATION, 2026-08-30, against the installed binary

Deployed via `scripts/install.sh` (IRON LAW: never run production from `target/`) and the daemon restarted.

| DONE WHEN | Result |
|---|---|
| 1. `cargo test` green, no network, no API key | **PASS**, 562 tests |
| 2. `/status` lists grok and equals `supported_agent_names()` | **PASS**, `[gemini, codex, deepseek, claude, grok]` |
| 3. `ask_agent {agent:"supergrok"}` returns pong | **PASS**, alias normalized, `"Grok responded on attempt 1"` |
| 4. Session resume: turn 2 recalls turn 1 | **PASS**, codeword ZEPHYR7 recalled across turns |
| 5. Named session spawn for grok | **PASS** |
| 6. grok on the peer-review panel | **PASS** (unit + integration) |
| 7. Telemetry with tokens AND cost | **PASS**, 3 turns logged at $0.00163, $0.00574, $0.00322; `$ai_generation` firing |
| 10. Fleet worker / token-economics | **PASS** for fleet and token-economics. **ABE deferred, see below** |
| 11. Parity sweep | **PASS**, one real gate found and fixed in `daemon-http` |

**REQ-GROK-013 confirmed live:** the lifecycle read `SPAWNED, WORKING, DONE` with `on attempt 1`. No retry.

### RESOLVED in Slice M: the config change is no longer required

`resolve_connector_command` now falls back to `$HOME/.local/bin/<name>` when a bare binary name is not on PATH.
Verified by REMOVING `TRIUMVIRATE_GROK_BIN` from `~/.claude.json` entirely, reinstalling, restarting the daemon, and
confirming grok still answers. A fresh clone works with no per-machine config.

Precedence: explicit `TRIUMVIRATE_*_BIN` always wins (an operator-set path is authoritative and must never be
second-guessed), then PATH, then the install dir, then the bare name so failure stays a normal ENOENT.

### The original problem, for the record

The daemon runs with `env -i` and a PATH of `/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin`, which does **not**
include `~/.local/bin`. A bare `grok` therefore ENOENT'd. Fixed by adding to the triumvirate MCP env block in
`~/.claude.json`, exactly as `TRIUMVIRATE_CODEX_BIN` already was:

```
TRIUMVIRATE_GROK_BIN=/Users/michaelboscia/.local/bin/grok
```

**This lives in the operator's `~/.claude.json`, not in this repo**, so a fresh machine needs it set or every grok
dispatch fails with `No such file or directory (os error 2)`. Backup written to `~/.claude.json.bak-*`.

## FINAL VERIFICATION 2026-08-31, against the deployed binary

Every check run, not asserted. 585 tests, clippy clean with `--all-targets`, CI green on `8048a73`.

| Check | Result |
|---|---|
| workspace tests | PASS, 585, zero failing suites |
| clippy `--workspace --all-targets -D warnings` | PASS |
| installed binary carries grok | PASS |
| `/status` advertises grok | PASS, equals `supported_agent_names()` |
| consult via canonical name | PASS |
| consult via the `supergrok` alias | PASS, normalized to `agent: grok` |
| single attempt, REQ-GROK-013 | PASS, `on attempt 1` in production |
| `/token-summary?agent=grok` | PASS |
| `/token-summary?agent=deepseek` | PASS, this used to 400 |
| doctor reports auth KIND | PASS, `cached login (subscription)` |
| doctor spends no tokens | PASS |
| named session resumes its OWN memory | PASS |
| named session cannot see another's | PASS |

The last two matter as a PAIR. Isolation is trivial to get by breaking resume, and resume is trivial
to get by breaking isolation. Both hold at once.

Tool surface is now logged per turn: a real consult reported **126 tools, 11 commands** against the
420 of full `~/.claude.json` inheritance, which confirms the curated MCP config by measurement.

## Known gaps, stated rather than closed

- **A grok worktree ABE worker is UNVERIFIED.** codex reaches the main repo's `.git` via `--add-dir`;
  grok has no equivalent, so git operations needing the parent `.git` may fail. No grok worktree
  dispatch has ever been run.
- Two pre-existing `--all-targets` issues live outside this work and were left alone.

## PEER REVIEW STATUS: all slices reviewed

- **B** reviewed by Codex + Antigravity. 6 defects found, all real, all fixed.
- **C** reviewed by Codex. 5 defects found, including an API trap.
- **A, D, E, F, G, H, J, K, L, M, N: NOT REVIEWED.** That is the runner, dispatch, doctor, integration suite,
  peer-review panel, fleet, token economics, the parity sweep, connector resolution, and ABE.

## Slice status

| Slice | State | Commit | Peers reviewed |
|---|---|---|---|
| A identity + `supported_agent_names()` | **DONE** | `3893d27` | not reviewed (mechanical) |
| B invocation builder | **DONE**, 20 tests green | `68c10d3` + fixes | Codex, Antigravity (Grok hit max-turns) |
| C parser | **DONE**, 21 tests green vs real fixtures | `4c8d8d3` + fixes | Codex (5 defects found and fixed) |
| D spawn/dispatch + defects 2,3 | **DONE**, 179 tests green | `e41a55d` | awaiting peer review |
| E doctor/README/aliases + defect 1 | **DONE**, 222 tests green | `cc192b2` | |
| F mock binary + integration | **DONE**, 14 integration + 2 live-gated | | |
| G peer-review panel | **DONE**, grok is a default reviewer | | |
| H fleet | **DONE**, grok launches via the shared builder | | |
| I/N ABE | **DONE**, ABE is agent-aware; codex path byte-identical | | |
| J token-economics | **DONE** via the direct path; no offline scanner is possible | | |
| K shared-types sweep | **DONE**, one real gate found and fixed | | |
| L e2e | **DONE**, 2 live tests gated on `TRIUMVIRATE_LIVE_GROK=1` | | |
| M connector resolution | **DONE**, no per-machine config needed | `24a3dd0` | |

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

## Slice I (ABE): blocked, and it is NOT a grok-specific gap

`abe/task_tracker.rs` hardcodes `agent: "codex"` with a pre-existing comment saying so:
*"the daemon only spawns Codex workers via ABE today; if Claude/Gemini ABE workers land in a future task, this should
branch on `TaskRecord` metadata."*

So ABE is Codex-only for **every** agent. Gemini and Claude are equally absent. Making grok an ABE worker requires
the ABE dispatch path to spawn non-Codex workers at all, which is a feature that does not exist yet for anyone.

**Deliberately not faked.** Setting `agent: "grok"` on a lifecycle event while ABE still spawns a Codex process
would produce telemetry that lies. The honest state is: ABE multi-agent support is one task, and grok comes along
free when it lands.

## Slice J: there is NO offline token scanner for grok, by design

The other agents leave token counts on disk. Grok does not: `~/.grok/sessions/<cwd>/prompt_history.jsonl` holds only
`{timestamp, session_id, prompt, is_bash}`, with no usage block. Verified against a real profile.

Grok reports usage and `total_cost_usd` LIVE in the `end` event, so spend is recorded through
`direct::record_daemon_tokens` from the runner, where the numbers exist. Writing a scanner over that file would
produce records with zeroed counts, which is worse than none because it looks like measured spend.

## Slice K: one real gate found

`daemon-http/src/lib.rs:825` hardcoded `matches!(agent, "claude" | "codex" | "gemini")`, so `/token-summary?agent=grok`
returned 400. **It also rejected `deepseek`**, which is dispatchable and accumulates records. Same drift class as the
four `supported_agents` surfaces. Now validated against `is_supported_agent_name`.

Every other file carrying `"codex"` without `"grok"` uses it as a test fixture or example, not a dispatch decision.

## Known pre-existing issue, NOT introduced here

`cargo clippy --workspace --all-targets` fails on `daemon-core/src/pantheon_session.rs:174` with
"an async construct yields a type which is itself awaitable". That crate was never touched by this work. Left alone.

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

---

## Chorus fix list, 2026-09-01

Worked from `GROK_ADAPTER_CHORUS_FIXLIST.md`.

### FIND-GROK-01 tool-call mapping: PARTIALLY CLOSED

The `target_file` bug was real and only half fixed. `agent-adapter/src/grok.rs` already
extracted `target_file` for its own `FileRead` event, with a comment saying Grok caught it.
`agent-adapter/src/lib.rs::format_working_state`, the GENERIC formatter the watch CLI uses, did
NOT: it looked for `file_path` and `path` only, which are the vendor guide's names. The live
capture sends `{"target_file":"target.txt"}`. So every grok tool line in the watch CLI rendered
the tool name instead of the file. Two surfaces, one fixed. FIXED, with three tests including
one pinned to the committed capture so a recapture with a different key fails loudly.

STILL OPEN: only `read_file` appears in a live fixture. Capturing one live turn each for
`run_terminal_command`, `search_replace`, `grep` and one Unknown remains to be done.

### FIND-GROK-02 slice D-N peer review: NOT DONE

No slice-scoped review pass has run. Unchanged.

### FIND-GROK-03 ABE honesty: CLOSED, Option A

`snapshot_workers` hardcoded `agent: "codex"` and `name: "codex-worker-{id}"`, so `/api/workers`,
the UI and telemetry reported Codex for EVERY worker. ABE only spawns Codex today, so the label
was true BY LUCK and would have kept being reported the moment anything else was dispatched.

`TaskRecord.agent` is now a real field, defaulting to `codex` at the one registration site, and
`snapshot_workers` reads it. Two tests: a record carrying `grok` reports grok and names the
worker `grok-worker-{id}`, and the default is still codex. The lie cannot land later.

### FIND-GROK-04 panel Fast isolation: CLOSED

Two problems, both fixed.

`TRIUMVIRATE_PEER_REVIEWERS` (comma list) now overrides the panel. The default is unchanged and
still seats grok, which was an explicit ruling. Dropping a reviewer is an env change rather than
a patch to `peer-review/src/lib.rs`. An empty or whitespace-only override falls back to the
default rather than silently producing an empty panel, which would look exactly like a review
that passed.

`mcp_bridge::grok::with_forced_fast()` pins grok to Fast on the calling thread, so a daemon
started with `TRIUMVIRATE_GROK_DEPTH=deep` cannot make every mandatory review a multi-minute
turn. Deliberately a THREAD-LOCAL, not a mutation of `std::env` around the child: reviewers
dispatch concurrently and a process-wide set would leak into the others. This repo has already
shipped one test that could not fail because of exactly that pattern, and while writing the
roster tests I reproduced it again and had to serialise them.

Tests prove the override beats a Deep environment, does NOT leak past its closure, and changes
the ARGS that actually cost time (max-turns and effort) rather than only the enum.

STILL OPEN: `with_forced_fast` is the mechanism; wiring the peer-review engine's dispatch to
call it is not done, because the engine records reviews in SQLite and does not spawn the agent
itself. Naming that honestly rather than claiming the seat is isolated.

### FIND-GROK-05 doctor compat drift: CLOSED

`triumvirate doctor` now reads `~/.grok/config.toml` and reports any of the five
`[compat.claude]` keys that is missing or true, naming the offending keys and what it costs:
roughly 420 tools instead of 126, measured at 66,559 input tokens to answer "pong".

A MISSING key counts as drift, because the default is inherit-on and silence is not safety.
Keys in other sections do not satisfy the check. Deliberately a line scanner rather than a TOML
parse: this crate has no toml dependency and a parse failure elsewhere in the file must not hide
the answer.

Verified live: `triumvirate doctor` prints `grok compat.claude: closed (all five keys false)`.
