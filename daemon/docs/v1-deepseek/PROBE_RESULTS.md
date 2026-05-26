# PROBE_RESULTS — Wave 0 live API contract verification

> **Task:** T-000 (REQ-DS-017). Run the live contract probe battery against the real
> `api.deepseek.com` to ground-truth the contract assumptions in
> `daemon/docs/specs/deepseek-integration-spec.md` BEFORE any production wiring is enabled.
>
> **Date:** 2026-05-25
> **Spec branch:** `spec/deepseek-integration-v1`
> **WAVE0_SHA:** `9ead5f88872d1bcb8d7bc408b58316f842189618` (this commit's parent + the test file)
> **Account:** funded ($10 top-up earlier in the goatrodeo session); the API key was sourced
> from `TRIUMVIRATE_DEEPSEEK_API_KEY` env only — never written to source or logs.
> **Reproduce:** `cargo test -p triumvirate --test deepseek_contract -- --ignored --nocapture --test-threads=1`

## Verdict
**8 / 8 PASSED.** Every load-bearing contract claim in the spec is now empirically grounded
against the live API. No spec changes required.

```
test probe_01_balance_endpoint_shape ............................ ok
test probe_02_models_endpoint_returns_v4_pro_and_v4_flash ....... ok
test probe_03_streaming_emits_reasoning_then_content_then_usage_then_done ... ok
test probe_04_reasoning_tokens_already_in_completion_tokens ..... ok
test probe_05_max_tokens_starvation_returns_finish_reason_length  ok
test probe_06_bad_key_returns_401_authentication_error .......... ok
test probe_07_malformed_request_returns_4xx_invalid_parameter ... ok
test probe_08_flash_non_thinking_no_reasoning_content ........... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.20s
```

## Per-probe results

### PROBE-01 — `/user/balance` contract (REQ-DS-018)
- Result: `is_available=true total_balance=9.99`
- Confirms: endpoint at API root (not /v1); returns `is_available` + `balance_infos[]` with
  `currency`/`total_balance`/`granted_balance`/`topped_up_balance` — exactly the schema the
  runbook (REQ-DS-018) tells operators to monitor.

### PROBE-02 — `/models` lists v4-pro + v4-flash (REQ-DS-005)
- Result: `models served = ["deepseek-v4-flash", "deepseek-v4-pro"]`
- Confirms: both v1 model IDs (per REQ-DS-005 / the spec's pricing rows) are served. Legacy
  `deepseek-chat` / `deepseek-reasoner` are no longer advertised by `/models` (matches the
  retirement on 2026-07-24 per the docs).

### PROBE-03 — Streaming SSE: reasoning → content → usage → `[DONE]` (REQ-DS-019 / REQ-DS-009 / REQ-DS-023)
- Result: `reasoning_chars=131 content_chars=93 keepalive_lines=0`
- Usage chunk (verbatim):
  ```json
  {
    "completion_tokens": 97,
    "completion_tokens_details": { "reasoning_tokens": 53 },
    "prompt_cache_hit_tokens": 0,
    "prompt_cache_miss_tokens": 17,
    "prompt_tokens": 17,
    "prompt_tokens_details": { "cached_tokens": 0 },
    "total_tokens": 114
  }
  ```
- Confirms: SSE chunks carry `delta.reasoning_content` then `delta.content`; the
  `[DONE]` sentinel terminates the stream; the usage chunk arrives before `[DONE]` (note: in
  this run no `: keep-alive` comments observed — the prompt completed fast so the API didn't
  queue; the keep-alive mechanism is doc-confirmed but only triggers under load — still in the
  Unknowns Register U-1).
- Implementation note: the probe was initially over-strict (expected an *empty-choices* chunk
  with usage); relaxed to "usage in any chunk" because DeepSeek emits usage flexibly. The
  build runner should mirror this (T-006: look for `usage` in any data chunk, don't gate on
  `choices == []`).

### PROBE-04 — `completion_tokens` INCLUDES `reasoning_tokens` (REQ-DS-009 / A-04b — no double-add)
- Result: `completion=147 reasoning=96 (incl) prompt=16 (hit=0 miss=16)`
- Confirms: `completion_tokens` (147) **includes** `completion_tokens_details.reasoning_tokens`
  (96). The runner's mapping at T-009 MUST be `output_tokens ← completion_tokens` directly —
  adding reasoning_tokens to output would overstate cost by ~65% for this call.
- Sanity: `prompt_tokens (16) == cache_hit (0) + cache_miss (16)` — the equality the spec
  documents.

### PROBE-05 — Shared-budget starvation (C-1, REQ-DS-005 / REQ-DS-030)
- Result: `finish_reason=length content.len()=0 reasoning.len()=260`
- Confirms (CRITICAL): with `max_tokens: 64` and `thinking: enabled`, reasoning consumed the
  entire budget (260 chars of reasoning, ZERO content), and `finish_reason: "length"` fired.
  This is the empirical proof that **`max_tokens` is NOT a frugality lever** — a tight cap
  produces empty answers. Validates the spec's generous default (32K) and REQ-DS-030's
  loud-failure handling.

### PROBE-06 — Bad key → 401 with `type=authentication_error` (REQ-DS-006)
- Result: `401 + error.type=authentication_error`
- Confirms: HTTP 401 is the reliable signal for auth failure. `error.type` IS meaningful
  here (`authentication_error`), but per the spec's verification work, **classification
  should still key on HTTP STATUS** because `error.code` is generic
  (`invalid_request_error` for both 401 and 402 — observed in the verification phase).

### PROBE-07 — Malformed request → 400 (REQ-DS-006)
- Trigger: `response_format: {"type": "json_object"}` without "json" in the prompt — a
  documented 4xx path.
- Result: `HTTP 400`
- Body:
  ```json
  {"error":{
    "message":"Prompt must contain the word 'json' in some form to use 'response_format' of type 'json_object'.",
    "type":"invalid_request_error",
    "param":null,
    "code":"invalid_request_error"
  }}
  ```
- Confirms: the spec's "classify-by-HTTP-status" rule. Note again that `error.code` =
  `"invalid_request_error"` here (the SAME generic value as 401, 402, and 422 — empirically
  unreliable for disambiguation).

### PROBE-08 — Non-thinking mode works on flash (REQ-DS-027 frugality lever)
- Request: v4-flash + `thinking: {"type":"disabled"}`
- Result: `content="ok" reasoning=''`
- Confirms: the per-call `thinking:off` toggle (REQ-DS-027) produces NO `reasoning_content`
  and returns a clean content-only response — the operator's chosen frugality lever works.

## Residual unknowns (still doc-level only — Unknowns Register U-1, U-2)
- **U-1 keep-alive cadence under real queue (10-min hold):** not inducible on a fast
  small-prompt probe. Doc-confirmed by DeepSeek; the runner's defensive handling stands
  (reqwest's `read_timeout` rolling-reset is the primary detector — verified by T-005's
  90-second slow-drip mock test, not by this live probe).
- **U-2 mid-stream error inside HTTP-200 ("ghost success"):** not inducible on demand. The
  runner's defensive per-chunk error detection (REQ-DS-029) is in the build plan and will be
  unit-tested with a synthetic stream.

## Cost
~$0.01 across 8 probes (consolidated final run). The account balance went from $9.99 to
roughly $9.98 across this verification.

## Operator notes
- The API key used for these probes is in this session's transcript — **rotate it** when
  convenient. The next operator who runs these probes should set a fresh key in
  `TRIUMVIRATE_DEEPSEEK_API_KEY`.
- The probes are `#[ignore]`-gated, so they do NOT run on a default `cargo test`. They run
  ONLY via the explicit `cargo test -p triumvirate --test deepseek_contract -- --ignored`
  invocation, which is the right behavior for CI (no surprise spending).
