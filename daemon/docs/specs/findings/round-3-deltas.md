# Round 3 Deltas — 2026-05-25

Triggered by the operator's question "do we understand the CHALLENGES?" → challenge-class research
(`round-3-research-challenges.md`) → this delta. Effective spec = candidate + R1 + R2 + this.

## User-intent corrections (no new user decision; these FIX prior auto-resolves)
| ID | Correction | REQs |
|----|-----------|------|
| X-01 | **`max_tokens` is a SHARED reasoning+answer budget.** R2's `max_tokens=4096`-as-frugality-lever would return EMPTY answers (reasoning eats the budget). REVERSED: **max_tokens generous, default 16384**, = capacity not throttle. | REQ-DS-005, REQ-DS-015 |
| X-02 | **Frugality model, FINAL:** the frugality lever is **pre-flight `thinking:on/off`** (REQ-DS-027), thinking ON by default (D-07). The in-flight reasoning cap is NOT a frugality lever (you can't tell a runaway loop from a hard 15k-token problem — Gemini COH-01); it's a HIGH catastrophic-loop circuit breaker only. | REQ-DS-005, REQ-DS-027, REQ-DS-028 |

## Auto-Resolves
- R3-A01 **[Codex DS-R3-01]**: R3 explicitly SUPERSEDES R2-A06 / REQ-DS-015's `max_tokens=4096`.
  Default `TRIUMVIRATE_DEEPSEEK_MAX_TOKENS=16384`. → REQ-DS-005/015.
- R3-A02 **[Gemini COH-01/F-01 + Codex DS-R3-01/02, CORRECTED]**: **`max_tokens` is the natural
  catastrophic-loop bound; the in-flight reasoning cap is an OPTIONAL early-abort, DEFAULT DISABLED.**
  Math fix (both auditors): a 32k cap can't fire when max_tokens=16k (the shared budget ends the
  stream at 16k via `finish_reason:length` first). So: the PRIMARY runaway bound is **max_tokens
  (16384) + `finish_reason:length` → loud fail + metering (REQ-DS-030)** — a loop burns at most 16k
  output (~$0.014, cheap). The separate in-flight reasoning cap
  (`TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS`) is **DEFAULT 0/disabled**; when set it MUST be
  `< max_tokens` (validated) and aborts early (local error + estimated metering, NOT a provider
  breaker trip) — useful mainly when an operator raises max_tokens high for deep problems. No dead
  code, no paradox. → REQ-DS-028.
- R3-A03 **[Codex DS-R3-03 + Gemini, C-3]**: **"ghost success" — stream-embedded error detection.**
  The SSE parser checks EVERY chunk for a top-level `error` key; if present, normalize it into the
  SAME DeepSeek error enum used for HTTP failures (`insufficient_balance`→402 hard-open,
  `rate_limit_exceeded`→429, `overloaded`/`internal_error`→5xx-transient) and feed the SAME breaker
  policy. A separate DETECTION path, NOT a separate policy. → REQ-DS-029 (new).
- R3-A04 **[Codex DS-R3-04 + Gemini COH-03, C-4]**: **`finish_reason` = strict LOUD failure.**
  `length` (truncated), `content_filter`, and null/unknown → the runner does NOT return
  `Ok(parsed)`; it returns a hard error to Claude ("DeepSeek: response truncated / filtered / api
  error") — never pass partial gibberish to the orchestrator. (Consistent with fail-loud; both
  twins.) → REQ-DS-030 (new).
- R3-A05 **[Codex DS-R3-05/D3-AUD-07, C-5]**: **low-noise events** — emit lifecycle events only
  (request_started, first_reasoning, first_answer_token, usage_parsed, completion, error); keep-alive
  emits AT MOST ONE heartbeat per configured interval (default **30s**, always below the 90s frozen
  threshold) to reset `last_event_at` without a firehose; NO per-token `MessageDelta` for DeepSeek. → REQ-DS-019.
- R3-A06 **[Gemini COH-02]**: **CoT strip default reinforced** — the generous max_tokens (X-01) makes
  this load-bearing: a 16k-reasoning response returned raw would cost Claude ~18k INPUT tokens
  ("context poisoning"). Default = strip reasoning from the payload to Claude; human/log keeps it;
  `include_reasoning=true` (REQ-DS-027) opts in. Confirms R2-A05/REQ-DS-023. → REQ-DS-023.
- R3-A07 **[Codex DS-R3-06/D3-AUD-02/03 + Gemini F-02/F-03, CORRECTED]**: **metering keys on
  usage-chunk-RECEIVED, not failure class.** Any PAID path (incl. finish_reason=length,
  ghost-success, runaway abort, dirty disconnect) meters the consumed tokens — DeepSeek charges
  regardless. If the final usage chunk arrived → `usage_source=exact`; if not → `estimated`
  (bytes/4 + prompt estimate). **Err-path persistence owner (named):** the DeepSeek runner returns a
  TYPED failure carrying the partial/estimated usage, and `execute_ask_agent` persists that token
  record BEFORE returning `Err` (today persistence is `Ok(parsed)`-only ~581). Extends REQ-DS-026.
