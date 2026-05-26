# Round 2 Deltas — 2026-05-25

Effective spec = candidate spec + round-1-deltas + this. Focus: integration mechanics +
two new user constraints (API-novelty, token frugality). Source spec merged at the EDIT gate.

## User Calls
| ID | Decision | Choice | REQs |
|----|----------|--------|------|
| D-06 | Rust client | **Hand-rolled `reqwest` + SSE** (both twins; `reqwest` already a workspace dep). NOT `async-openai` — this first-ever API integration needs low-level control over read_timeout, keep-alive, usage-chunk, error-body classification, and lifecycle events; an SDK hides exactly what we must observe. Reconsider an SDK only after the probe battery + thin client prove the contract. | REQ-DS-004 |
| D-07 | Frugality posture | **Thinking ON by default** (effort=high — low/medium auto-map to high, so OFF-vs-ON is the only real lever) WITH rails: max_tokens default 4096, CoT bifurcated (human-only, not into Claude's context), per-consult cost surfaced, and Claude can pass `thinking:off` per-call for quick/cheap consults. Diversity-first with frugality guardrails. | REQ-DS-005, REQ-DS-023 |

## Auto-Resolves
- R2-A01: **Hand-rolled reqwest** (D-06 consensus). → REQ-DS-004.
- R2-A02: **Probe battery = Wave 0** (both twins; constraint A). REQ-DS-017 runs against the real
  endpoint (free 5M grant) BEFORE the runner architecture is committed. → REQ-DS-017.
- R2-A03 **[Codex DS-R2-01 Critical + DELTA-05/D2-AUD-04, CORRECTED]**: **DeepSeek BYPASSES the
  daemon's generic 3-attempt retry loop** (`execute_ask_agent` ~373). Explicit acceptance:
  `attempt_schedule` for `agent=="deepseek"` is ONE outer attempt; the runner owns
  retry+classification internally. Classification: HTTP **4xx app errors (400/401/402/422) never
  retry** (402 → hard breaker trip); **429** honors `Retry-After` (internal, bounded); **5xx**
  bounded internal backoff. **Network nuance (DELTA-05):** a connection failure BEFORE the first
  byte is eligible for ONE internal retry; a MID-STREAM dirty disconnect FAILS LOUD (don't
  double-pay for partial deep reasoning) and records estimated metering (REQ-DS-026). → REQ-DS-006/014.
- R2-A04 **[Codex DS-R2-03 ⊗ Gemini COH-02, CORRECTED after delta audit DELTA-01/D2-AUD-06]**:
  **TWO distinct timers, not one.**
  (1) **Idle/read-timeout ~60s** (reqwest `read_timeout`, rolling, reset by ANY received byte incl.
  keep-alive comments) = the PRIMARY dead-stream detector → no bytes for 60s ⇒ fail loud.
  (2) **Absolute SLA ceiling = a GENEROUS backstop, default ~600s** (env-configurable; outer
  `tokio::time::timeout` around the DeepSeek attempt) — NOT 90s. thinking-ON + max_tokens 4096
  generates for ~3-4.5 min, so a 90s absolute ceiling would fail EVERY legitimate deep query
  (the defect the audit caught). The ceiling rarely fires; it's a backstop for pathological
  10-min-queue cases, and even then it's VISIBLE (progress events), not a silent block.
  Keep-alive comments MUST emit a low-noise `WorkingStateEvent` so the `StuckDetector` (60s idle /
  90s frozen, `agent-adapter/stuck.rs`) doesn't false-positive, and so the long-running consult is
  visible. Both timers fail LOUD. Supersedes R1 A-07's 900s AND the bad ~90s. → REQ-DS-007/024.
- R2-A05 **[Gemini COH-01 + DELTA-02 / Codex D2-AUD-01/03, CORRECTED]**: **CoT bifurcation, made
  OPTIONAL.** Default: the `ask_agent` response to Claude carries FINAL `content` only
  (frugality); `reasoning_content` is stored separately (storage owner NAMED — see REQ-DS-023:
  a size-bounded ledger/log record, NOT stuffed into `OutboxEvent.detail`). BUT Claude is the
  council synthesizer and sometimes NEEDS the reasoning — so a per-call `include_reasoning`
  (default false) lets Claude opt to receive the trace. Requires the per-call request surface
  (REQ-DS-027). → REQ-DS-023, REQ-DS-027.
- R2-A06 **[D-07 + COH-03]**: frugal request defaults — `thinking:{type:enabled}`, `reasoning_effort:high`
  (low/medium infeasible), `max_tokens` default **4096** (env knob), per-call `thinking`/`effort`
  override exposed to Claude. → REQ-DS-005.
- R2-A07 **[Codex DS-R2-04 + constraint B]**: **per-consult cost visibility is v1, not deferred** —
  after the usage chunk, synchronously compute cost from the price row and emit a lifecycle/progress
  cost line (cheap path; `AskAgentResponse` has no cost field today and persist writes
  `cost_usd:None`). Optionally add `token_usage`/`cost_usd` to `AskAgentResponse` (larger, optional).
  Only the hard cumulative spend-CAP stays deferred. → REQ-DS-021 (amended).
- R2-A08 **[Gemini COH-04/DELTA-04 + Codex D2-AUD-02, CORRECTED]**: **dirty-disconnect metering
  fallback.** If the stream ends before the usage chunk: estimate `estimated_tokens =
  bytes_received/4 + prompt_token_estimate` (prompt via local char/4) and record
  **`usage_source=estimated`**. **Persistence gap (Codex D2-AUD-02):** `persist_daemon_token_record`
  derives `usage_source` as `agy`→else `exact` (agent_exec.rs:167), so this requires a code path to
  mark DeepSeek records `estimated` — add `usage_source` to `ParsedAgentResult` OR a DeepSeek
  `parser_mode` convention. → REQ-DS-009 / REQ-DS-026 + Unknowns Register.
- R2-A09 **[Codex DS-R2-06]**: DeepSeek breaker states = `HardOpenInsufficientBalance` (402; manual/
  top-up reset, NO auto-half-open), `OpenTransient` (429/5xx; backoff), `HalfOpen` (recovery probe).
  Do NOT clone agy's quota-reset-window logic. → REQ-DS-006.
- R2-A10 **[Codex DS-R2-07]**: stateless — runner IGNORES inbound `session_id`, returns a synthetic
  accounting id, never resumes; two sequential calls share no state. Strengthens REQ-DS-020.
- R2-A11 **[Codex DS-R2-08]**: timeout error text distinguishes idle-timeout vs absolute-ceiling;
  drop the hard-coded "TIMEOUT after 60s" (`execute_ask_agent` ~718) for the DeepSeek path. → REQ-DS-007.
- R2-A12 **[Gemini COH-05/DELTA-03 + Codex D2-AUD-05, CORRECTED]**: anti-bulk = BOTH a **hard**
  daemon-side byte/char intercept on the DeepSeek `ask_agent` path (reject payload over a threshold,
  e.g. ~16KB, with a clear error — the real enforcement) AND **soft** guidance in the global
  `ask_agent` tool doc / Claude skill ("DeepSeek is remote+metered — logic/architecture/reasoning,
  not bulk/file-grep/logs"). Note: there is no per-agent tool-description surface today (one generic
  MCP tool desc, main.rs ~470) — the hard intercept is the enforceable owner. → REQ-DS-025.
- R2-A13: SSE parse details — ignore lines starting `:` (keep-alive) and the `[DONE]` sentinel;
  order = reasoning_content* → content* → empty-`choices` usage chunk; fold reasoning tokens into
  `output_tokens`. → REQ-DS-019/009.
- R2-A14: minimal headers — `Authorization: Bearer sk-…` + `Content-Type: application/json`; no
  org/project/api-version. Versioning via URL (`/v1`). → REQ-DS-013/015.
- R2-A15 **[Codex DS-R2-10]**: env knob names (add to R1's BASE_URL/API_KEY/MODEL):
  `TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS` (~60), `_TIMEOUT_SECS` (SLA ceiling ~90),
  `_TCP_KEEPALIVE_SECS` (~30), `_MAX_CONCURRENT`, `_MAX_RPM`, `_MAX_TOKENS` (4096),
  `_THINKING` (on), `_REASONING_EFFORT` (high). → REQ-DS-015.

## REQ Additions / Renames
- ADD **REQ-DS-023** — CoT bifurcation (optional). *Done when:* by default `ask_agent deepseek`
  `.response` contains ONLY final `content`; `reasoning_content` is stored to a NAMED, size-bounded
  target (ledger record or a per-request log file with retention — NOT `OutboxEvent.detail`, which
  would bloat the outbox) and is NOT in the response payload — UNLESS the call sets
  `include_reasoning=true` (REQ-DS-027), in which case the response includes the trace. *Test:*
  default call → `.response`==content + separate stored trace; `include_reasoning=true` → trace in response.
- ADD **REQ-DS-024** — orchestrator timeout, fail-loud. *Done when:* (a) idle/read-timeout
  (`TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS`, default 60) — no bytes for the window ⇒ loud error;
  (b) absolute ceiling (`TRIUMVIRATE_DEEPSEEK_TIMEOUT_SECS`, default **600**) via an OUTER
  `tokio::time::timeout` around the attempt ⇒ loud error; both produce explicit errors (no silent
  block, no sibling substitution) with text distinguishing idle-vs-ceiling; keep-alive emits a
  progress event resetting `last_event_at` so a legit streaming/queuing consult is NOT marked STUCK.
  *Test:* dead stream (no bytes) → idle error at ~60s; keep-alive-only under ceiling → not STUCK; past ceiling → ceiling error.
- ADD **REQ-DS-025** — anti-bulk. *Done when:* (a) HARD — the daemon DeepSeek path rejects an
  `ask_agent` payload over a configurable byte threshold (default ~16KB) with a clear error; (b)
  SOFT — the global `ask_agent` tool doc / Claude skill notes DeepSeek is remote+metered (logic/
  reasoning, not bulk/file-grep/logs). *Test:* an oversized payload to deepseek → rejected with a clear error.
- ADD **REQ-DS-026** — dirty-disconnect metering. *Done when:* a stream ending before the usage
  chunk records `usage_source=estimated` with `estimated = bytes_received/4 + prompt_estimate`,
  never silently 0; persistence is able to mark the record `estimated` (not forced to `exact` by the
  current agent_exec.rs:167 path). *Test:* truncated mock stream → estimated record, not zero, not exact.
- ADD **REQ-DS-027** — per-call override surface. *Done when:* `AskAgentRequest` gains OPTIONAL
  fields (`deepseek_thinking`, `deepseek_reasoning_effort`, `deepseek_include_reasoning`,
  `deepseek_max_tokens`) defaulting to None⇒env config; the MCP `ask_agent` tool schema exposes them
  so Claude can override per-call (e.g. `thinking:off` for a quick consult, `include_reasoning:true`
  for a deep one). Backward-compatible (optional, ignored by gemini/codex). *Test:* a call with
  `deepseek_thinking=off` sends `thinking:{type:disabled}`; absent ⇒ env default (on).
- AMEND **REQ-DS-004** — hand-rolled reqwest (D-06).
- AMEND **REQ-DS-005** — frugal defaults (thinking on/effort high/max_tokens 4096) + per-call override.
- AMEND **REQ-DS-006** — DeepSeek-owned retry/classification (bypass generic loop); breaker states
  HardOpenInsufficientBalance/OpenTransient/HalfOpen.
- REWRITE **REQ-DS-007** — read_timeout(60s rolling) + SLA ceiling(~90s) + tcp_keepalive(30s) +
  keep-alive→progress event + idle-vs-ceiling error text. (Supersedes R1 A-07's 900s.)
- AMEND **REQ-DS-009** — SSE usage-chunk mapping (hit→cached, miss→input, reasoning→output);
  dirty-disconnect → estimated (REQ-DS-026).
- AMEND **REQ-DS-017** — Wave 0, against the real endpoint (free grant).
- AMEND **REQ-DS-019** — streaming; keep-alive event resets StuckDetector; CoT to log not response.
- AMEND **REQ-DS-021** — per-consult cost VISIBILITY is v1 (lifecycle/progress line); only the hard
  cumulative cap is deferred.
- No deletions. New IDs 023-027 verified unused vs 001-022.

## Unknowns Register (constraint A — first-ever API integration)
| U | Unknown | Resolved by |
|---|---------|-------------|
| U-1 | Exact SSE chunk ordering/shape in thinking mode | Wave-0 live probe (streaming parse) |
| U-2 | Keep-alive comment cadence/format during a real queue | Wave-0 probe (may need induced load) |
| U-3 | Final usage chunk presence + cache-hit/miss populated | Wave-0 probe |
| U-4 | Error body shapes (401/422 live; 402/429 synthetic) | Wave-0 probe + unit tests |
| U-5 | Dirty mid-stream disconnect → partial/no usage | Defensive: REQ-DS-026 estimated fallback |
| U-6 | events_tx / StuckDetector behavior with a streaming HTTP agent | daemon integration test |
| U-7 | Cancellation (dropped future aborts conn) vs the CLI kill path | daemon integration test |
| U-8 | reqwest read_timeout interaction with SSE keep-alive bytes | Wave-0 probe |
| U-9 | DeepSeek concurrency cap vs daemon's existing worker model | integration test |

## Net State After Round 2
- REQ count: **27** (was 22; +5: 023/024/025/026/027 — 027 added during the R2.8 delta audit).
- R2 auto-resolves: 15 (R2-A01…A15). R2 user decisions: 2 (D-06, D-07).
- Critical defects caught this round:
  1. Generic 3-attempt retry loop would wrongly retry DeepSeek hard errors (Codex, Critical).
  2. CoT would pollute Claude's context + violate frugality if piped raw (Gemini, Critical).
  3. 10-min keep-alive vs fail-loud tension → SLA-ceiling synthesis (both twins).
  4. StuckDetector false-positive on legit keep-alive queue (Codex).
  5. Dirty disconnect → silent 0-token metering (Gemini).
- Two-twin value this round: Codex = code-path conflicts (retry loop, StuckDetector, cost field,
  breaker states); Gemini = coherence (CoT bifurcation, fail-loud-vs-block, frugality posture,
  estimated metering, anti-bulk guidance). Fully orthogonal.

## Twin Audit (R2.8 — both twins; CRITICAL caught + fixed)
**Auditors:** `deepseek-delta2-codex` (daemon, structural/code-grounded) + Gemini (via `gemini` MCP
pro/high — daemon session path still down; MCP fallback). Independent.

**CRITICAL (found by BOTH) — FIXED:** the R2-A04 timeout was incoherent — a 90s *absolute* ceiling
vs thinking-ON + max_tokens 4096 (~3-4.5 min generation) would fail every legitimate deep query.
Corrected to two distinct timers: idle/read-timeout 60s (primary, keep-alive-reset) + generous
absolute ceiling default 600s (outer `tokio::time::timeout`, configurable). → R2-A04, REQ-DS-007/024.

**Codex (structural) — 8 findings, resolved:**
- D2-AUD-01 (High) no per-call request surface → ADD REQ-DS-027 (optional AskAgentRequest fields). ✅
- D2-AUD-02 (High) estimated metering can't be marked (persist forces exact, agent_exec.rs:167) →
  R2-A08/REQ-DS-026 require a usage_source path on ParsedAgentResult. ✅
- D2-AUD-03 (High) reasoning_content storage has no owner (OutboxEvent has no blob field) →
  REQ-DS-023 names a size-bounded ledger/log target, not OutboxEvent.detail. ✅
- D2-AUD-04 (Med) attempt_schedule must be 1 for deepseek → R2-A03 explicit + test. ✅
- D2-AUD-05 (Med) no per-agent tool-desc surface → REQ-DS-025 hard byte intercept is the enforceable owner. ✅
- D2-AUD-06 (Med) SLA owner = outer tokio::timeout; read_timeout = reqwest setting → REQ-DS-024. ✅
- D2-AUD-07 (Low) StuckDetector is 60s idle / 90s frozen → R2-A04 corrected wording. ✅
- D2-AUD-08 (Low) cost-line owner = synchronous lifecycle/progress from price table → R2-A07. ✅
- Codex CLEAN: all file/line citations verified (retry ~373, StuckDetector, cost_usd None ~187,
  session writeback ~591, hardcoded 60s ~718); REQ-ID hygiene; probe-first DAG valid.

**Gemini (coherence) — 5 findings, resolved:**
- DELTA-01 (Critical) SLA-vs-thinking — same as the shared CRITICAL above. ✅
- DELTA-02 (High) CoT bifurcation undercuts Claude-as-synthesizer → made OPTIONAL via
  include_reasoning (REQ-DS-023 + 027). ✅
- DELTA-03 (Med) anti-bulk needs hard + soft → REQ-DS-025. ✅
- DELTA-04 (Med) estimated formula → REQ-DS-026 (bytes/4 + prompt estimate). ✅
- DELTA-05 (Low) retry nuance: pre-first-byte conn drop = 1 retry; mid-stream = fail loud → R2-A03. ✅

**Verdict: R2.8 PASS** — the shared CRITICAL and all High/Med findings resolved in this revised
delta. The two-twin rule earned its keep again: both independently caught the SLA defect from
different angles (Codex: timer ownership; Gemini: token-generation math).
