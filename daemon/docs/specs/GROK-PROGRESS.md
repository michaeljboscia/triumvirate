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

**This table was stale until 2026-09-02 and said slices D to N were awaiting review.** They were
reviewed on 2026-09-01 and the chorus section below recorded it, so the top of this file
contradicted its own body. Corrected here. The contradiction is kept in the record rather than
deleted, because a status table that disagrees with the evidence beneath it is the failure mode
this document exists to prevent.

| Slice | State | Commit | Peers reviewed |
|---|---|---|---|
| A identity + `supported_agent_names()` | **DONE** | `3893d27` | not reviewed (mechanical) |
| B invocation builder | **DONE**, 20 tests green | `68c10d3` + fixes | Codex, Antigravity (Grok hit max-turns) |
| C parser | **DONE**, 21 tests green vs real fixtures | `4c8d8d3` + fixes | Codex (5 defects found and fixed) |
| D spawn/dispatch + defects 2,3 | **DONE**, 179 tests green | `e41a55d` | Codex, Antigravity, Grok, 2026-09-01 |
| E doctor/README/aliases + defect 1 | **DONE**, 222 tests green | `cc192b2` | Codex, Antigravity, Grok, 2026-09-01 |
| F mock binary + integration | **DONE**, 14 integration + 2 live-gated | | Codex, Antigravity, Grok, 2026-09-01 |
| G peer-review panel | **DONE**, grok is a default reviewer | | Codex, Antigravity, Grok, 2026-09-01 |
| H fleet | **DONE**, grok launches via the shared builder | | Codex, Antigravity, Grok, 2026-09-01 |
| I/N ABE | **DONE**, ABE is agent-aware; codex path byte-identical | | Codex, Antigravity, Grok, 2026-09-01. **ABE is Codex only. The grok worktree path is UNVERIFIED.** |
| J token-economics | **DONE** via the direct path; no offline scanner is possible | | Codex, Antigravity, Grok, 2026-09-01 |
| K shared-types sweep | **DONE**, one real gate found and fixed | | Codex, Antigravity, Grok, 2026-09-01 |
| L e2e | **DONE**, 2 live tests gated on `TRIUMVIRATE_LIVE_GROK=1` | | Codex, Antigravity, Grok, 2026-09-01 |
| M connector resolution | **DONE**, no per-machine config needed | `24a3dd0` | Codex, Antigravity, Grok, 2026-09-01 |

That pass found 5 defects. Two of them were findings I had already marked CLOSED and which were
not closed. See the chorus section below; it is evidence, not history to be tidied.

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

### FIND-GROK-02 slice D to N peer review: CLOSED 2026-09-01

The pass ran. Codex, Antigravity and Grok each took a different lens. 5 defects, listed in the
chorus section below. The header table above said "awaiting peer review" for another full day
after this was true, which is its own small instance of the same problem.

### FIND-GROK-03 ABE honesty: CLOSED, Option A. Option B is OUT OF SCOPE.

`snapshot_workers` hardcoded `agent: "codex"` and `name: "codex-worker-{id}"`, so `/api/workers`,
the UI and telemetry reported Codex for EVERY worker. ABE only spawns Codex today, so the label
was true BY LUCK and would have kept being reported the moment anything else was dispatched.

`TaskRecord.agent` is now a real field, defaulting to `codex` at the one registration site, and
`snapshot_workers` reads it. Two tests: a record carrying `grok` reports grok and names the
worker `grok-worker-{id}`, and the default is still codex. The lie cannot land later.

### FIND-GROK-04 panel Fast isolation: CLOSED 2026-09-02, after being wrongly marked closed once

Two problems, both fixed.

`TRIUMVIRATE_PEER_REVIEWERS` (comma list) now overrides the panel. The default is unchanged and
still seats grok, which was an explicit ruling. Dropping a reviewer is an env change rather than
a patch to `peer-review/src/lib.rs`. An empty or whitespace-only override falls back to the
default rather than silently producing an empty panel, which would look exactly like a review
that passed.

**The rest of this section is SUPERSEDED and kept as evidence. `with_forced_fast` was deleted.**

What it said: a thread-local pinned grok to Fast on the calling thread, and the mechanism was
shipped while the wiring was not, which was already recorded here as STILL OPEN.

Two things were wrong with it, found by review rather than by me.

Codex and Antigravity independently called the thread-local unsound. Tokio's work-stealing
scheduler moves tasks across OS threads at every await point, so the flag does not follow the
task it was set for, and a panic inside the closure would leave it set on that worker with no
Drop guard. It was deleted rather than patched.

