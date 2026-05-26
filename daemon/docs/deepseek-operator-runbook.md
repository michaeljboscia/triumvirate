# DeepSeek — Operator Runbook

> **Purpose:** Everything an operator needs to safely run the DeepSeek sibling
> agent in the Triumvirate daemon. Mirrors `agy-operator-runbook.md` in
> structure. DeepSeek is **remote+metered+paid** — read the funding and
> data-egress sections before enabling.
>
> **Branch where this landed:** `spec/deepseek-integration-v1`
> **Spec:** `daemon/docs/specs/deepseek-integration-spec.md`
> **Implementation plan:** `daemon/docs/v1-deepseek/IMPLEMENTATION_PLAN.md`
> **Live API contract verification:** `daemon/docs/v1-deepseek/PROBE_RESULTS.md`

---

## 1. One-time setup (before enabling DeepSeek)

### 1.1 Account must be funded — REQ-DS-017
DeepSeek is a paid API. There is **no usable free tier** for the v4-pro / v4-flash
models we integrate (the “free 5M token grant” claim from the public docs was
empirically absent on our funded account — see PROBE_RESULTS.md). The operator
**must top up the account before enabling DeepSeek dispatch**.

  1. Open the DeepSeek console → Billing → Top-up.
  2. Recommended starting balance: **$10**. The contract probe battery (8 calls)
     consumes approximately $0.01 total, so $10 buys hundreds of full audit
     runs plus production headroom.
  3. Verify funding lands BEFORE setting `TRIUMVIRATE_DEEPSEEK_API_KEY` in any
     daemon environment.

### 1.2 Balance monitoring — GET /user/balance (REQ-DS-017)
The DeepSeek API exposes a `GET /user/balance` endpoint. The daemon does NOT
poll it automatically (avoiding unnecessary calls). Operators MUST check it
manually before and after high-volume periods, and during incident response:

```sh
curl -sS https://api.deepseek.com/user/balance \
  -H "Authorization: Bearer $TRIUMVIRATE_DEEPSEEK_API_KEY" \
  | jq '.balance_infos[] | {currency, total_balance, granted_balance, topped_up_balance}'
```

When the breaker latches `HardOpenInsufficientBalance` (HTTP 402), the operator
should hit this endpoint to confirm balance status before re-funding. The
breaker stays open until manually reset — it does NOT auto-recover from 402.

### 1.3 Configure the API key
Set the API key in the daemon's environment. Same shape as Codex/Gemini:

```sh
export TRIUMVIRATE_DEEPSEEK_API_KEY="sk-..."   # required
```

The key is wrapped in a redacted-Debug `ApiKey` newtype (mcp-bridge/src/
deepseek_config.rs) so it cannot leak into tracing spans, panic messages,
error chains, or the per-request log file. Plaintext access is restricted to
`ApiKey::expose()`, called exactly once per request in the `Authorization`
header construction at `mcp-bridge/src/deepseek.rs::run_inner`.

**Recommended: use the on-disk key file** instead of the env var, since the
daemon is launched by the MCP shim and the env-var path requires the parent
shell of every launch chain to have the variable exported. The file fallback
is set once and works for every future daemon process regardless of who
launches it:

```sh
mkdir -p ~/.triumvirate
echo -n 'sk-YOUR-KEY-HERE' > ~/.triumvirate/deepseek.key
chmod 600 ~/.triumvirate/deepseek.key
```

