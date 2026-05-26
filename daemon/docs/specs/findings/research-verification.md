# Research Verification — triangulation against primary sources (2026-05-25)

**Why:** the operator flagged that the R1/R2/R3 research rested on a SINGLE channel
(`gemini-search`) for a fast-moving, future-dated target — a confabulation risk. This pass
re-verifies the load-bearing facts against THREE independent sources:
- **G** = Gemini web search (R1/R2/R3 findings)
- **C** = Claude reading the OFFICIAL docs directly (WebFetch of api-docs.deepseek.com)
- **X** = Codex independent (its own knowledge + its own official-doc check)

## Result table
| Fact | G | C | X | Verdict |
|------|---|---|---|---------|
| Model IDs `deepseek-v4-pro` / `deepseek-v4-flash`; legacy `deepseek-chat`/`deepseek-reasoner` deprecated 2026-07-24 | ✓ | ✓ | ✓ | **CONFIRMED** |
| OpenAI-compatible; base `https://api.deepseek.com`; Bearer `sk-` | ✓ | ✓ | ✓ | **CONFIRMED** |
| `reasoning_content` field, sibling to `content`; resending it in input → 400 | ✓ | ✓ | ✓ | **CONFIRMED** |
| Thinking via `thinking:{type:enabled\|disabled}` (default enabled) + `reasoning_effort:high\|max` (low/med→high, xhigh→max, default high) | ✓ | ✓ (thinking_mode page) | ✓ | **CONFIRMED** — `thinking` is in `extra_body` (SDK) = a raw JSON body field for hand-rolled reqwest; `reasoning_effort` top-level |
| Pricing v4-pro $0.435 in-miss / $0.003625 in-hit / $0.87 out (75% promo → PERMANENT 2026-05-31 15:59 UTC) | ✓ | ✓ | ✓ | **CONFIRMED** (cache ~120×) |
| Pricing v4-flash $0.14 in-miss / $0.0028 in-hit / $0.28 out | ✓ | ✓ | ✓ | **CONFIRMED** (cache ~50×) |
| `max_tokens` includes the CoT (shared budget); reasoner default 32K / max 64K | ✓ | ✓ (reasoner page) | ✓ | **CONFIRMED** (v4 default not explicit on the page → set ours; lean ~32K to avoid starving the answer) |
| Concurrency limits 500 (pro) / 2500 (flash); 429 on exceed | ✓ | ✓ | ✓ | **CONFIRMED** |
| Keep-alive: empty lines (non-stream) / `: keep-alive` SSE comments (stream); server closes if no inference within 10 min | ✓ | ✓ | ✓ | **CONFIRMED** |
| Error codes 400/401/402-insufficient_balance/422/429/500/503 | ✓ | ✓ | ✓ | **CONFIRMED** |
| **5M free-token grant, 30-day expiry** | ✓ | — | ✗ (not in docs) | **UNVERIFIED — do NOT assume; probe the real balance/grant** |
| Outage history (7h blackout Mar 29-30; peak-hour 503s) | ✓ | — | — | **SINGLE-SOURCE — plausible, unverified; treat as operational caution not fact** |
| Mid-stream error inside HTTP-200 SSE ("ghost success") | ✓ (general SSE pattern) | — | — | **GENERIC pattern, NOT DeepSeek-confirmed → the Wave-0 probe must verify whether DeepSeek does this** |

## Corrections this pass made to MY OWN earlier claims (logged for honesty)
- I briefly claimed "Gemini fabricated the `thinking`/`reasoning_effort` params" — WRONG. My
  WebFetch had hit the legacy `deepseek-reasoner` page (no param); the v4 `thinking_mode` page
  confirms the params. Codex caught this. The params are REAL; D-07/REQ-DS-027 stand.
- I briefly claimed "pricing is 4× understated ($1.74/$3.48)" — WRONG. That was a misread of the
  standard column; the promo ($0.435/$0.87) is current AND becomes permanent 2026-05-31. Original
  numbers were right.
- Lesson: WebFetch's summarizer is noisy on tabular/multi-page docs; tri-source + cross-check is
  what corrected it. This is the multi-source discipline the operator asked for, working.

## Spec deltas from verification (fold at EDIT)
- REQ-DS-005/027: CONFIRM thinking enabled default + reasoning_effort high default; for hand-rolled
  reqwest, `thinking` and `reasoning_effort` are JSON body fields (no SDK extra_body wrapper).
- REQ-DS-009: CONFIRM price rows — pro 0.435/0.003625/0.87, flash 0.14/0.0028/0.28; note promo→permanent.
- REQ-DS-005/015: **`max_tokens` default = 32768** (operator-confirmed 2026-05-25) — the API's own
  reasoner default; shared with CoT; reduces finish_reason=length on hard problems; still bounded,
  frugality still rides the thinking-toggle.