Grok found the deeper one: there was no panel dispatch to isolate. `enforce_mandatory_peer_review`
wrote a review row and auto-approved it, so no reviewer was ever spawned. Fixing the depth of a
dispatch that does not happen is theatre.

**CLOSED 2026-09-02, commit `9cfa54a`.** Depth is now a parameter on
`build_grok_invocation_with_profile`, the same shape `sandbox_override` already uses. Not a
thread-local, and not a child env var either: the argv is built in the parent, so a child env
would arrive too late to change a flag. The panel seat reads `is_peer_review`, which is
`serde(skip)`, so no client can set it to make its own consult cheap. An explicit
`TRIUMVIRATE_GROK_EFFORT` still wins, because the override exists to stop a Deep daemon making
every review multi-minute, not to overrule a person.

Four tests, including the control that an ordinary consult on the SAME Deep daemon is still
Deep. Mutations run: override ignored, and None defaulting to Fast.

### FIND-GROK-05 doctor compat drift: CLOSED

`triumvirate doctor` now reads `~/.grok/config.toml` and reports any of the five
`[compat.claude]` keys that is missing or true, naming the offending keys and what it costs:
roughly 420 tools instead of 126, measured at 66,559 input tokens to answer "pong".

A MISSING key counts as drift, because the default is inherit-on and silence is not safety.
Keys in other sections do not satisfy the check. Deliberately a line scanner rather than a TOML
parse: this crate has no toml dependency and a parse failure elsewhere in the file must not hide
the answer.

**Live tripwire added 2026-09-02, commit `9cfa54a`, which was the remaining half.** Doctor
detects drift BEFORE a turn. What grok advertises DURING a turn is a different number, because
MCP servers add to it. A tool surface above 200 now warns with the count. Warn only: a large
surface is expensive, not wrong, and failing there would break working setups over a cost
signal. The threshold sits between the two observed captures, 26 tools with none connected and
420 with them.

Verified live: `triumvirate doctor` prints `grok compat.claude: closed (all five keys false)`.

---

## FIND-GROK-02 peer pass, 2026-09-01. Two findings I marked CLOSED were not closed.

Three peers reviewed slices D-N AND the solo work in ac66950. Corrections to my own claims come
first, because the word "closed" was the problem.

### THE HEADLINE, found by Grok: mandatory peer review does not review

`enforce_mandatory_peer_review` (`agent_exec.rs:1888`) writes a review row and then immediately
calls `submit_review(review_id, "approve", "auto-approved in mandatory peer review mode")`.
**No agent is ever spawned. No reviewer ever looks at anything.**

`TRIUMVIRATE_REQUIRE_PEER_REVIEW=1` is a rubber stamp that creates a database record saying
approved. Verified independently.

Consequences that invalidate work I did today:
- FIND-GROK-04's premise (a slow grok seat dominating panel latency) does not exist in the form
  I fixed, because there is no panel dispatch.
- `default_reviewers()` and `TRIUMVIRATE_PEER_REVIEWERS` configure a roster nothing dispatches
  to. Grok, on unknown names failing "loudly at dispatch": "There is no dispatch."
- The "grok is a default reviewer" ruling is decorative while this holds.

NOT FIXED. It is a bigger change than the rest of this pass and it deserves its own goal.

### FIND-GROK-04: NOT CLOSED. I marked it closed and it is not.

Two independent reviewers called `with_forced_fast` unsound: tokio's work-stealing scheduler
moves tasks across OS threads at every await, and a panic inside the closure would leave the
flag set on that worker with no Drop guard. Grok added that the claim held only for a
synchronous closure, and that nothing production called it anyway.

The helper is REMOVED rather than patched. What survives is a test that Fast and Deep differ in
the arguments that cost time. The finding stays OPEN, and the reason it cannot be closed is the
auto-approve defect above.

I had written "CLOSED" and then "STILL OPEN" two paragraphs apart in the same document. Grok:
"Shipping an unused helper and marking the finding closed is the lie the chorus exists to stop."

### FIND-GROK-05: was NOT closed, now is

The fixlist required a NON-ZERO warn exit, fatal under `TRIUMVIRATE_GROK_STRICT_COMPAT=1`, and
a committed config snippet. I shipped a printed line and marked it closed. Doctor still returned
`Ok(())` no matter what, so any script gating on it could not see the drift.

NOW: doctor returns an error on drift under `TRIUMVIRATE_GROK_STRICT_COMPAT=1`, prints an
explicit WARN otherwise, and `daemon/docs/grok-compat-claude.example.toml` is committed so the
setting is not tribal knowledge on one laptop.

