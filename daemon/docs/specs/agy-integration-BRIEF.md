# AGY Integration — BRIEF (new-session entry point)

> You are picking up a migration that has already been **spec'd, goatrodeo-reviewed, and verified against the live binary.** Your job is to **build it.** Read this (1 min) → then the HANDOFF (5 min) → then the SPEC (REQ detail). Do not re-litigate decisions that are marked VERIFIED.

## Mission (one line)
Replace Triumvirate's `gemini` CLI backend with the Antigravity CLI (`agy`) **before 2026-06-18** (legacy Gemini CLI stops serving that day), under a **subscription-only** auth model, preserving maximum functionality.

## Status
- ✅ Spec written + `/goatrodeo` (Phase 0 + 3 rounds + Phase 3 CLEAN + ledger).
- ✅ **Verified against the live binary** (agy 1.0.1) — every load-bearing assumption checked; reality *simplified* several decisions.
- ✅ Committed + PR'd: branch `docs/agy-integration`, **PR #34 → `main`** (docs only).
- ⬜ **Build not started.** ← that's you (goatrodeo Phase 4 → 8).

## The doc set (read in this order)
1. **THIS BRIEF** — orientation.
2. **`daemon/docs/specs/agy-integration-HANDOFF.md`** — full context: architecture, `file:line` code map, env knobs, gotchas, phased plan.
3. **`daemon/docs/specs/agy-integration-spec.md`** — the ~60 `REQ-###` requirements (source of truth).
4. Evidence (as needed): **`research/antigravity/agy-verification/FINDINGS.md`** + probes 1–9 + the verified **`agy-sandbox.sb.template`**.

## Non-negotiables (constraints)
- **Subscription/OAuth only — NEVER an API key** (and the SDK is API-key-only, so don't use it).
- Public agent name stays **`gemini`**; agy is an internal backend behind `TRIUMVIRATE_GEMINI_BACKEND`.
- **Codex path untouched.** Changes are additive + feature-flagged + config-rollbackable.
- Spawn agy under **our `sandbox-exec` profile from day one** (agy `-p` auto-writes files; its own `--sandbox` doesn't contain them).

## Don't re-litigate (VERIFIED reality)
- Auth = plaintext file `~/.gemini/oauth_creds.json` (not Keychain); daemon is a LaunchAgent → no auth blocker.
- **Pipe capture works** (no stdout drop) → default pipe; PTY behind `TRIUMVIRATE_AGY_CAPTURE=pty` fallback only.
- Timeouts must **SIGKILL the process group** (Go ignores SIGALRM); +1 retry handles the rare transient hang.
- **Single-turn only** (no resumable multi-turn); **tokens `unmetered`** (no honest headless count); model served = Gemini 3.1 Pro (High), readable from `--log-file`.
- 429/exhaustion → **provider circuit-breaker → codex** (NOT gemini-cli; shared quota pool). Model-level failover is gone.
- No `--output-format`/`--model`/stdin/`@file`. ARG_MAX ~1MB. Concurrency safe (cap is a quota knob).

## First moves
1. Confirm PR #34 is merged (or merge it); `git checkout main && git pull`; branch fresh (`feat/agy-backend`).
2. Verify `agy` is installed + authed: `agy --version` (expect 1.0.1) and a quick `agy -p "2+2"`. If agy upgraded, re-capture `agy --help` and re-run the REQ-060–064 battery first — the binary changes fast.
3. Run **goatrodeo Phase 4** (`/uncompromising-executor`) against `agy-integration-spec.md` to produce the canonical docs + execution plan — OR, if building directly, start at the backend selector (REQ-001–005) + the `run_agy_cli_process_with_session` skeleton (REQ-010–026) with the sandbox-exec wrapper wired in.
4. **Phase A (must land before 2026-06-18):** selector, pipe capture + SIGKILL/retry, sandbox wrapper, `--log-file` model/auth/error parse, single-turn + warn, degraded route + breaker + rate-limit, unmetered tokens, ARG_MAX guard, fleet second site, doctor checks, version pin.

## Live assets
- Scoping daemons (may still be warm; re-spawn if not): `agy-migration-codex` (code), `agy-migration-gemini` (research).
- Memory: `gemini-cli-to-antigravity.md`, `auth-subscriptions-only-never-api-keys.md`.
