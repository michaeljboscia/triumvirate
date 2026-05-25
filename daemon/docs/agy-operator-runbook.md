# AGY Backend — Operator Runbook

How to operate Triumvirate's Antigravity-CLI (`agy`) backend for the `gemini` sibling.
Companion to `specs/agy-integration-spec.md` (the requirements) and
`../../research/antigravity/agy-verification/FINDINGS.md` (the live-binary evidence).

> **Hard constraints.** Subscription/OAuth only — **never** an API key
> (`ANTIGRAVITY_API_KEY`/`GEMINI_API_KEY` are forbidden by policy and unsupported by
> the binary). The public agent name stays `gemini`; `agy` is an internal backend.
> Codex is untouched. Rollback is config-only while gemini-cli still serves.

---

## 1. One-time setup (before using agy)

### 1.1 Authenticate (REQ-031)
agy authenticates via an interactive OAuth login that persists to a **plaintext file**
`~/.gemini/oauth_creds.json` (mode 600) — *not* the macOS Keychain. The Triumvirate
daemon runs as a LaunchAgent in your logged-in user session, so it reads this file
directly; no special launch context is needed.

```bash
agy                       # run once interactively, complete the OAuth login in the browser
agy -p "2+2"              # verify: a second call reuses the token with no prompt → prints 4
```
The refresh token has no local expiry stamp; persistence is governed by Google-side
revocation/inactivity. If a dispatch later fails with an auth error, just run `agy`
once interactively again.

### 1.2 Pin the binary path (REQ-033)
Install `agy` at a stable path (a fixed symlink is fine) so updates don't change the
resolved binary out from under the daemon. Confirm:
```bash
which agy                 # e.g. ~/.local/bin/agy
agy --version             # expect the pinned version (see §4)
```

### 1.3 Readiness check
```bash
triumvirate doctor        # look for the "AGY backend" section:
                          #   version: PASS (installed=…, expected=…)
                          #   probe (agy -p "2+2"): PASS (oauth + capture ok)
```
A `probe … PASS` proves OAuth **and** output capture both work non-interactively.

---

## 2. Operating modes

The backend is selected per daemon by environment, then **restart the daemon**.

### 2.1 Default — gemini-cli (no change)
Unset (or any value other than `agy`) runs the legacy gemini-cli path, byte-for-byte
unchanged. This is the rollback target while gemini-cli still serves.

### 2.2 Cutover — agy primary
```bash
TRIUMVIRATE_GEMINI_BACKEND=agy
```
Every `gemini` request is served single-turn by agy. On a hard failure the **degraded
route** takes over (see §3). Rollback is just unsetting this and restarting.

### 2.3 Side-by-side comparison — shadow mode
Run both backends and compare their answers on real traffic, without risking the
user-facing answer:
```bash
TRIUMVIRATE_GEMINI_BACKEND=gemini-cli     # primary — the trusted answer returned to the user
TRIUMVIRATE_GEMINI_SHADOW=on              # also run agy on every gemini request
```
- The primary's answer is returned as usual.
- agy's answer rides along in the response (`shadow_response`, `shadow_latency_ms`,
  `shadow_error`) **and** every comparison is appended to
  `~/.triumvirate/agy-shadow-compare.jsonl` for offline review:
  ```bash
  tail -f ~/.triumvirate/agy-shadow-compare.jsonl | jq '{prompt, primary_response, shadow_response, shadow_latency_ms}'
  ```
- Flip `BACKEND=agy` + `SHADOW=on` to compare the other direction.
- **Cost:** shadow mode doubles usage of the shared Google subscription quota pool
  (gemini-cli and agy draw from the same pool). It is a validation tool, not a
  steady-state mode — the rate limiter and concurrency cap still throttle the agy side.

---

## 3. Resilience behavior (what happens on failure)