Grok verified the scanner against its REAL config and confirmed it parses correctly, including
that `[compat.cursor]` is not swallowed by the `[compat.claude]` match.

STILL NOT DONE from that finding: the `tool_surface.tools > 200` tripwire after live consults.

### FIND-GROK-03: Option A met, but the write path was cosmetic

Grok: both production write sites hardcoded `"codex"`, `register` took no agent, my comment
claimed a `Default` impl that does not exist, and my note said "the one registration site" when
there are two. Antigravity threaded `agent` through the `AbeTaskTracker` trait and both call
sites, which is the real fix, and it is kept.

### Test defects in my solo work

- `the_live_fixture_uses_target_file` was theater: `include_str!(...).contains("target_file")`,
  which never called the formatter. Replaced with a test that pulls the real `rawInput` out of
  the capture and runs `format_working_state` over it.
- `forced_fast_tests` set the depth env var without `env_lock()`, racing the neighbouring depth
  tests. The surviving test now lives inside the module that owns the lock, because a second
  mutex would not serialise against the first. Third time this repo has produced that race.

### Slices with NO defect found

D session tracking, D timeout process-group kill, D `finish_full()`, D retry double-spend,
H fleet argv and empty resume, J token scanner roots, K HTTP agent gate. Codex gave evidence
for each rather than a bare pass.

### One real defect in slice J, found by Codex

Grok direct token persistence wrote `thinking_tokens: 0` and `cost_usd: None` while the parser
had captured both. Grok quota burn was under-recorded. Codex's patch is saved at
`scratchpad/codex-token-fix.diff` and is NOT applied: it also carried an incomplete ABE change
that left the workspace not compiling. Applying the token half deliberately is follow-up work.

### PROCESS FAILURE: I told three reviewers the tree was frozen, then handed one a charter that
### told it to fix things

Codex modified 14 files and left the build broken while the other two were mid-read. That is my
error: the fixlist charter says "Fix every defect that is real. Do not acknowledge and leave",
and I passed it verbatim to a reviewer while promising the others a frozen artifact.

Worse, my own harness comment claimed `codex exec` without `--full-auto` is read-only. That is
the SAME false containment claim I had already corrected inside the daemon, repeated in
`~/.claude/scripts/peer-review/dispatch.sh`, and it survived because no review had ever been
told to write. `dispatch.sh` now passes `--sandbox read-only` explicitly.

39 groups green. Clippy clean except the pre-existing `pantheon` warning.

### Slice J defect applied deliberately, 2026-09-01

Codex's finding, applied by hand rather than by taking its patch, because that patch also
carried an incomplete ABE change that left the workspace not compiling.

`persist_daemon_token_record` wrote `thinking_tokens: 0` and `cost_usd: None` while the grok
parser had both values in hand. Every grok consult under-recorded its quota burn.

- `ParsedAgentResult` gains `self_reported_cost_usd`. Only grok sets it, from
  `end.total_cost_usd`. Every other parser sets `None` explicitly rather than by default, so a
  new parser has to decide rather than inherit silence.
- `thinking_tokens` now reads from the usage block, which every streaming parser already fills.
- `Eq` was dropped from the `ParsedAgentResult` derive, since `f64` has no total order.
- The DeepSeek Err-path record still writes zeros. That one is deliberate and commented as a
  narrow error record, not the same defect.

grok runs on a flat plan, so the cost figure is a USAGE signal rather than a bill, and it is the
only per-turn quota number a subscription agent gives us.

Three tests against the live capture, mutation-verified: dropping the passthrough reds
`grok_cost_survives_into_the_parsed_result`. The batch path asserts it reports NO cost rather
than inventing one.

### Harness containment, now tested rather than claimed

`~/.claude/scripts/peer-review/dispatch.sh` passes `codex exec --sandbox read-only`. Verified
empirically: told to create a file, codex did not, and reported "The filesystem sandbox is
read-only."

## The review gate, 2026-09-02. FIND-REVIEW-01 through 05.

Worked from `GROK_CHORUS_NEXT.md`. Every item below was reviewed by Codex, Antigravity and Grok
on a frozen tree, each with a different lens. Two of the five findings came from that review
rather than from me, and one of my own commit-message claims was proven false by it.

