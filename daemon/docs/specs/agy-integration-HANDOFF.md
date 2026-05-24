# AGY Integration — Build Handoff (start here on a fresh context)

**Purpose:** resume the build of Triumvirate's Gemini→Antigravity-CLI (`agy`) migration with zero prior context. Read this top to bottom, then open the spec. Everything below is **verified against the live binary (agy 1.0.1, macOS, this machine) unless marked otherwise.**

**Mission:** replace Triumvirate's `gemini` CLI backend with `agy` before **2026-06-18** (the day the legacy Gemini CLI stops serving), preserving the maximum functionality possible under a **subscription-only** auth model.

---

## 0. Read-first file map
- **Spec (source of truth, ~60 REQs):** `daemon/docs/specs/agy-integration-spec.md` — every requirement is `REQ-###`, decisions are inline-tagged (R1/R2/R3, VERIFIED 2026-05-24).
- **Verification record + raw evidence:** `research/antigravity/agy-verification/FINDINGS.md` + `probe1..9` scripts + `*-results.txt` + `help.txt`/`config-layout.txt`.
- **Verified sandbox profile (ship this):** `research/antigravity/agy-verification/agy-sandbox.sb.template`.
- **Original research (two briefs + synthesis):** `research/antigravity/` (`Migration Brief…(Claude-web)`, `Migration Analysis…(Gemini source)`, `triumvirate-agy-migration-synthesis.md`). Background only — superseded by the spec + FINDINGS where they conflict.
- **Process:** this spec went through `/goatrodeo` (Phase 0 + 3 rounds + Phase 3 CLEAN + decision ledger). The build is goatrodeo **Phase 4 → 8**.

## 1. Hard constraints (do not violate)
- **C1 — Subscription/OAuth ONLY. Never an API key.** (`ANTIGRAVITY_API_KEY`/`GEMINI_API_KEY` are forbidden AND unsupported by agy.)
- **C2 — Host is macOS.** Auth persists via a plaintext file `~/.gemini/oauth_creds.json` (NOT the Keychain).
- **C3 — Public agent name stays `gemini`.** The agy swap is an internal backend, not a new agent surface.
- **C4 — Codex path untouched.**
- **C5 — Rollback is config-only** while gemini-cli still serves (until 2026-06-18; after that there is no rollback — see degraded route).

