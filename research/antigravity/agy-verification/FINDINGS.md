# agy Verification Battery — Findings

**Date:** 2026-05-24 · **Binary:** `agy` v**1.0.1** at `~/.local/bin/agy` (140 MB Go binary) · **Host:** macOS (darwin), authenticated via subscription OAuth.
**Purpose:** Resolve the spec's REQ-060–064 gates against the live binary. Raw captures in this directory (`help.txt`, `version.txt`, `config-layout.txt`, `probe-results.txt`, `probe2-results.txt`).

## Headline results

| REQ | Question | Verified result |
|---|---|---|
| **REQ-060** | Real flag set | **No `--output-format`, no `--model`/`-m`, no `--prompt-file`/stdin.** `--sandbox` exists (desc: *"Run in a sandbox with terminal restrictions enabled"*). `--print-timeout` default 5m. `-p/--print`, `-c/--continue`, `--conversation`, `--add-dir`, `--dangerously-skip-permissions`. Subcommands: changelog/help/install/plugin/update. → confirms REQ-012/013/026/058. |
| **REQ-062** | non-TTY stdout drop? | **DID NOT REPRODUCE.** `agy -p` to a pipe returned `4\n`, exit 0, ~6s — **7/7 clean** across controlled runs. PTY returned `4\r\n`, exit 0, 7.5s. → the mandatory-PTY premise (REQ-020) does not hold on 1.0.1. |
| — | Intermittent hang | **One hang observed:** the first (cold) `agy -p`, wrapped in `perl -e 'alarm…; exec'` and harness-backgrounded, hung **hours** at 0% CPU, 0 bytes. Not reproduced in 7 subsequent clean subprocess-pipe runs. Conclusion: **intermittent cold-start hang**, not a deterministic non-TTY drop. |
| **REQ-064** | Exit codes | success = **0**; bad flag = **2** (`flags provided but not defined: …` on stderr). Quota/RESOURCE_EXHAUSTED exit code **not yet sampled** (would require exhausting the 5-hour window). |
| **REQ-061** | On-disk layout | `~/.antigravity` **does not exist** (research's MCP path was wrong). `~/.antigravitycli/<uuid>.json` is a **symlink** → `~/.gemini/config/projects/<uuid>.json`. Real home is `~/.gemini/` (dirs: `antigravity/`, `antigravity-cli/`, `config/`, `history/`; files: `settings.json`, `state.json`, `projects.json`, `google_accounts.json`, `installation_id`). |
| **REQ-031/032** | Auth storage | **Plaintext file** `~/.gemini/oauth_creds.json` (mode 600, modified during session). **NOT the macOS Keychain.** → the LaunchDaemon-vs-Keychain blocker is **MOOT**; any process running as the user reads the file; persistence is file-based. 7/7 calls reused creds with no prompt (REQ-063 effectively satisfied for in-session reuse; long-run expiry still unmeasured). |

## Two Go-specific gotchas (verified)

1. **`SIGALRM` does not kill agy.** A `perl -e 'alarm 90; exec agy …'` guard failed to terminate a hung agy — the Go runtime swallows it. **Timeouts MUST `SIGKILL` the process group** (`os.killpg`/`kill -9`), which is what the working probe used. Sharpens REQ-014/020.
2. **A hang is possible (cold start).** Rare but real → a hard-kill timeout + one retry is required regardless of pipe-vs-PTY.

## Spec impact

- **REQ-020 (PTY):** premise (non-TTY drop) did not reproduce → PTY is **not required**; plain `Stdio::piped()` works. *Decision pending: simplify to pipe vs keep PTY defensively.*
- **REQ-014:** timeout must be a **SIGKILL process-group** hard-kill (Go ignores soft signals). Confirmed.
- **REQ-031/032:** auth is **file-based** (`~/.gemini/oauth_creds.json`); Keychain/LaunchDaemon concern removed.
- **REQ-061/060/064:** resolved as above; capture artifacts committed.

## Still unverified (need targeted, agentic tests)

- ~~REQ-016/062 — `--sandbox` confinement.~~ **RESOLVED 2026-05-24 (probe3): `--sandbox` does NOT confine filesystem writes.** With `agy --sandbox -p` (no `--dangerously-skip-permissions`), agy auto-executed BOTH writes — one inside the workspace and one to `/tmp` OUTSIDE it (exit 0, ~18s, replied "DONE"). Two conclusions: (1) **Issue #45 confirmed** — `agy -p` auto-runs tools without the skip flag; (2) **`--sandbox` ("terminal restrictions") is not a filesystem boundary** — the R3 decision REQ-016 is insufficient. Real containment needs our own `sandbox-exec` profile. **probe4 — VERIFIED:** a Triumvirate-controlled `sandbox-exec` profile (`allow default` → `deny file-write*` → re-allow {workspace, `~/.gemini`, `~/.antigravitycli`, temp, std devices}) **blocked an out-of-home-root write even when agy tried its built-in file tool AND shell AND python** ("Operation not permitted"), while **allowing** a read of a staged artifact OUTSIDE the workspace and network. Option A confirmed; reusable profile at `agy-sandbox.sb.template`. Consult-safety RESOLVED.
## Easy batch (probe5/probe6, 2026-05-24)
- **Concurrency:** ✅ SAFE — 3 simultaneous `agy -p` all answered correctly in parallel (~7.6s), no state collision. The concurrency cap is a quota knob, not a correctness need (REQ-055 default raised to 3).
- **ARG_MAX:** ✅ macOS limit is ~1MB (1048576), not 256KB — a 280KB prompt ran fine. Guard threshold raised to 900_000 (REQ-058).
- **Multi-turn `-c`:** ✅ continues the prior conversation (global) — confirmed.
- **`ANTIGRAVITY_CONVERSATION_ID`:** ❌ exists in the binary but does NOT give isolated resumable multi-turn for `-p` (conv B forgot its codeword; conv A hung). Single-turn (REQ-040) stands.
- **`ANTIGRAVITY_SKIP_UPDATE_CHECK`:** ❌ NOT a real var (absent from binary strings) — confabulated; REQ-059 corrected.
- **Second hang observed:** the `CONVERSATION_ID` resume call hung (killed at 90s) → hangs are intermittent + recurring (2 total), resume/state ops a suspected trigger. SIGKILL+retry is load-bearing.
- **Profile robustness:** ◐ no denied-path errors under a richer task (reads + shell + git), but the requested `out.txt` wasn't created despite "DONE" — possible false-completion; needs a clean re-test.
- **Real env vars discovered** (strings): `ANTIGRAVITY_CONVERSATION_ID`, `ANTIGRAVITY_EXECUTABLE_DATA_DIR`, `ANTIGRAVITY_PROJECT_ID`, `ANTIGRAVITY_TRAJECTORY_ID`, `ANTIGRAVITY_BROWSER_TOOLS_ENABLED`, `GEMINI_DIR`, … (no `*_API_KEY`, no `*_OUTPUT_FORMAT`, no skip-update).

## HARD batch — still open (runtime-only / time-bound)
1. **Quota / RESOURCE_EXHAUSTED behavior (REQ-051/053/064).** Need the exact exit code + string, and whether it returns empty+exit0 (the dangerous "looks like success" case). *How:* (a) instrument the wrapper to log exit/stderr verbatim on ANY failure and capture opportunistically in normal use [low cost, no control of timing]; or (b) a controlled burst to deliberately exhaust the 5-hr window at an off time [definitive, but locks out agy for the window]. Design degraded-route to treat ambiguous failures conservatively (→ codex, never assume success on empty).
2. **Long-run token expiry (REQ-063).** **(a) DONE 2026-05-24:** read `~/.gemini/oauth_creds.json` — access token ~1 hr (auto-refreshed), **refresh token present with NO local expiry stamp** → no evidence of a short fuse; persistence is governed by Google-side revocation/inactivity (unobservable locally). Scope includes `cloud-platform`. Conclusion: file-based auth looks durable. **(b) still needed:** instrument auth-failure logging to catch the first real re-auth over days.
3. **Intermittent hang — CHARACTERIZED 2026-05-24 (probe7): 0/30 hung** across plain / `-c` / `CONVERSATION_ID`-resume (the condition that hung once in probe6 did NOT reproduce in 10 retries). Total ~2 hangs in ~52 calls, both early/transient; last 34 consecutive clean. Conclusion: rare, non-deterministic transient, NOT tied to resume/state ops. The SIGKILL timeout + one retry is sufficient (independent transient → retry succeeds with high probability). **Closed.**