| Finding | State | Commit | Found by |
|---|---|---|---|
| FIND-REVIEW-01 `review_submit` cannot approve an unfinished review | **CLOSED** | `20be42a` | Grok (spec), Claude (bug) |
| FIND-REVIEW-02 inflight cap honoured on the dispatch path | **CLOSED** | `20be42a` | Grok (spec) |
| FIND-REVIEW-03 a dispatch-owned review is not client writable | **CLOSED** | `4a457fc` | **Codex**, confirmed by Grok |
| FIND-REVIEW-04 the verdict string is validated, not stored verbatim | **CLOSED** | `4a457fc` | **Grok** |
| FIND-REVIEW-05 the `claude` seat could never produce a receipt | **CLOSED** | `b9c13bd` | Claude, while implementing 1.5 |

### FIND-REVIEW-01: the record could be written by anyone

`092f90b` made the review dispatch real, which removed the auto-approve. The layer underneath
was still open: `submit_review` took `(review_id, verdict, comments)` and its UPDATE had no
state guard and no notion of who was calling. It set `state = 'done'` on any row in any state
for any caller.

An approve now requires the row to be `in_progress` AND the submitter to be the assigned
reviewer, or the in-process dispatch. `Submitter` is a required parameter rather than an
`Option`, so every submit path has to state on the record which it is.

### FIND-REVIEW-02: the cap counted reviews and never limited them

`request_review` parks a review as `pending` once the inflight cap is met, and nothing on the
dispatch path read that state, so a parked review was dispatched exactly like a running one.
It now blocks BEFORE spending a live model call. The test asserts the invocation file does not
exist, because asserting only that the turn was blocked would pass even if the reviewer had
been spawned and rejected afterwards.

### FIND-REVIEW-03: a name in a request body is not an identity

**Codex found this against my first fix, and Grok confirmed it independently.** The
reviewer-name check was not an authentication boundary: `review_request` RETURNS the assigned
reviewer to the caller, and the mandatory path emits it in the `REVIEW_PENDING` outbox line, so
a client can simply name it back and match.

Ownership can be enforced where naming cannot. A review the mandatory dispatch is conducting is
marked `dispatch_owned`, set only in-process, with no request field that deserialises into it,
and accepts writes from `Submitter::Dispatch` alone. The check covers every verdict, not just
approve: refusing only approve would leave a denial of service where a client lands a reject on
a live review and the dispatch's own submit then fails as "changed state", which reads like an
infrastructure fault.

Client-requested reviews are deliberately unaffected. Fleet queues those for an agent to pick up
over MCP; they are bookkeeping and were never a gate.

### FIND-REVIEW-04: a padded approve was not an approve

**Grok found this.** The verdict string was written to the row verbatim, and the identity check
fired only on an exact case-insensitive `approve`. So an MCP body carrying a trailing space, or
the word `approved`, skipped the identity check entirely and still set `state = 'done'`.
Verdicts are now normalised and anything outside approve, concerns, reject, indeterminate is
refused. `indeterminate` is accepted on purpose: it is the fail-closed verdict, and refusing to
store it would turn a blocked turn into a submit error.

### FIND-REVIEW-05: the claude seat was blind

`claude` is a DEFAULT reviewer, and its runner built `ParsedAgentResult` from
`Default::default()`: no parser mode, no tool calls. Turning sight on for the reviewer would
have rejected that seat 100% of the time for a fault that is the harness's, which is exactly the
failure already hit with agy one seat over.

New `claude_stream.rs`, verified live against the installed CLI, fixtures captured from that run
including the permission-DENIED case. Two flag traps are recorded in the source:
`--allowedTools` is variadic and swallows the prompt, and without an allow-list a headless
claude auto-denies its own reads.

### Goal 1.5: what the sight gate on the reviewer does and does not buy

The artifact is written to `.triumvirate/reviews/<review_id>.md`, named in the prompt, and
passed as `required_sources`. The old comment here said `required_sources` had to stay empty
because a pasted artifact makes a toolless review legitimate. That was true, and it also meant a
reviewer could approve having opened nothing. Giving the artifact a path removes the tension
rather than picking a side.

Stated plainly: requiring a read of text that is also in the prompt is partly ceremonial. What
it buys is exact. A reviewer that made zero tool calls can no longer return APPROVE, which is
the failure actually observed on 2026-09-01 when a peer described its own toolless output as
"rigorous sourcing". **It does not prove the contents were used.** A reviewer can open the file
and still write from memory. That next layer is entailment against the opened text, or a human,
and it is not built.

### A false claim in my own commit message, found by Antigravity

`20be42a` listed "SQL guard removed" among five mutations that killed tests. It did not. What
was actually run was BOTH guards removed together, which the Rust guard accounts for. Removing
only the state condition from the UPDATE leaves the whole suite green, because every test that
attempts an invalid transition is stopped by the Rust guard before the SQL runs. Verified by
running that mutation. The clause stays as defence in depth against a read/write race across two
connections, and the code now says plainly that no test covers it.