- REQ-DS-007/024 (CONSEQUENCE of the 32K bump): a full 32K generation ≈ 25-35 min at ~15-20 tok/s,
  which would collide with the R2/R3 absolute ceiling of 600s. Resolution: the **idle/read-timeout
  (60s no-bytes) remains the PRIMARY failure detector** (a hung stream fails fast regardless of
  length); **raise the absolute ceiling default to ~1800s** (30 min, env-knob `_TIMEOUT_SECS`) so it
  doesn't guillotine a legitimately long, still-progressing deep-reasoning consult. DeepSeek's own
  10-min-no-inference close still bounds the queue case. Typical consults finish in 1-5 min; the
  ceiling is a rare backstop. (Avoids re-introducing the R2 DELTA-01 "ceiling kills legit reasoning"
  bug now that max_tokens is larger.)
- REQ-DS-017 (Wave-0 probe) MUST live-verify the three UNVERIFIED items: (a) actual free
  grant/balance, (b) whether DeepSeek emits mid-stream errors inside a 200 (ghost-success — REQ-DS-029),
  (c) real outage/latency behavior. Do NOT hard-code the 5M grant as a planning assumption.
- The architecture (first-class agent, native HTTP to OpenAI-compat, fail-loud, metered, streaming,
  thinking-toggle frugality) is VINDICATED by primary sources — no architectural change.

## LIVE PROBE (real api.deepseek.com, 2026-05-25, operator-supplied key, $0 — empty account)
Endpoints hit: `GET /user/balance`, `GET /models`, `POST /chat/completions` (402 + 401 paths).
The key was used via env var only, never persisted; operator advised to ROTATE (pasted in chat).

**Empirically CONFIRMED:**
- Auth contract works; `GET /user/balance` →
  `{"is_available":false,"balance_infos":[{"currency":"USD","total_balance":"0.00","granted_balance":"0.00","topped_up_balance":"0.00"}]}`.
- **NO free grant on this account** (`granted_balance:"0.00"`) — empirically kills the "5M grant"
  assumption. The account is at $0 and `is_available:false`.
- `GET /models` → exactly `deepseek-v4-flash`, `deepseek-v4-pro` (real, served; legacy ids not listed).
- 402 body: `{"error":{"message":"Insufficient Balance","type":"unknown_error","param":null,"code":"invalid_request_error"}}` (HTTP 402).
- 401 body: `{"error":{"message":"Authentication Fails...","type":"authentication_error","param":null,"code":"invalid_request_error"}}` (HTTP 401).
- A chat request carrying `thinking:{type:enabled}` + `reasoning_effort:high` was rejected for
  BALANCE (402), not as invalid params (would be 400/422) → weak support that those params are accepted.

**BUILD-CRITICAL CORRECTION (only live traffic revealed this):**
- The error-body **`code` is GENERIC** (`invalid_request_error` for BOTH 402 and 401) and **`type`
  is UNRELIABLE** (`unknown_error` for an insufficient-balance 402). → **Error classification MUST
  key on the HTTP STATUS CODE (402/401/429/5xx), NOT on `error.code`/`error.type`.** The body is
  advisory only (use `message` for display). This corrects R2-A06/R3 (which assumed an `error.code`
  taxonomy). Apply to REQ-DS-006 (breaker classification) and REQ-DS-029 (ghost-success: a
  stream-embedded `error` object must also be classified by any embedded HTTP-ish status or treated
  as a generic transient/hard per a small explicit map, since `code` won't disambiguate).

**OPERATIONAL PREREQUISITE (Wave 0 + runbook REQ-DS-017/018):**
- The DeepSeek account MUST be FUNDED (prepaid top-up) before ANY call succeeds — there is no
  auto-grant. Wave-0 chat/stream/usage probes are BLOCKED until top-up. Add to the runbook:
  fund the account; monitor `total_balance`; a $0 balance ⇒ 402 on every call ⇒ breaker hard-open.

## LIVE PROBE — FUNDED account ($10 top-up, 2026-05-25) — the remaining items
Probes A-D, total cost <1¢ (small max_tokens). Key via env only; ROTATE advised.

**A) v4-pro non-stream, thinking enabled, reasoning_effort high — CONFIRMED:**
- `reasoning_content` present (308 chars) and SEPARATE from `content`; finish_reason `stop`.
- `system_fingerprint` real: `fp_9954b31ca7_prod0820_fp8_kvcache_20260402`.
- **Exact usage schema:** `{prompt_tokens, completion_tokens, total_tokens,
  prompt_tokens_details:{cached_tokens}, completion_tokens_details:{reasoning_tokens},
  prompt_cache_hit_tokens, prompt_cache_miss_tokens}`. Verified: prompt = hit + miss (18=0+18);
  **completion_tokens (174) INCLUDES reasoning_tokens (120)**.