- R3-A08 **[Codex DS-R3-07/D3-AUD-06, C-7]**: **`system_fingerprint` → ONE named destination: a
  DeepSeek per-request log record** (NOT `cli_version`, which stays the token model key; NOT a new
  token-schema column for v1). Captured for detecting under-load backend/quantization shifts. → REQ-DS-018.
- R3-A09 **[Gemini COH-04 + C-6]**: **unreliability CEMENTS fail-loud** — DeepSeek's documented
  outages/peak-503s REINFORCE no-silent-substitution (substitution would be "a systemic lie"). A
  503/timeout surfaces directly to Claude as a tool error; Claude explicitly decides to ask another
  sibling or inform the user. Confirms REQ-DS-008/024 (no change, recorded as ratified-under-pressure).
- R3-A10 **[Codex DS-R3-08/D3-AUD-04/05, EXPANDED]**: **failure-class → breaker behavior table**
  (the four classes, made unambiguous):
  | Class | Trips provider breaker? | Meters? |
  |-------|------|------|
  | Provider error 402 (incl. ghost-success `insufficient_balance`) | YES — HARD open | yes |
  | Provider error 429 / 5xx / 503 (incl. embedded) | YES — OpenTransient (threshold+cooldown) | yes if usage rcvd |
  | **Idle/read-timeout (60s, no bytes)** | YES — OpenTransient (network/dead-stream) | estimated |
  | **Absolute ceiling (600s)** | NO — local orchestrator abort, fail loud | estimated |
  | **Runaway reasoning cap (if enabled)** | NO — local abort, fail loud | estimated |
  | **Bad finish_reason (length/content_filter/null)** | NO — loud fail (budget/policy/anomaly, not provider down) | exact if usage rcvd else estimated |
  Generous max_tokens (X-01) raises pressure on the ceiling; finish_reason=length is the natural
  cost backstop. → REQ-DS-007/024/030.
- R3-A11 **[Codex DS-R3-09, C-6]**: runbook/config defaults for 503 handling — 503 = TRANSIENT
  breaker (not hard-open); operator-set threshold (e.g. N 503s → OpenTransient) + cooldown; note
  peak hours (02:00-14:00 UTC worst). → REQ-DS-018.
- R3-A12 **[C-7]**: **data-egress note** — consult content is sent to DeepSeek's servers
  (China-routed); inherent to the API-key path (already accepted, D-01) but documented in the
  runbook for operator awareness. → REQ-DS-018.

## REQ Additions / Renames
- ADD **REQ-DS-028** — optional runaway-reasoning early-abort. *Done when:* DEFAULT DISABLED
  (`TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS=0`); when set >0 it MUST validate `< max_tokens`; the
  SSE loop tracks an estimated reasoning-token count and crossing the cap aborts with a LOCAL error +
  estimated metering, provider breaker NOT tripped. The DEFAULT runaway bound is max_tokens itself +
  finish_reason:length (REQ-DS-030). *Test:* cap=12k & max_tokens=16k & stream >12k reasoning →
  local abort + estimated record, breaker closed; cap=0 → no early abort (finish_reason:length governs).
- ADD **REQ-DS-029** — stream-embedded error ("ghost success"). *Done when:* an HTTP-200 stream
  containing `data:{"error":{...}}` is detected per-chunk and classified via the SAME error enum/
  breaker as the HTTP-status path. *Test:* a 200 stream with an embedded `insufficient_balance`
  error → hard breaker trip (like 402), not a success.
- ADD **REQ-DS-030** — finish_reason loud-failure. *Done when:* `finish_reason ∈ {length,
  content_filter, null/unknown}` → the runner returns a hard error (not `Ok(parsed)`, no partial
  text to Claude); the consumed tokens are STILL metered (exact if usage chunk arrived, else
  estimated — DeepSeek charges for a length-failure); it does NOT trip the provider breaker
  (budget/policy/anomaly, not provider-down). *Test:* stream ending `finish_reason:"length"` with a
  usage chunk → loud error + EXACT metered record + breaker closed.
- AMEND **REQ-DS-005** — max_tokens default 16384 (capacity, not throttle); frugality = pre-flight
  thinking on/off; runaway protection = REQ-DS-028.