### Known open, stated rather than left to silence

- The concurrent-modification arm of `submit_review` and the telemetry-on-rejection path have no
  test. Both named by Antigravity.
- `persistent_worker_reuse_second_call_is_faster_and_marked_reused` failed once under full
  parallel load and passed 3/3 alone. It asserts one call is faster than another, which is a
  wall-clock assertion on a loaded machine.
- FIND-GROK-01 remainder: only `read_file` appears in a live grok fixture. Capturing
  `run_terminal_command`, `search_replace`, `grep` and one Unknown is still to do.
- ABE `dispatch_codex` and fleet task completion do not enter this gate at all. That is not a way
  the gate is a stamp; it is work that leaves the building without it.

### FIND-REVIEW-06, found by Grok in round 2: a partial read is not a read

Grok reviewed the frozen tree at `b9c13bd` and showed how cheap the receipt was to satisfy
without reading. Claude's own captured fixture is `Read` with `"limit": 1`, and `head` is a
classified reader on the codex backend. Either satisfies the sight gate after looking at ONE
line of a file whose contents were, at that point, also pasted into the prompt. The receipt
proved a syscall ran against the path, not that anything was read.

Grok's own proposed mitigation was taken, in its words: stop pasting the artifact into the
prompt, put a daemon-generated nonce in the file that is not in the prompt, and require that
nonce on the verdict. The nonce sits at the END of the file, so a `head -1` or `limit: 1` read
now fails.

The author cannot forge it: the nonce is minted after the author's turn has already finished, so
it does not exist at the time the artifact was written.

Removing the paste is also strictly stronger prompt-injection containment than the fence that
replaced it. Text that is not in the prompt cannot compete with the prompt's own instructions.
`review_08` was rewritten to assert that stronger property rather than the fence markers.

A missing nonce only ever upgrades a PASSING verdict to Indeterminate. It never touches a
blocking one, so a reviewer that correctly rejected on a partial read still rejects with its own
reasoning intact. `review_18` pins that, and the mutation that removes the guard turns it red.

**Still not proven, and this is the honest limit:** the reviewer read the file. It was not shown
to have judged it. An agent can read every byte, quote the last line, and write its verdict from
nothing. That layer is entailment against the opened text, or a human. Grok said the same and
recommended this before entailment rather than instead of it.

### Round 2 also caught two more, and one disagreement between peers

**Codex:** a malformed `is_error` was read as success, because `as_bool().unwrap_or(false)`
folded "absent", "explicit bool" and "present but unparseable" into two outcomes instead of
three. A malformed explicit FAILURE could satisfy sight. Now an explicit match, failing closed.

**Codex:** `saw_result()` returned `final_response.is_some()`, so a terminal event with no string
`result` field would be parsed correctly, tool calls and all, then discarded as plain text and
false-reject an honest review.

**Grok, on the same code, said the `is_error` handling HOLDS.** It read both fixtures, confirmed
both were handled, and stopped there. That is true and insufficient: neither fixture contains a
malformed flag. Codex was right. This is the argument for a panel rather than a reviewer, and it
is recorded rather than smoothed over.

**Grok:** `u_gd_01` tests the BUILDER, not the route. Deleting `panel_child = req.is_peer_review`
from `run_named_agent_with_session_and_model` would leave it green. That is precisely the
"tested the helper, not the route" bug this pass was told not to repeat. Rule 5 was satisfied and
Rule 2 was not. `u_gp_01` now drives a real grok child through the dispatch path and reads its
argv; running that exact deletion leaves the builder test green (4 passed) and turns the route
test red.

**Grok:** the operator-authority exception was documented for `TRIUMVIRATE_GROK_EFFORT` and not
for `TRIUMVIRATE_GROK_MAX_TURNS`, which made the same deliberate rule look like an oversight on
one of the two. Documented and pinned.

**Grok:** `finish()` stamped the streaming parser mode unconditionally, which was safe only while
every caller remembered to check `saw_result()` first. The parser now names its own mode.

**Antigravity:** `run_claude_cli_process_with_session` had no test at all. Every review test
seats claude as the AUTHOR, whose binary is a `mock-` connector, so the runner under change was
never executed. Four tests now drive it with a fake claude that records its own argv.

**Antigravity confirmed, by tracing rather than by assertion, that the test mock is not a
bypass:** `is_mock_connector` matches only the `mock-` prefix, the reviewer stand-in is
`fake-codex-*`, so tests execute the real runner and the real parser, and neither allowlist
carries a `cfg(test)` directive. Grok reached the same conclusion independently.
