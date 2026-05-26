# DeepSeek → Triumvirate Integration — CANONICAL SPEC

> **Status:** REVIEWED & APPROVED via `/goatrodeo` 2026-05-25. Three rounds, 14 live searches,
> 3-source verification (Gemini + Claude reading official docs + Codex) **plus a live
> api.deepseek.com probe** ($10 funded). All REQ-IDs settled. Delta ledgers preserved as audit trail
> under `findings/`. Reference: agy-integration-spec.md (the "add a sibling" worked example).

---

## 0. Goal
Add **DeepSeek** as a first-class 4th Triumvirate sibling — consultable by Claude through the
existing `ask_agent {agent, message}` MCP tool, alongside `gemini` and `codex`. The first
non-Anthropic/Google/OpenAI voice; the first **API** (not CLI-subprocess) sibling in the daemon.

## 1. Strategic thesis (ratified)
DeepSeek is a genuinely different model family (different vendor, MoE architecture, MIT
open-weights). It should be a **first-class agent** (`gemini`/`codex`/**`deepseek`**), NOT a
hidden backend behind another agent. The diversity value comes from real adversarial dissent.

## 2. Ratified decisions
- **D-01 Access:** Cloud **`deepseek-v4-pro`** via API key — a documented EXCEPTION to the
  "subscriptions only, never API keys" rule (rationale: no GPU to self-host v4-pro
  [datacenter-only ~500GB VRAM]; DeepSeek has no subscription product so OAuth is impossible;
  metered cost is low enough that the rule's cost-avoidance rationale doesn't bite). Self-host
  was declined (no hardware). Memory updated with the exception.
- **D-02 Shape:** Native Rust HTTP (hand-rolled `reqwest` + SSE) against the OpenAI-compatible
  endpoint. NOT `async-openai` (this first API integration needs low-level control of timeouts,
  keep-alive, usage chunks, error classification — the SDK hides exactly what we must observe).
- **D-03 Role:** **True 4th participant** — consultable via `ask_agent` on ANY topic (reasoning,
  general, code review), exactly like `gemini`/`codex`. No sandbox in v1 (consult/Q&A, not
  file-writing). A file-writing `dispatch_deepseek` and `/council` skill integration are deferred.
- **D-04 Model is a config knob:** v4-pro for v1; `TRIUMVIRATE_DEEPSEEK_MODEL` makes **Pro↔Flash a
  one-line tuning swap** when usage data exists.
- **D-05 Spend:** Prepaid balance + HTTP 402 hard breaker-trip + **exact metered cost surfaced per
  consult**. Client-side cumulative spend-cap deferred (cheap pricing + prepaid = low runaway).
- **D-06 Client:** hand-rolled reqwest (subsumed by D-02; recorded for traceability).
- **D-07 Frugality posture:** **thinking ON by default** (effort=high — low/medium auto-map to
  high). Frugality lever = pre-flight `thinking:on/off` toggle Claude controls per task.
  Generous `max_tokens` (32K default — must be large because it's a shared reasoning+answer
  budget; empirically proven a tight cap returns empty answers). Runaway protection is a
  separate, default-disabled circuit breaker.

---

## 3. Requirements (30)

### 3.1 Identity & first-class surface
- **REQ-DS-001** — `deepseek` is exposed as a first-class agent (peer to `gemini`/`codex`),
  consultable via the existing `ask_agent {agent,message}` MCP tool. No new MCP tool.
- **REQ-DS-013** — `is_supported_agent_name` (`mcp-bridge/src/lib.rs:37`) extended to accept
  `"deepseek"`.
- **REQ-DS-016** — Full first-class surface: `display_agent_name` (mcp-tools), prewarm slot,
  daemon `/status` `supported_agents` (main.rs), session-spawn error text (main.rs),
  `execute_ask_agent` error text (agent_exec.rs), MCP env in `~/.claude.json`. `deepseek` is a
  **new top-level agent name**, NOT a `GeminiBackend`-style enum.
- **REQ-DS-022** — "True 4th participant." v1: consultable via `ask_agent` on any topic with
  done-when test (`ask_agent deepseek` succeeds for reasoning, general, code-review prompts).
  Deferred: file-writing `dispatch_deepseek`, `/council` skill integration.

### 3.2 Wiring (the dispatch path)
- **REQ-DS-002** *(resolved via D-01)* — Access path = **cloud `api.deepseek.com/v1`** (the
  OpenAI-compatible endpoint). Self-host declined (no GPU; v4-pro is datacenter-only). The
  OpenAI-compat boundary is preserved so a future config swap to a local endpoint remains
  trivial; self-host is NOT a roadmap commitment.
- **REQ-DS-003** *(resolved via D-01)* — The "subscriptions only, never API keys" rule has a
  **documented DeepSeek EXCEPTION** (DeepSeek has no subscription product; cost cheap enough that
  the rule's cost-avoidance rationale doesn't bite). Memory updated. Guardrails: prepaid balance
  + HTTP 402 hard breaker-trip + exact metered ledger; API key in `_API_KEY` env, never logged,
  never in argv, Bearer only.
- **REQ-DS-004** *(resolved via D-02/D-06)* — Integration shape = **hand-rolled `reqwest` + SSE**
  (native Rust HTTP), NOT `async-openai`. Runner returns `ParsedAgentResult` (REQ-DS-014).
- **REQ-DS-014** — Add a `"deepseek"` arm in `run_named_agent_with_session_and_model`
  (agent_exec.rs:~1216) returning `ParsedAgentResult`. `execute_ask_agent` is NOT forked.
  `run_agent_process_with_session` is UNCHANGED (it's subprocess-shaped) — unless a deferred CLI
  backend is built.
- **REQ-DS-015** — Connector = native HTTP env knobs (no `BIN`/`ARGS`):
  - `TRIUMVIRATE_DEEPSEEK_BASE_URL` (default `https://api.deepseek.com/v1`)
  - `TRIUMVIRATE_DEEPSEEK_API_KEY` (NEVER logged, NEVER in argv, used only as `Authorization: Bearer`)
  - `TRIUMVIRATE_DEEPSEEK_MODEL` (default `deepseek-v4-pro`)
  - `TRIUMVIRATE_DEEPSEEK_MAX_TOKENS` (default **32768**; shared reasoning+answer budget)
  - `TRIUMVIRATE_DEEPSEEK_THINKING` (default `enabled`)
  - `TRIUMVIRATE_DEEPSEEK_REASONING_EFFORT` (default `high`; `xhigh`→`max`)
  - `TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS` (default **60**)
  - `TRIUMVIRATE_DEEPSEEK_TIMEOUT_SECS` (default **1800**, absolute outer ceiling)
  - `TRIUMVIRATE_DEEPSEEK_TCP_KEEPALIVE_SECS` (default 30)
  - `TRIUMVIRATE_DEEPSEEK_MAX_CONCURRENT` (default **8** — small headroom for an interactive
    daemon; DeepSeek's own account cap is 500 pro / 2500 flash but a single operator rarely needs
    that much), `_MAX_RPM` (default **60** — conservative; raise if probes show no throttling)
  - `TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS` (default **0=disabled**; if set, validated `< _MAX_TOKENS`)
  - **Deferred:** cumulative hard spend-cap envs. The candidate spec's `deepseek_command()` +
    `_BIN`/`_ARGS` + `resolve_connector_command` arm are DEFERRED to a CLI backend.

### 3.3 Models, thinking, & frugality
- **REQ-DS-005** — Model selection via env (config knob). v4-pro = default (reasoning flagship,
  1M context, ~$0.435 in-miss / $0.87 out / $0.003625 in-hit per M after the 75%-off promo
  becomes permanent 2026-05-31 15:59 UTC). v4-flash = the fast/cheap alt ($0.14 / $0.28 / $0.0028)
  — same code path, swap via `_MODEL`. Reasoning is enabled by `thinking:{type:enabled|disabled}`
  + `reasoning_effort:high|max` (request-body JSON fields for raw HTTP; `low`/`medium`→`high`,
  `xhigh`→`max`). v1 default: thinking ENABLED, effort `high`. `max_tokens` is GENEROUS
  capacity (32K), NOT a frugality throttle — frugality is the **pre-flight thinking toggle**
  (REQ-DS-027). Empirical: max_tokens=64 with thinking ON returns `finish_reason:length` +
  empty content (the shared-budget starvation we designed against).
- **REQ-DS-027** — Per-call override surface. `AskAgentRequest` gains OPTIONAL fields
  (`deepseek_thinking`, `deepseek_reasoning_effort`, `deepseek_include_reasoning`,
  `deepseek_max_tokens`) defaulting to None ⇒ env defaults. MCP `ask_agent` tool schema exposes
  them so Claude can override per-call (e.g. `deepseek_thinking:disabled` for a quick consult).
  Backward-compatible (optional; gemini/codex ignore). The **`thinking` toggle is THE frugality
  lever**.
- **REQ-DS-028** — Optional runaway-reasoning early-abort. DEFAULT DISABLED
  (`_REASONING_CAP_TOKENS=0`); when set >0, validated `< _MAX_TOKENS`; SSE loop tracks an
  estimated reasoning-token count and crossing the cap aborts with a LOCAL error + estimated
  metering, **provider breaker NOT tripped**. Default runaway bound is max_tokens itself +
  `finish_reason:length` (REQ-DS-030).

### 3.4 Streaming, lifecycle, & CoT bifurcation
- **REQ-DS-019** — Streaming SSE with low-noise lifecycle events. Done when: the runner consumes a
  streamed response, parses `data: {…chat.completion.chunk…}` chunks with `delta.{content,
  reasoning_content}`, recognizes `data: [DONE]`, and emits ordered events `request_started`,
  `first_reasoning`, `first_answer_token`, `usage_parsed`, `completion`/`error`. Keep-alive
  comments (`: keep-alive`) reset `last_event_at` via a THROTTLED heartbeat (at most one event per
  30s, always below the 60s/90s `StuckDetector` thresholds; agent-adapter/stuck.rs). NO per-token
  `MessageDelta` events.
- **REQ-DS-023** — CoT bifurcation, OPTIONAL. Default: `ask_agent` `.response` carries final
  `content` ONLY (frugality; otherwise Claude pays input-context cost for the CoT);
  `reasoning_content` is stored to a NAMED size-bounded target (a DeepSeek per-request log/ledger
  record — NOT `OutboxEvent.detail`). When the call sets `include_reasoning=true` (REQ-DS-027), the
  trace is included in the response. Stripped CoT may be logged at debug for incident audit.

### 3.5 Resilience, timeouts, & error classification
- **REQ-DS-006** — Resilience module (`mcp-bridge/src/deepseek_resilience.rs`, NOT a generic-copy
  of `agy_resilience`): concurrency cap (`tokio::sync::Semaphore`; DeepSeek's own account-level
  cap is v4-pro **500** / v4-flash **2500**), token bucket, breaker. **Reset-window cooldown does
  NOT map** (no quota reset — prepaid balance + dynamic 429 pressure). Breaker states:
  `HardOpenInsufficientBalance` (402 — manual/top-up reset, no auto-half-open), `OpenTransient`
  (429/5xx/503/idle-timeout — backoff + cooldown), `HalfOpen` (recovery probe). **DeepSeek runner
  OWNS retry & classification**; `attempt_schedule` for `agent=="deepseek"` in `execute_ask_agent`
  is ONE outer attempt (bypassing the generic 3-attempt loop, agent_exec.rs:~373); HTTP-4xx app
  errors NEVER retry; 402 hard-trip; 429 honors `Retry-After`; **pre-first-byte connection failure
  = 1 internal retry; mid-stream dirty disconnect = fail loud** (don't double-pay for thinking).
  **Classification keys on HTTP STATUS, NOT on `error.code`** (empirically `error.code` is generic
  `invalid_request_error` for both 402 and 401; `error.type` is unreliable — `unknown_error` for a
  402). Body is advisory only.
- **REQ-DS-007** — HTTP cancellation-safe timeout policy.
  (a) **Idle/read-timeout 60s** (rolling, reset by any received byte incl. keep-alive comments) —
  PRIMARY dead-stream detector → loud error.
  (b) **Absolute SLA ceiling 1800s** (env-knob; outer `tokio::time::timeout` around the attempt)
  — a generous backstop so deep-reasoning consults (32K tokens × ~15-20 tok/s) aren't guillotined.
  (c) `tcp_keepalive` 30s. Dropping the future aborts the connection. SIGKILL is meaningless for
  native HTTP. Error messages distinguish idle-vs-ceiling-vs-runaway-vs-provider; no hardcoded "60s".
- **REQ-DS-024** — Orchestrator fail-loud timeouts. Done when: a consult exceeding either timer
  aborts with an explicit loud error (no silent block, no sibling substitution); keep-alive emits a
  progress event so a legit sub-ceiling stream is NOT marked STUCK. Test: dead stream → idle error;
  past ceiling → ceiling error.
- **REQ-DS-029** — Stream-embedded error ("ghost success"). Per-chunk check for top-level `error`
  key in any `data:` JSON; if present, normalize into the SAME error-handling path as HTTP errors
  (classify by any embedded status / a small explicit map, since `error.code` won't disambiguate).
  Same breaker policy, different DETECTION path.
- **REQ-DS-030** — `finish_reason` loud-failure. `finish_reason ∈ {length, content_filter,
  null/unknown}` → runner returns a hard error to Claude (NOT `Ok(parsed)`, no partial gibberish);
  the consumed tokens ARE metered (DeepSeek charges); provider breaker NOT tripped (budget/policy/
  anomaly, not provider-down).
- **REQ-DS-008** — Fail-loud default; **no silent sibling substitution.** Confirmed by DeepSeek's
  documented unreliability (peak-hour 503s, the March 7h blackout — silently substituting Gemini
  when the user asked for DeepSeek would be a systemic lie). A future degraded route MUST set
  `answered_by_agent`/`answered_by_backend`/`degradation_reason` in `AskAgentResponse`.

### 3.6 Token economics & metering
- **REQ-DS-009** — Exact metering via the existing `token-economics` schema (NO schema change —
  `cached_per_mtok` storage.rs:45 + `cached_tokens` lib.rs:43 already price input/output/cached
  separately, keyed by exact `model` string; attribution.rs:62). Empirically-verified mapping:
  - `output_tokens ← completion_tokens` (already INCLUDES `completion_tokens_details.reasoning_tokens` — **do NOT double-add**)
  - `input_tokens ← prompt_cache_miss_tokens`
  - `cached_tokens ← prompt_cache_hit_tokens` (== `prompt_tokens_details.cached_tokens`)
  - Add `price_table` rows: `deepseek-v4-pro` (in-miss $0.435, in-hit $0.003625, out $0.87) and
    `deepseek-v4-flash` ($0.14 / $0.0028 / $0.28). Promo→permanent 2026-05-31 15:59 UTC.
  Per-consult cost computed synchronously from the price table after the usage chunk.
- **REQ-DS-010** *(resolved via D-05)* — Spend mechanism = **prepaid balance + HTTP 402 hard
  breaker-trip + exact metered cost in the ledger** (+ low-balance alerts via runbook). A
  client-side cumulative spend-CAP is DEFERRED (low runaway risk: prepaid + 402 + cheap pricing).
  If/when added later, see REQ-DS-021's synchronous-cost-calc constraint.
- **REQ-DS-021** — Per-consult cost VISIBILITY is v1: after the usage chunk, emit a lifecycle/
  progress cost line (cheap path — `AskAgentResponse` extension is optional). Hard cumulative
  spend-CAP is DEFERRED; a future cap must compute cost synchronously (NOT from
  `persist_daemon_token_record`'s `cost_usd:None`, agent_exec.rs:187).
- **REQ-DS-026** — Metering keys on USAGE-CHUNK-RECEIVED, not failure class: exact if the final
  usage chunk arrived (even on a finish_reason failure or 200-with-content), else estimated
  (`bytes_received/4 + prompt_estimate`). ALL paid paths meter (dirty disconnect, reasoning-cap
  abort, ghost-success, bad finish_reason, ceiling/idle timeout). Err-path persistence owner: the
  runner returns a TYPED failure carrying usage; `execute_ask_agent` persists the token record
  BEFORE returning `Err` (today persistence is `Ok(parsed)`-only ~581). **Blast-radius safeguard:**
  the persist-before-Err path is **gated to typed failures that carry a populated usage record**
  (DeepSeek's failure type only) — Gemini/Codex/Claude Err paths remain unchanged and unaffected,
  with no collateral persistence and no new state mutation. Unit test: a non-deepseek `Err` from
  `execute_ask_agent` writes zero token records (regression guard).

### 3.7 Statelessness, routing, sandbox
- **REQ-DS-011** — Explicit-route only (no auto-router exists today). **Council integration is a
  SKILL-level FOLLOW-ON (Claude-side, deferred — not v1 daemon scope)**; the daemon does NOT wire
  a 4-way council in v1.
- **REQ-DS-012** — No sandbox in v1 (`ask_agent` is consult/Q&A, no file writes).
- **REQ-DS-020** — v1 is stateless single-turn. Worker state stores one `session_id` per
  agent::cwd, but DeepSeek HTTP has no honest resume → return a synthetic accounting id
  (documented format, e.g. `deepseek-<uuid>`); `require_reused_worker` does not treat the missing
  remote session as a failure; **prewarm = safe NO-OP** for DeepSeek (reconciles "implement the
  prewarm interface" with stateless-no-prewarm semantics).

### 3.8 Anti-bulk guidance
- **REQ-DS-025** — Anti-bulk = HARD + soft. (a) Daemon DeepSeek path rejects an `ask_agent`
  payload over a configurable byte threshold (default ~16KB) with a clear error — the enforceable
  guard. (b) Soft note in the global `ask_agent` tool doc / Claude skill: DeepSeek is remote +
  metered → logic/reasoning, not bulk/file-grep/logs. (There's no per-agent tool-description
  surface today; main.rs ~470.)

### 3.9 Verification & operations
- **REQ-DS-017** — Verification probe battery = **Wave 0** of the build, against the real
  endpoint. Lives at `daemon/.../tests/deepseek_contract.rs` with `#[ignore]`, env-gated
  (`TRIUMVIRATE_DEEPSEEK_API_KEY`), run via `cargo test -- --ignored`. Prerequisite: **funded
  account** (no auto-grant; $0 balance ⇒ 402 on every call). Probes: auth ok, model resolves
  (`deepseek-v4-pro` / `deepseek-v4-flash`), streaming parses, reasoning_content separated, usage
  fields incl. cache hit/miss present, 401 on bad key, 422 error-body shape, finish_reason=length
  reproduced. (402/429 environmental — assert classification via unit tests w/ synthetic responses.)
  Residual doc-level items to live-verify under load: keep-alive cadence + 10-min close,
  ghost-success-in-200 (REQ-DS-029).
- **REQ-DS-018** — Operator runbook (mirror `agy-operator-runbook.md`). Includes:
  - **Account must be funded** (prepaid; $0 ⇒ 402 ⇒ breaker hard-open). Monitor `total_balance`
    via `GET /user/balance`.
  - Expected peak-hour 503s (02:00-14:00 UTC worst); 503 = `OpenTransient` w/ threshold+cooldown.
  - `system_fingerprint` (e.g. `fp_…_prod0820_fp8_kvcache_…`) captured to a **DeepSeek per-request
    log record** (NOT `cli_version`, which is the token model key) — detect backend/quantization
    shifts under load.
  - Data-egress note: consult content goes to DeepSeek's servers (China-routed). Inherent to the
    API-key path (ratified, D-01), documented for awareness.

---

## 4. Failure-class → breaker behavior (the table)
| Class | Trips provider breaker? | Meters? |
|-------|------|------|
| 402 (incl. ghost-success `Insufficient Balance`) | YES — `HardOpenInsufficientBalance` | yes |
| 429 / 5xx / 503 (incl. embedded) | YES — `OpenTransient` (threshold+cooldown) | yes if usage rcvd |
| Idle/read-timeout (60s, no bytes) | YES — `OpenTransient` (dead-stream) | estimated |
| Absolute ceiling (1800s) | NO — local orchestrator abort, fail loud | estimated |
| Runaway reasoning cap (if enabled) | NO — local abort, fail loud | estimated |
| Bad `finish_reason` (length/content_filter/null) | NO — loud fail (budget/policy/anomaly) | exact if usage rcvd else estimated |

---

## 5. Unknowns Register (live items handed to Wave 0)
| U | Unknown | Resolved by |
|---|---------|-------------|
| U-1 | Keep-alive comment cadence under real queue + 10-min close behavior | Wave 0 probe under induced load |
| U-2 | Whether DeepSeek emits a mid-stream error inside HTTP-200 (ghost success) | Wave 0 probe under failure injection (else defensive handling stands) |
| U-3 | Real free-grant/balance on production accounts | Wave 0 (`GET /user/balance`); 5M-grant CONFIRMED ABSENT on this account |
| U-4 | Real outage/latency behavior under sustained workload | Operational; runbook |

---

## 6. Verification provenance
Foundation triple-sourced + live-verified: Gemini web search · Claude reading official docs at
`api-docs.deepseek.com` directly · Codex independent + its own doc check · **LIVE
`api.deepseek.com` probes ($10 funded)**. Empirical confirmations: model IDs (v4-pro/flash) served;
`reasoning_content` field; `thinking`/`reasoning_effort` params accepted; exact usage schema
(`prompt_cache_miss_tokens` / `prompt_cache_hit_tokens` / `completion_tokens` incl
`reasoning_tokens`); SSE chunk shape + `[DONE]`; **C-1 shared-budget starvation reproduced**
(max_tokens=64 thinking-ON → finish_reason=length + empty content); 402/401 shapes & generic
`error.code`. Provenance file: `findings/research-verification.md`. Round-by-round audit trail
under `findings/round-{1,2,3}-{interrogation,research,deltas}.md`.

## 7. Out of scope / deferred
File-writing `dispatch_deepseek` (would need a sandbox); `/council` skill 4-way integration
(Claude-side, not daemon); cumulative hard spend-cap (REQ-DS-021 — visibility is v1, hard cap is
later); CLI-subprocess backend (`_BIN`/`_ARGS` shape stays unimplemented until a real need); self-
host of v4-pro (datacenter VRAM); v4-flash self-host (no GPU available); auto-routing among
siblings (no router exists).