## 2. What we're building (architecture, post-verification)
A new **`agy` backend** for the `gemini` agent, selected by env, living beside the existing gemini path (which stays intact for rollback + the degraded route):
1. **Backend selector** at the existing seam → dispatch to a new `run_agy_cli_process_with_session`.
2. **Invocation:** `agy --sandbox -p <message>`, stdin null. NO `-o/--output-format`, NO `-r/--resume`, NO `-c`, NO `--model`. Pass `--print-timeout` = our agy timeout. Pass `--log-file <per-dispatch temp>`.
3. **Capture:** plain `Stdio::piped()` (the non-TTY drop did NOT reproduce). A **SIGKILL-process-group hard-kill timeout + one retry** handles the rare transient hang (Go ignores SIGALRM — soft kills don't work). A PTY path stays behind `TRIUMVIRATE_AGY_CAPTURE=pty` as a regression fallback.
4. **Containment:** spawn agy under **our own `sandbox-exec` profile** (`agy-sandbox.sb.template`) — denies file-writes outside {workspace, `~/.gemini`, temp}, leaves reads + network open. (agy's own `--sandbox` does NOT confine writes; agy `-p` auto-executes tool writes without the skip flag — Issue #45.) NEVER combine with `--dangerously-skip-permissions`.
5. **Result:** `parser_mode="agy-pty-plain-text"` (or `agy-pipe-plain-text`), `session_id=None`, ANSI-stripped text; bypass `GeminiStreamParser`.
6. **`--log-file` parsing** for observability: serving model (`Propagating selected model … label="Gemini 3.1 Pro (High)"`), auth method (`authMethod=consumer`), error/quota class. NOT token counts (none exist headless). Parse FINAL state, never generic error-grep (startup logs benign "not logged in" warnings even on success). Parse-then-delete.
7. **Single-turn** (multi-turn is unavailable; `ANTIGRAVITY_CONVERSATION_ID` does not resume). Named sessions get a visible "multi-turn not available" notice; never fake continuity.
8. **Resilience (the 429 story):** provider **circuit breaker** + **rate limit/backpressure** + **degraded route**. On quota/429 → do NOT retry (→ storm); trip breaker → route to **codex** (NOT gemini-cli, same quota pool). Transient/hang → backoff + 1 retry.
9. **Token accounting:** `usage_source = exact|estimated|unmetered`; agy rows = `unmetered` (no honest headless count exists), excluded from cost sums.
10. **Degraded substitution honesty:** when codex answers a `gemini` request, response carries `answered_by_agent`/`degraded_from_backend`/`degradation_reason` + a text prefix.

## 3. Verified ground truth (reality vs. the original assumptions)
| Topic | Reality (verified 2026-05-24) |
|---|---|
| Binary | `agy` **1.0.1**, `~/.local/bin/agy` (140MB Go). Model served: **Gemini 3.1 Pro (High)**, `authMethod=consumer`, backend `cloudcode-pa.googleapis.com/v1internal`. |
| Auth | **plaintext file** `~/.gemini/oauth_creds.json` (not Keychain) → LaunchDaemon/Keychain blocker is MOOT; daemon is already a **LaunchAgent**. Refresh token has no local expiry → durable. |
| Flags | NO `--output-format`, NO `--model`, NO stdin/`@file`. `--sandbox`/`--print-timeout`(5m)/`--log-file`/`-c`/`--conversation`/`--add-dir`/`--dangerously-skip-permissions` exist. |
| stdout | non-TTY drop did **NOT** reproduce — pipe capture 7/7 clean → **pipe default**, PTY optional. |
| Hang | rare transient (~2 in ~52 calls, 0/30 controlled). SIGKILL+retry sufficient. |
| Kill | **SIGALRM does NOT kill agy** (Go) → must SIGKILL the process group. |
| Tool exec | `agy -p` **auto-writes files without `--dangerously-skip-permissions`** (Issue #45). agy's `--sandbox` does **not** confine writes (wrote to `/tmp`). |
| Containment | **our `sandbox-exec` profile WORKS** — blocked out-of-workspace writes via built-in tool + shell + python; allowed staged-artifact reads + network. |
| Concurrency | safe (3 parallel clean) — cap is a quota knob (default 3), not correctness. |
| ARG_MAX | ~1MB (280KB worked) → guard 900KB. |
| Multi-turn | unavailable headless (`CONVERSATION_ID` doesn't resume) → single-turn. |
| Tokens | NO headless source (slash cmds TUI-only; asking the model returns a fabricated number; no `usageMetadata` in stdout/log) → `unmetered`. |
| Quota | exact exit-code/string UNKNOWN (only on a real lockout) → instrument + treat ambiguous as quota. Shared quota pool with gemini-cli. |
| Observability | agy dropped gemini-cli's `--telemetry`/OTLP. DIY only. Official `google-antigravity` SDK is **API-key-only** → unusable under C1. |
| Exit codes | success=0, bad-flag=2 (quota TBD). |

## 4. Code map (insertion points — verify lines on fresh checkout, code drifts)
- **Backend dispatch seam:** `daemon/crates/triumvirate/src/agent_exec.rs:1682` (`run_agent_process_with_session`). Add the agy branch here; keep `run_gemini_cli_process_with_session` (`:1211`) + batch (`:1332`) intact.
- **New fn** `run_agy_cli_process_with_session` beside `:1211`.
- **Command resolution / new env helpers:** `daemon/crates/mcp-bridge/src/lib.rs` (`gemini_command`/`resolve_connector_command` ~`:220`; supported agents `:34` — keep `gemini`+`codex` only; streaming default `:249`).
- **`--model` faildown to disable for agy:** `agent_exec.rs:335` + `:918`; **prewarm to gate off:** `:976/:984`; **`has_any_arg` (exact-match, misses `--prompt=`):** `:1155`; **heartbeat:** `:453/:492`; **degraded/error path:** `:582` (+ dispatch `:1658`).
- **Worker session registry (single-turn, no synthetic id):** `daemon/crates/agent-worker/src/lib.rs:16/78/117/128`; writeback `agent_exec.rs:525/586/962`.
- **Stream parser (bypass for agy):** `daemon/crates/agent-adapter/src/gemini.rs` (`GeminiStreamParser`).
- **Token economics (unmetered schema):** `daemon/crates/token-economics/src/{attribution.rs:47/58, queries.rs:89/134, storage.rs:14, lib.rs:36}`; usage→zeros today at `agent_exec.rs:153/211`; summaries `mcp-tools/src/token_tools.rs:123/173`.
- **Fleet second site (REQ-090):** `daemon/crates/fleet/src/orchestrator.rs:81` (direct `gemini -p`, piped) + fan-out `:295/:303` — must use the shared selector + concurrency cap.
- **Named/daemon sessions (warn):** `mcp-tools/src/inter_agent.rs:103/155/170`; daemon HTTP `main.rs:2041/2055`.
- **Response schema (degraded honesty fields):** `daemon/crates/shared-types/src/lib.rs:39` (`AskAgentResponse`).
- **doctor (agy readiness checks):** `cli_ops.rs:59` (`run_doctor`), wired `main.rs:147/1301/1323`; health/status `main.rs:533/641`.
- **LaunchAgent confirmation:** `daemon/crates/daemon-core/src/lib.rs` (`render_launch_agent`, `~/Library/LaunchAgents/com.triumvirate.daemon-v2.plist`).
- **Dep to add:** `portable-pty` to `daemon/crates/triumvirate/Cargo.toml` (in Cargo.lock, not declared) — only needed for the PTY fallback path.

## 5. Env knobs (introduced by this work)
```
TRIUMVIRATE_GEMINI_BACKEND=gemini-cli|agy          # default gemini-cli
TRIUMVIRATE_AGY_BIN=agy                              # + TRIUMVIRATE_AGY_ARGS
TRIUMVIRATE_AGY_CAPTURE=pipe|pty                     # default pipe
TRIUMVIRATE_AGY_CONNECTOR_TIMEOUT_SECS=900           # vs gemini/codex 180
TRIUMVIRATE_GEMINI_DEGRADED_ROUTE=gemini-cli,codex   # or "fail"
TRIUMVIRATE_GEMINI_DEGRADED_TOTAL_TIMEOUT_SECS=900
TRIUMVIRATE_AGY_MAX_CONCURRENT=3
TRIUMVIRATE_AGY_MAX_PROMPT_BYTES=900000
TRIUMVIRATE_AGY_HEALTH_PROBE_SECS=300
TRIUMVIRATE_AGY_EXPECTED_VERSION=1.0.1               # + TRIUMVIRATE_AGY_STRICT_VERSION
```
**Not real / forbidden:** `ANTIGRAVITY_SKIP_UPDATE_CHECK` (confabulated, no-op), `ANTIGRAVITY_API_KEY`/`GEMINI_API_KEY` (forbidden by C1, unsupported anyway).

## 6. Build plan (goatrodeo Phase 4 → 8)
- **Phase 4 — Canonical docs + execution plan** (`/uncompromising-executor` → PRD, TEST_PLAN with reality tests, IMPLEMENTATION_PLAN, etc.). Trait-to-task traceability.
- **Phase A (MVP, must land before 2026-06-18):** backend selector; `run_agy_cli_process_with_session` (pipe + SIGKILL/retry); sandbox-exec wrapper; `--log-file` model/auth/error parse; single-turn + warn; degraded route + breaker + rate-limit; unmetered token rows; ARG_MAX guard; fleet second site; doctor checks; version pin. Feature-flagged + config-rollback.
- **Phase B (recover, after MVP safe):** quota-signal instrumentation (capture on first real event), optional char-based `estimated` tokens, long-run token-expiry logging. (Multi-turn + streaming are NOT recoverable under C1 — do not chase.)
- **Reality tests (mandatory):** mock `agy` that emits nothing when `[ -t 1 ]` false (a pipe-stub fails, real impl passes); concurrency test asserts no resume flags + no synthetic session id; rollback test asserts gemini-cli path still gets stream-json; sandbox test asserts out-of-workspace write blocked.

## 7. Gotchas / landmines
- **Go ignores SIGALRM** — `perl alarm`/soft kills won't stop a hung agy. SIGKILL the process group.
- **agy auto-writes files** on a plain `-p` (Issue #45) — the sandbox-exec wrapper is the only thing stopping repo mutation. Don't skip it.
- **`--log-file` startup noise:** "You are not logged into Antigravity" / "Failed to get OAuth token" appear on SUCCESSFUL calls (startup race before auth). Never treat generic log errors as failure.
- **gemini-cli fallback is useless for quota** (shared pool) — quota → codex directly.
- **gemini-search was wrong repeatedly** this session (claimed API-key works, keyring not file, SDK supports OAuth — all false). Trust the binary + PyPI + primary sources over it.
- **The SDK path is a dead end under C1** (API-key only). The only subscription-programmatic alternative is calling private `cloudcode-pa` with IDE-spoofing headers — grey-area, rejected.
- **`agy --help` is authoritative** — re-capture it after any agy upgrade and re-run the verification battery (REQ-059/060–064); the binary changes fast.

## 8. Open items (deferred, non-blocking)
- Quota exact exit-code/string → wrapper instrumentation captures on first real lockout (don't force it).
- Long-run (multi-day) token refresh cadence → auth-failure logging.
- Optional `estimated` token volume (char-based) if a rough dashboard number is ever wanted.

## 9. Live assets
- Persistent scoping daemons: `agy-migration-codex` (code) and `agy-migration-gemini` (research) — still warm with this context if reachable; re-spawn if not.
- Memory: `~/.claude/projects/.../memory/gemini-cli-to-antigravity.md` + `auth-subscriptions-only-never-api-keys.md`.

---
**First action on the fresh context:** read `agy-integration-spec.md` in full, then run goatrodeo Phase 4 (`/uncompromising-executor`) against it — or, if building directly, start at the backend selector (REQ-001–005) and the `run_agy_cli_process_with_session` skeleton (REQ-010–026), behind `TRIUMVIRATE_GEMINI_BACKEND=agy`, with the sandbox-exec wrapper from day one.