The daemon checks `TRIUMVIRATE_DEEPSEEK_API_KEY` first (so tests and CI can
override) then falls back to the file. If NEITHER is set, the daemon fails
loud at first DeepSeek consult with a typed `MissingApiKey` error that
names BOTH searched sources — operator immediately knows where to put the
key. (Pre-2026-05-26 the daemon would silently send an empty `Bearer ` to
the API and surface a misleading HTTP 401 — see the bug-fix commit at PR #38.)

The file path is configurable via `$TRIUMVIRATE_HOME` (defaults to
`$HOME/.triumvirate/`). Loose file permissions (mode > 0600) get a runtime
warning but the daemon still reads the file — fail-soft for operator
flexibility.

### 1.4 Smoke test
After setting the key and confirming balance, run the live contract probe
battery as a smoke test:

```sh
cargo test -p triumvirate --test deepseek_contract -- --ignored --nocapture
```

All 8 probes should pass. The expected wall-clock is ~4–11 seconds. The cost
of one full run is approximately **$0.01** against your funded account.

---

## 2. Operating modes

### 2.1 Default — disabled
Without `TRIUMVIRATE_DEEPSEEK_API_KEY` set, dispatch into the `deepseek` arm
returns Err at first use (the runner's lazy `OnceLock` init returns
`ConfigError`-wrapped anyhow). No background cost, no surprise dispatch.

### 2.2 Enabled — direct consults
Once the key is set, `ask_agent({agent:"deepseek", message:"..."})` from any
MCP client (Claude Code, Codex, Triumvirate-aware tooling) routes through to
the DeepSeek streaming API. The default consult is content-only (CoT
bifurcation per REQ-DS-023 — see §6).

### 2.3 Per-call overrides — REQ-DS-027
The ask_agent payload accepts four optional DeepSeek-specific fields (defined
in `shared-types/src/lib.rs::AskAgentRequest`):

| Field | Type | Effect |
|---|---|---|
| `deepseek_thinking` | `"enabled" \| "disabled"` | Overrides `TRIUMVIRATE_DEEPSEEK_THINKING` for this call only. |
| `deepseek_reasoning_effort` | `"low" \| "medium" \| "high" \| "max" \| "xhigh"` | Low/Medium/High → API "high"; Max/Xhigh → API "max". |
| `deepseek_include_reasoning` | `true \| false` | When `true`, the response carries `<reasoning>…</reasoning>` ahead of the content. |
| `deepseek_max_tokens` | `u32` | Per-call max_tokens override. |
| `deepseek_model` | `"deepseek-v4-pro" \| "deepseek-v4-flash"` (string) | **Default: `deepseek-v4-pro`** (held pending capability eval — see `PRO_VS_FLASH_TEST_PLAN.md`). Per-call override of `cfg.model`. Unknown values surface as HardProvider(400). Flip the default with `TRIUMVIRATE_DEEPSEEK_MODEL=deepseek-v4-flash` operator-wide; or use the per-call field for selective override. |

Gemini and Codex callers ignore these fields (they're optional and
`#[serde(skip_serializing_if = "Option::is_none")]`).

---

## 3. Resilience behavior

### 3.1 Three-state breaker — REQ-DS-006 / REQ-DS-010
The DeepSeek runner uses an explicit three-state circuit breaker
(`mcp-bridge/src/deepseek_resilience.rs::Breaker`). Unlike the agy breaker
(which is keyed to quota windows), this one is keyed to HTTP failure classes:

| State | Trigger | Recovery |
|---|---|---|
| `Closed` | default | requests pass |
| `HardOpenInsufficientBalance` | HTTP 402 | **manual operator reset only** — does NOT auto-recover; top up + restart daemon |
| `OpenTransient{until, attempts}` | 3 consecutive 429/5xx | cooldown elapses → HalfOpen |
| `HalfOpen{lease, attempts}` | cooldown ended | one probe; success → Closed, failure → OpenTransient with grown cooldown (2× per attempt, capped 10min) |

### 3.2 Peak-hour 503 expectation — 02:00 to 14:00 UTC
DeepSeek's published infrastructure operates under heavier load during this
window. Expect **503 transient failures more frequently** here; the breaker
will OpenTransient on 3-in-a-row and back off with exponential cooldown.
This is correct behavior — DO NOT lower the breaker's transient threshold to
chase availability during peak.

### 3.3 Breaker tuning
`BreakerConfig` defaults (in `deepseek_resilience.rs`):

| Knob | Default | Notes |
|---|---|---|
| `transient_threshold` | 3 | consecutive 429/5xx before OpenTransient |
| `base_cooldown` | 30s | initial cooldown duration |
| `backoff_multiplier` | 2.0 | per-attempt growth factor |
| `max_cooldown` | 600s (10 min) | ceiling on cooldown growth |
| `half_open_lease` | 60s | how long the probe slot stays open before re-opening |

To tune in production, edit `BreakerConfig::default()` and re-deploy. There is
NO env-knob for these today — they are not load-bearing enough to warrant the
config surface (per REQ-DS-015 / round-3 review).

### 3.4 Other in-runner safety mechanisms

- **Rolling read_timeout (REQ-DS-007)** — `cfg.read_timeout` (default 60s) is
  the per-chunk idle limit; each SSE byte resets it. Long thinking-mode
  responses are tolerated as long as data keeps trickling.
- **Absolute SLA ceiling (REQ-DS-024)** — `cfg.timeout` (default 1800s) wraps
  the whole consult in `tokio::time::timeout`. Fires loud as
  `AbsoluteTimeoutExceeded`.
- **Concurrency cap (REQ-DS-006)** — `cfg.max_concurrent` (default 8) limits
  in-flight consults. Backpressure via `tokio::sync::Semaphore`.
- **RPM cap (REQ-DS-006)** — `cfg.max_rpm` (default 60) wraps a TokenBucket
  with a non-await-held mutex. Set lower to gentle-traffic the API.
- **Runaway-reasoning cap (REQ-DS-028)** — `cfg.reasoning_cap_tokens`
  (default 0 = disabled). When >0, the parser aborts the stream once the
  observed reasoning_content / 4 exceeds the cap. Does NOT trip the breaker
  (budget signal, not provider fault).
- **Anti-bulk byte cap (REQ-DS-025)** — `TRIUMVIRATE_DEEPSEEK_BULK_BYTES`
  (default 16384). DeepSeek `ask_agent` payloads over this size are rejected
  at the entry with "payload too large" + "metered" in the error. Gemini and
  Codex are local CLIs and are NOT subject to this cap.

---

## 3.5 Operator findings from the 2026-05-26 live TIER-2 probes

Two surprises surfaced when we ran the validator probe sweep against the
live API. Neither requires a code change — they're calibration notes for
the operator.

### Cache hits are opportunistic, NOT a reliable cost-saver
Probe B.11 sent an identical 76-token prompt twice (above the documented
64-token chunk threshold), 2 seconds apart. **Both calls reported zero
cache hits** (`hit=0 miss=76` × 2). This matches DeepSeek's "best-effort"
disclaimer in the official docs but contradicts the implied 120× discount
($0.003625/M cache hit vs $0.435/M miss for v4-pro). Plan operational
budgets assuming **miss-rate pricing**; treat cache hits as a windfall, not
a baseline.

### `max_tokens` must cover reasoning AND content
Probe B.14: `max_tokens=30` + `thinking=enabled` + a math prompt →
`finish_reason=length`, `content=""`, 30 reasoning tokens, **zero content
tokens**. The caller paid for 30 output tokens and got nothing usable.
Our `BadFinishReason::Length` typed failure catches this, but the
mitigation is operator-side: **set `max_tokens` high enough to cover the
reasoning + the answer**. Suggested floors with thinking enabled:
  - Trivial yes/no / single-token answer: ≥256 tokens
  - Non-trivial single-paragraph answer: ≥1024 tokens
  - Complex reasoning + answer: ≥4096 tokens
Below those floors, expect to pay for thinking that never produces output.

## 4. Observability

### 4.1 Per-request log file — REQ-DS-018 / REQ-DS-023
Every successful consult writes a JSON file at:

```
$TRIUMVIRATE_DEEPSEEK_LOG_DIR/<request_id>.json
```

Default `log_dir` is `$HOME/.triumvirate/deepseek-logs/`. The record contains:

| Field | Source |
|---|---|
| `request_id` | DeepSeek's `id` from the first chunk |
| `model` | `cfg.model` |
| `system_fingerprint` | DeepSeek's `system_fingerprint` (when present) |
| `reasoning_content` | model trace (truncated to `log_reasoning_cap_bytes`, default 256KB, UTF-8-safe) |
| `content` | the final answer |
| `usage` | `{input_tokens, output_tokens, cached_tokens, usage_source}` |
| `cost_usd` | populated by T-013's persist hook |
| `finish_reason` | "stop" on success |
| `timestamp` | RFC3339 UTC |

**system_fingerprint capture (REQ-DS-018):** the fingerprint is captured to
each per-request log file. Operators can grep `$LOG_DIR/*.json` for any
fingerprint change — that flags a backend model rollover by DeepSeek (their
v4-pro/v4-flash deployments are versioned internally even though the public
model name stays stable).

### 4.2 Privacy guarantees baked into the log shape
The serialised record SHAPE forbids:
  - `api_key` / `Authorization` field names — never serialised
  - `messages` field — request payload is NEVER persisted, only response
    artifacts (per REQ-DS-023 scope_out)

There is a regression test
(`deepseek_per_request_log_writes_expected_fields_and_excludes_secrets`) that
runs on every CI invocation asserting this. A future change that landed the
request body in the log file would fail this test.

**Operator caveat:** `reasoning_content` and `content` are MODEL OUTPUT. A
model that parrots the user's prompt verbatim CAN cause prompt fragments to
appear in the log indirectly. This is not a daemon-side secret leak — the
runner never writes the request payload — but it is a data-retention concern
for deployments with sensitive prompts. Set `log_reasoning_cap_bytes` very low
(e.g. 256) to truncate near-aggressively if required.

### 4.3 Token economics — REQ-DS-009 / T-013
Token records are persisted to the same `token-economics` SQLite database
that Gemini/Codex consults populate. The DeepSeek mapping
(`mcp-bridge/src/deepseek.rs::map_usage`):

```
input_tokens  ← prompt_cache_miss_tokens
output_tokens ← completion_tokens     (NOT + reasoning_tokens — A-04b)
cached_tokens ← prompt_cache_hit_tokens
```

When the usage chunk is missing (mid-stream disconnect, runaway abort), the
runner falls back to `usage_source = "estimated"` via `bytes_received / 4`.

T-013's persist-before-Err hook ensures that even FAILED consults (402, 429,
mid-stream disconnect) land a token record so cost attribution doesn't lose
billable tokens. The blast-radius guard
(`persist_deepseek_err_path_is_gated_to_deepseek_agent_only` test) ensures
this is ONLY applied to DeepSeek — Gemini/Codex Err paths are untouched.

