# Seven Primes launch report (T-303, REQ-082)

Launched overnight 2026-10-18 to 2026-10-19 (ET) on the live daemon (`/Users/michaelboscia/.local/bin/triumvirate`, launchd
`com.triumvirate.daemon-v2`), built from feat/seven-primes. Rollback: `/Users/michaelboscia/projects/triumvirate-rollback/`
(3.9.0 binary and the pre-launch `~/.claude.json`).

## Enablement (D-09 order: Mistral, Kimi, Qwen)
`~/.claude.json` triumvirate env: `TRIUMVIRATE_ENABLE_{MISTRAL,KIMI,QWEN}=1` and `TRIUMVIRATE_{MISTRAL,KIMI,QWEN}_BIN` set to the
absolute `~/.local/bin` paths (the daemon's PATH has no `~/.local/bin`). Per A-2 there is no TTY rule; `doctor --live` was run
by the orchestrator.

| Seat | Provider | Billing | CLI | doctor --live (verified.json probed_at, UTC) | Served model |
|---|---|---|---|---|---|
| mistral | Mistral | subscription | vibe 2.26.1 | 2026-10-19T05:55:42Z PASS | not reported by the stream (null) |
| kimi | Moonshot | subscription | kimi-code 2.2.0 | 2026-10-19T05:55:39Z PASS | not reported by the stream (null) |
| qwen | Alibaba (QwenCloud Token Plan) | subscription | qwen-code 0.25.2 | 2026-10-19T05:55:47Z PASS | qwen3.7-plus |

Read-back of `/status` (live daemon, after the launch): every new seat `configured: true, verified: true, suspended: false,
available: true, reason: null`. Read-back of `$TRIUMVIRATE_HOME/seats/<seat>/verified.json` matches the table above.

## Stage 1: smoke (each seat answers; parser, session, containment)
- ask_agent (strict_agent) arithmetic and identity: all three answered 391 and named their own lab (LIVE_SMOKE-2026-10-19.md).
- Live E2E (`verify-live-agents.sh all --launch`, REQ-081, docs/seven-primes/live-evidence.json): kimi 4/4, mistral 4/4,
  qwen 3/3 PASS. Covered: a file-grounded answer that passes the sight gate on a named source (sight receipt: the gate
  accepted the turn), a canary write that leaves no file (seatbelt plus repo audit), a second session turn that resumes the
  same vendor session, Kimi's forced first-turn failure leaving the session broken, Mistral running the Python CLI.
- Parser modes: kimi-stream-json, mistral export plus session journal, qwen stream-json. Containment: macOS seatbelt
  (`sandbox-exec`) plus the repo audit on every new-seat dispatch; seat homes under `$TRIUMVIRATE_HOME/homes/`.
- Unavailable seats: none (all three AVAILABLE). Existing seats (claude, codex, gemini/antigravity, grok, deepseek) unchanged.

## Stage 2: one real sight-gated review per new prime (D-08)
Source: `/Users/michaelboscia/projects/triumvirate-seven-primes-build/scripts/check-live-evidence.sh` (required_sources;
require_sight; strict_agent). Each review opened the source (sight gate passed, 1 tool call each).

| Seat | Request id | Verdict | Finding |
|---|---|---|---|
| mistral | c8b504fb-48de-4f1b-a005-577872aa8f2d | REJECT | line 8 does not check that git succeeded, so a git failure reads as a code_tree mismatch. Confirmed and fixed in 4973234b. |
| kimi | dda6f321-ccf4-4572-9271-a22334ae8c68 | APPROVE | the timestamp is only checked for presence, so freshness rests on the tree hash. Noted (by design: the hash is the freshness contract). |
| qwen | de3e4440-fad9-42d5-92c8-6b0cc4f991a8 | APPROVE | the success line prints the seat count, not the pass count. Cosmetic. |

All three answered from the file (each cited correct line numbers for the three failure paths). Mistral's REJECT found the
only real defect of the three.

## Defects found by the launch itself (all fixed before this report)
A-13 (kimi agent file description, qwen --auth-type openai, qwen pinned qwen3.7-plus); qwen session id on `system/init`;
read evidence for all three seats (sight gate); `/status` reason null; stray-key scan placeholders; doctor CLI resolution;
check-live-evidence git failure message.

## Not covered here
Writer seats (fleet and worktree dispatch) are the next goal (docs/seven-primes/WRITER_SEATS_PLAN.md). PostHog
instrumentation of the new paths follows that.