- **Usage mapping (corrects A-04b — do NOT double-add reasoning):**
  `output_tokens ← completion_tokens` (already incl. reasoning); `input_tokens ←
  prompt_cache_miss_tokens`; `cached_tokens ← prompt_cache_hit_tokens` (== prompt_tokens_details.cached_tokens).
  `reasoning_tokens` is available for observability/CoT-size, not a separate billable add.
**B) v4-pro streaming, include_usage — CONFIRMED:** SSE `data: {…chat.completion.chunk… delta:{content,
  reasoning_content}}`; reasoning phase (reasoning_content populated) → answer phase (content
  populated, reasoning_content:null); `data: [DONE]` sentinel present; system_fingerprint per chunk.
**C) C-1 SHARED-BUDGET STARVATION — EMPIRICALLY PROVEN:** v4-pro, thinking ON, max_tokens=64 →
  `finish_reason:length`, reasoning_tokens=64, **content="" (EMPTY)**. Reasoning ate the whole
  budget. ⇒ validates generous max_tokens=32K default + REQ-DS-030 (length→loud-fail). The original
  4096-frugality-lever WOULD have shipped empty answers.
**D) v4-flash thinking:disabled — CONFIRMED:** no reasoning_content, content="ok", finish_reason stop.
  The frugal non-think path works (the thinking-toggle frugality lever, REQ-DS-027).

**STILL doc-level only (not inducible without load — defensive handling already specced):**
- Keep-alive `: keep-alive` cadence + the 10-min connection close (0 observed on fast calls;
  doc-confirmed; REQ-DS-007/024 idle-timeout handling stands).
- Mid-stream error inside HTTP-200 ("ghost success", REQ-DS-029) — not inducible on demand; keep the
  defensive per-chunk `error` check + classify by any embedded status, body advisory only.

## EDIT-gate twin audit (2026-05-25, post-DL)
The canonical merged spec was verified by both twins (fresh sessions / inline content for Gemini):

**Codex (`deepseek-edit-codex` daemon, code-grounded):**
- F-01 (High) → 4 missing explicit REQ entries (002/003/004/010); substance was in decisions block
  but list-of-30 traceability was broken. **FIXED:** explicit cross-reference REQ entries added in
  §3.2 (002/003/004) and §3.6 (010). ✅
- CLEAN: max_tokens 4096→32768 fully landed; ceiling at 1800s; HTTP-status error classification;
  failure-class table; usage mapping (no double-add); provenance + Unknowns Register; REQ IDs
  019-030 all present, no duplicates; D-01..D-07 faithfully recorded.

**Gemini (via `gemini` MCP pro/high, coherence — daemon session path down; inline content):**
- C-01 (High) → REQ-DS-011's "extends /council" CONTRADICTED §7-deferred / D-03 / REQ-DS-022.
  **FIXED:** REQ-DS-011 now explicitly states council integration is a deferred skill-level
  follow-on, NOT v1 daemon scope. ✅
- M-01 (Medium) → REQ-DS-026's "execute_ask_agent persists before Err" is a shared-infra change
  with blast-radius risk to Gemini/Codex error paths. **FIXED:** REQ-DS-026 now gates the path
  to DeepSeek's typed-failure-with-usage; adds a regression-guard unit-test bullet. ✅
- L-01 (Low) → REQ-DS-015 `_MAX_CONCURRENT` / `_MAX_RPM` lacked defaults. **FIXED:** defaults 8 / 60
  added with rationale. ✅
- CLEAN: D-01..D-07 faithfulness; supersessions fully landed; failure-class/timeout/metering one
  coherent story.

**EDIT verdict: PASS** — both twins' findings resolved; canonical spec faithfully encodes the
approved deltas; ready for Phase 3.

## NET: foundation status = TRIPLE-SOURCED + LIVE-VERIFIED
Architecture + all load-bearing facts confirmed by Gemini-search + Claude-reads-official-docs +
Codex + LIVE api.deepseek.com. Empirical corrections folded: HTTP-status error classification (not
error.code), usage mapping (output←completion_tokens, no double-add), C-1 proven (max_tokens 32K +
ceiling 1800s), 5M grant is FALSE (account needs funding). Residual doc-level items (keep-alive,
ghost-success) have defensive handling. No architectural change across the whole verification.