- **Degraded route** (`TRIUMVIRATE_GEMINI_DEGRADED_ROUTE`, default `gemini-cli,codex`;
  set to `fail` to disable): when the agy primary hard-fails, the request is retried
  on the next backend in the chain. **Quota/429 failures skip gemini-cli** (it shares
  agy's exhausted pool) and go straight to **codex**. gemini-cli auto-drops out of the
  chain by failing its exec once the binary retires — no date logic. A codex answer is
  returned with honest `answered_by_*` fields and a `⚠ answered by Codex` text prefix;
  the public `agent` stays `gemini`.
- **Circuit breaker:** repeated quota/429 opens the circuit and routes Gemini work
  straight to codex; a half-open probe is tried after an exponential cooldown capped at
  the ~5-hr Ultra reset window. Prevents a retry storm.
- **Timeouts:** a hung agy (Go ignores soft signals) is SIGKILLed at the process-group
  level after the timeout, with one retry for transient hangs.
- **No silent empty success:** an empty answer is retried once then fails loud.
- **Token accounting:** agy has no honest headless token count, so its dispatches are
  recorded `usage_source=unmetered` (counted as occurrences, excluded from cost sums —
  shown as `unmetered_records` in `get_token_summary`).
- **Health:** a background probe (`TRIUMVIRATE_AGY_HEALTH_PROBE_SECS`, default 300s,
  only when agy is selected) surfaces `agy_capture_health` / `agy_backend_health` on the
  daemon `/health` endpoint — it catches a silent stdout-drop regression that live
  traffic cannot.

---

## 4. Version drift (REQ-059)

agy updates frequently. The backend runs `agy --version` once and compares it to
`TRIUMVIRATE_AGY_EXPECTED_VERSION`:
- **mismatch + default:** warns, proceeds.
- **mismatch + `TRIUMVIRATE_AGY_STRICT_VERSION=true`:** refuses the agy backend.

**On any agy upgrade:** re-run the verification battery (REQ-060–064 — see
`research/antigravity/agy-verification/`), update the captured artifacts, and bump
`TRIUMVIRATE_AGY_EXPECTED_VERSION`. Caveat: pinning the binary bounds *local* drift
only — Google can push server-side harness changes that alter agy's output with the
binary pinned.

Last verified-good version: **1.0.2** (2026-05-24).

---

## 5. Environment variables

| Variable | Default | Meaning |
|---|---|---|
| `TRIUMVIRATE_GEMINI_BACKEND` | `gemini-cli` | `agy` selects the agy backend; anything else = legacy path |
| `TRIUMVIRATE_GEMINI_SHADOW` | off | `on` also runs the other Gemini backend for comparison (§2.3) |
| `TRIUMVIRATE_AGY_BIN` / `_ARGS` | `agy` / — | binary path + extra args |
| `TRIUMVIRATE_AGY_CAPTURE` | `pipe` | `pty` is reserved (fails loud — not yet implemented) |
| `TRIUMVIRATE_AGY_CONNECTOR_TIMEOUT_SECS` | `900` | agy timeout; also passed to `--print-timeout` |
| `TRIUMVIRATE_GEMINI_DEGRADED_ROUTE` | `gemini-cli,codex` | fallback chain; `fail` disables |
| `TRIUMVIRATE_GEMINI_DEGRADED_TOTAL_TIMEOUT_SECS` | `900` | total wall-clock budget for the degraded route |
| `TRIUMVIRATE_AGY_MAX_CONCURRENT` | `3` | global cap on simultaneous agy children |
| `TRIUMVIRATE_AGY_MAX_RPM` | `30` | token-bucket call-rate ceiling |
| `TRIUMVIRATE_AGY_MAX_PROMPT_BYTES` | `900000` | fail-loud guard (no stdin/`@file` in agy) |
| `TRIUMVIRATE_AGY_HEALTH_PROBE_SECS` | `300` | health-probe interval |
| `TRIUMVIRATE_AGY_EXPECTED_VERSION` | `1.0.2` | pinned version |
| `TRIUMVIRATE_AGY_STRICT_VERSION` | off | refuse on version mismatch |
| `TRIUMVIRATE_AGY_BREAKER_THRESHOLD` | `3` | consecutive quota failures before the breaker opens |
| `TRIUMVIRATE_AGY_BREAKER_COOLDOWN_SECS` | `120` | breaker base cooldown (exponential, capped at ~5 hr) |

**Forbidden / not real:** `ANTIGRAVITY_API_KEY`, `GEMINI_API_KEY` (policy + unsupported);
`ANTIGRAVITY_SKIP_UPDATE_CHECK` (confabulated — not a real agy variable).

---

## 6. Rollback

While gemini-cli still serves: unset `TRIUMVIRATE_GEMINI_BACKEND` (or set it to
`gemini-cli`) and restart the daemon. No code change. After gemini-cli stops serving,
there is no rollback — the degraded route to codex is the safety net.