- AMEND **REQ-DS-015** — env: `_MAX_TOKENS` default 16384 (supersedes R2's 4096), ADD
  `_REASONING_CAP_TOKENS` (default **0 = disabled**; if set, validated `< _MAX_TOKENS`).
- AMEND **REQ-DS-019** — low-noise lifecycle events + throttled keep-alive heartbeat; no per-token.
- AMEND **REQ-DS-023** — CoT strip is the DEFAULT (context-poisoning guard under generous max_tokens).
- AMEND **REQ-DS-026** — metering keys on USAGE-CHUNK-RECEIVED, not failure class: exact if the
  final usage chunk arrived (even on a finish_reason failure), else estimated (bytes/4 + prompt
  estimate). ALL paid paths meter (dirty disconnect, reasoning-cap abort, ghost-success, bad
  finish_reason, ceiling/idle timeout). Err-path persistence owner: runner returns typed failure
  carrying usage; `execute_ask_agent` persists before returning `Err`.
- AMEND **REQ-DS-018** — runbook adds: peak-hour 503 expectation + 503-transient breaker defaults,
  `system_fingerprint` observability, data-egress (China-routed) note.
- AMEND **REQ-DS-027** — the `thinking` per-call toggle is explicitly THE frugality lever (Claude
  sets thinking:off for trivial tasks); thinking ON default.
- No deletions. New IDs 028-030 verified unused vs 001-027.

## Net State After Round 3
- REQ count: **30** (was 27; +3: 028/029/030).
- R3 corrections: 2 (X-01/X-02). R3 auto-resolves: 12 (R3-A01…A12). R3 user decisions: 0 (the
  challenge research corrected mechanics; no new fork — D-07's intent preserved, its max_tokens
  mechanism fixed).
- Critical defects caught this round:
  1. max_tokens-as-frugality-lever → empty answers (shared budget). REVERSED.
  2. in-flight reasoning cap as frugality lever → would abort legit deep reasoning (Gemini). Reframed
     as a high catastrophic-loop breaker only; frugality moved to pre-flight thinking toggle.
  3. "ghost success" — HTTP-200 stream-embedded errors bypass HTTP-status classification.
  4. finish_reason=length silently passing truncated answers (both twins → loud failure).
- Both twins declared the spec READY after this round (Gemini: "no further rounds necessary";
  Codex: remaining items are build-detail, spec-resolvable now).

## Twin Audit (R3.8 — both twins; shared BLOCKER caught + fixed)
**Auditors:** `deepseek-delta3-codex` (daemon, code-grounded) + Gemini (via `gemini` MCP pro/high —
daemon session path down; MCP fallback). Independent.

**BLOCKER (found by BOTH) — FIXED:** the 16k max_tokens vs 32k reasoning-cap paradox — a 32k cap
can never fire when the shared budget ends at 16k (`finish_reason:length` first) ⇒ dead code.
Corrected (R3-A02/REQ-DS-028): max_tokens IS the default catastrophic-loop bound (+ finish_reason:
length backstop); the reasoning cap is DEFAULT-DISABLED and, if set, MUST be `< max_tokens`. ✅

**Codex (structural) — 7 findings, resolved:**
- D3-AUD-01 (Critical) the cap paradox — see BLOCKER. ✅
- D3-AUD-02 (High) metering must key on usage-received, not failure class → REQ-DS-026/A07. ✅
- D3-AUD-03 (High) Err-path persistence owner named (runner→typed failure→execute_ask_agent persists pre-Err). ✅
- D3-AUD-04 (Med) finish_reason failures don't trip the provider breaker → REQ-DS-030 + A10 table. ✅
- D3-AUD-05 (Med) idle/read-timeout = OpenTransient; absolute ceiling = local abort → A10 table. ✅
- D3-AUD-06 (Med) system_fingerprint → ONE destination (DeepSeek per-request log) → A08. ✅
- D3-AUD-07 (Low) keep-alive throttle precise + typo fixed → A05/REQ-DS-019. ✅
- Codex CLEAN: all code citations verified (persistence Ok-only ~581, events→outbox ~456, no
  finish_reason/metadata on ParsedAgentResult, cli_version=model key); REQ-ID hygiene 028-030.

**Gemini (coherence) — 4 findings, resolved:**
- F-01 (Blocker) the 16k/32k paradox — see BLOCKER. ✅
- F-02 (High) metering must still fire on finish_reason=length (DeepSeek charges) → REQ-DS-026/030. ✅
- F-03 (Med) ghost-success must meter tokens received before the embedded error → REQ-DS-026/029. ✅
- F-04 (Note) log stripped CoT at debug/trace for incident audit → folded into REQ-DS-018/023. ✅
- Gemini VERDICT: frugality model coherent + faithful; four failure classes now cleanly distinct;
  "no Round 4 — the spec is genuinely DONE once F-01 is fixed." (It is.)

**Verdict: R3.8 PASS** — the shared BLOCKER and all High/Med findings resolved. Both twins
independently confirmed the spec is READY to proceed to the Decision Ledger. Two-twin value again:
both caught the math paradox from different angles (Codex: env-default contradiction; Gemini: token-
budget arithmetic).