### 4.4 Price seed — `deepseek-v4-pro` / `deepseek-v4-flash`
On daemon startup, `init_process_token_db` calls
`token_economics::ensure_deepseek_prices` which seeds two rows in
`price_table`:

| model | input_per_mtok | output_per_mtok | cached_per_mtok |
|---|---|---|---|
| `deepseek-v4-pro` | $0.435 | $0.870 | $0.003625 |
| `deepseek-v4-flash` | $0.140 | $0.280 | $0.002800 |

Seeded with `effective_date = 2026-01-01T00:00:00Z`, `end_date = NULL`. The
idempotent SELECT-then-INSERT pattern checks for an ACTIVE OPEN-ENDED row
(Codex W1-review SHOULD-FIX #2) so a stale/future-dated row doesn't block the
canonical seed.

---

## 5. Data-egress — read this before enabling

DeepSeek is operated by **Hangzhou DeepSeek Artificial Intelligence Co., Ltd.**
The `api.deepseek.com` endpoint resolves to infrastructure routed through
mainland China. Sending a consult to DeepSeek means:

  - **The consult content (the `message` field) is sent to DeepSeek's
    infrastructure.** It is subject to DeepSeek's data-handling policies and
    the prevailing legal regime of the operator's jurisdiction.
  - **The response (content + reasoning_content) returns from DeepSeek's
    infrastructure** and is captured to the per-request log file on the
    daemon's local disk.
  - DeepSeek's stated retention/usage policies are documented at
    `https://api-docs.deepseek.com` — operators should review the current
    policy before enabling, since terms can change without daemon-side
    notification.

**Do NOT route content through DeepSeek that is:**
  - Subject to export-control restrictions (ITAR, EAR, DFARS, etc.).
  - PII covered by HIPAA / GDPR with no explicit cross-border transfer basis.
  - Proprietary code / data the operator is not authorised to share with a
    third-party model provider.

For deployments under those constraints, **do NOT set
`TRIUMVIRATE_DEEPSEEK_API_KEY`** and DeepSeek dispatch remains disabled.

---

## 6. CoT bifurcation — REQ-DS-023

By default (`deepseek_include_reasoning = None` or `false`), the
`AskAgentResponse.response` field carries **the model's final `content`
only**. The reasoning trace is captured to the per-request log file
(`reasoning_content` field) but is NOT returned to the caller's `.response`.

When the caller sets `deepseek_include_reasoning: true`, the response is:

```
<reasoning>
{the model's reasoning trace}
</reasoning>

{the final content}
```

The wrapper tags make the bifurcation visually explicit so downstream parsers
can strip or keep the trace as policy dictates. The reasoning trace is NOT
sent to `OutboxEvent.detail` — it lands only in the per-request log file and,
when opted in, in the response body.

---

## 7. Environment variables

| Variable | Default | Notes |
|---|---|---|
| `TRIUMVIRATE_DEEPSEEK_API_KEY` | (required) | Empty/absent disables DeepSeek dispatch. Wrapped in redacted-Debug `ApiKey`. |
| `TRIUMVIRATE_DEEPSEEK_BASE_URL` | `https://api.deepseek.com/v1` | Override only for local stub servers. |
| `TRIUMVIRATE_DEEPSEEK_MODEL` | `deepseek-v4-pro` | Set to `deepseek-v4-flash` for the faster/cheaper model. |
| `TRIUMVIRATE_DEEPSEEK_MAX_TOKENS` | `32768` | Per-request response token budget. Invalid values FAIL LOUD (Codex W1 fix). |
| `TRIUMVIRATE_DEEPSEEK_THINKING` | `enabled` | `disabled` suppresses reasoning_content emission. |
| `TRIUMVIRATE_DEEPSEEK_REASONING_EFFORT` | `high` | `max`/`xhigh` raises the reasoning ceiling. |
| `TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS` | `60` | Rolling per-chunk idle limit (REQ-DS-007). |
| `TRIUMVIRATE_DEEPSEEK_TIMEOUT_SECS` | `1800` | Absolute SLA ceiling (REQ-DS-024). |
| `TRIUMVIRATE_DEEPSEEK_TCP_KEEPALIVE_SECS` | `30` | Detects dead peers between requests. |
| `TRIUMVIRATE_DEEPSEEK_MAX_CONCURRENT` | `8` | In-flight consult cap. |
| `TRIUMVIRATE_DEEPSEEK_MAX_RPM` | `60` | Soft RPM cap (TokenBucket-gated). |
| `TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS` | `0` (disabled) | When >0, abort if reasoning chars/4 crosses the cap. Must be < `MAX_TOKENS`. |
| `TRIUMVIRATE_DEEPSEEK_LOG_DIR` | `$HOME/.triumvirate/deepseek-logs/` | Per-request log destination. |
| `TRIUMVIRATE_DEEPSEEK_LOG_REASONING_CAP_BYTES` | `262144` (256KB) | UTF-8-safe truncation of `reasoning_content` in the log. |
| `TRIUMVIRATE_DEEPSEEK_BULK_BYTES` | `16384` (16KB) | Anti-bulk byte cap (REQ-DS-025). |

Invalid numeric values (e.g. `MAX_TOKENS=oops`, `TIMEOUT_SECS=0`) FAIL LOUD
with `ConfigError::InvalidEnv` at daemon startup — they do NOT silently fall
back to the default (Codex W1-review BLOCKER fix).

---

## 8. Probe battery — the verification invocation

The 8-probe live contract battery lives at
`daemon/crates/triumvirate/tests/deepseek_contract.rs`. Each probe is
`#[ignore]`-gated so it does NOT run on a default `cargo test` (no surprise
spending). Run it explicitly:

```sh
TRIUMVIRATE_DEEPSEEK_API_KEY=$YOUR_KEY \
  cargo test -p triumvirate --test deepseek_contract -- --ignored --nocapture
```

| Probe | What it grounds |
|---|---|
| `probe_01_balance_endpoint_shape` | `/user/balance` envelope still exists; account is funded |
| `probe_02_models_endpoint_returns_v4_pro_and_v4_flash` | Both models still served |
| `probe_03_streaming_emits_reasoning_then_content_then_usage_then_done` | SSE event order / usage shape |
| `probe_04_reasoning_tokens_already_in_completion_tokens` | No-double-add invariant (REQ-DS-009 / A-04b) |
| `probe_05_max_tokens_starvation_returns_finish_reason_length` | Bad-finish detection (REQ-DS-030) |
| `probe_06_bad_key_returns_401_authentication_error` | Auth-error envelope shape |
| `probe_07_malformed_request_returns_4xx_invalid_parameter` | Hard-class envelope shape |
| `probe_08_flash_non_thinking_no_reasoning_content` | `thinking=disabled` works |

Latest captured run: see `daemon/docs/v1-deepseek/PROBE_RESULTS.md` (Wave 0 +
T-016 Wave 5 sections).

**Run the probe battery:**
  - **Before** enabling DeepSeek in a new daemon environment for the first
    time.
  - **After** any change to `mcp-bridge/src/deepseek*.rs` that touches the
    wire surface.
  - **As part of** incident response if a sibling consult fails in an
    unexpected way.

---

## 9. Key rotation

The API key in the daemon's environment SHOULD be rotated periodically. The
key initially used for the Wave-0 / Wave-5 probe runs is in the goatrodeo
session transcript and **must be rotated** when convenient.

### Rotation procedure
1. Generate a new key in the DeepSeek console → API Keys → Create.
2. Update the daemon's environment:
   ```sh
   export TRIUMVIRATE_DEEPSEEK_API_KEY="sk-NEW-KEY"
   ```
3. Restart the daemon so the `OnceLock`-cached `DeepSeekConfig` re-loads
   the new key on first use.
4. **Smoke test** the new key by running the probe battery (above) — all
   8 probes should pass, with `probe_06_bad_key_returns_401_authentication_error`
   still using its own intentionally-bad key (so a successful PROBE-06 with
   401 confirms the rest of the probes used the NEW funded key correctly).
5. In the DeepSeek console, **revoke the old key**.

### Compromise response
If the key may have been exposed (in a screenshot, a logs paste, etc.):
1. **Revoke immediately** in the DeepSeek console — don't wait for
   rotation. The daemon will start failing with 401 (`HardProvider(401)`)
   until a new key is set.
2. Check `$LOG_DIR/*.json` for any consult activity in the relevant time
   window — those records carry the model output but NOT the key itself, so
   they're safe to inspect.
3. Set the new key per the procedure above.

---

## 10. Rollback

To fully disable DeepSeek dispatch without removing the integration:

```sh
unset TRIUMVIRATE_DEEPSEEK_API_KEY
# Restart the daemon to clear the OnceLock cache.
```

The daemon continues to recognise `agent="deepseek"` (returning a typed error
at the first request) and all Gemini/Codex paths remain untouched. To remove
the integration entirely, revert the merge of `spec/deepseek-integration-v1`
into main.

---

## 11. Quick reference

| Scenario | Action |
|---|---|
| First-time enable | Top up → set key → run probe battery → enable in clients |
| Balance drops below comfort | `GET /user/balance` → top up → no daemon restart needed |
| HTTP 402 received | Breaker latches HardOpen — top up + restart daemon |
| Sustained 503/429 in peak window | Breaker auto-cycles via cooldown — expected |
| `system_fingerprint` change in logs | DeepSeek backend rollover — re-run probe battery to re-ground contract |
| Disable temporarily | `unset TRIUMVIRATE_DEEPSEEK_API_KEY` + daemon restart |
| Lower bulk-payload cap | `export TRIUMVIRATE_DEEPSEEK_BULK_BYTES=8192` (or your value) |
| Key compromise | Revoke in console → set new key → smoke-test |
