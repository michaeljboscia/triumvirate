# Round 2 — Research answers (integration mechanics, live Gemini search, May 2026)

## R2-Q1 — Rust client shape
- `async-openai` v0.38+ supports `with_base_url`, DeepSeek `reasoning_content` (v0.36+, or `byot`
  "bring your own type" feature), and `stream_options.include_usage`. Lower maintenance.
- Hand-rolled `reqwest` + SSE (`eventsource-stream`/`sse-rs`) gives full control over the byte
  stream, custom error classification, and the `read_timeout` keep-alive mechanism (below).
- **Tradeoff (→ D-06, surface):** `async-openai` = less code, tracks spec drift, but abstracts
  the low-level timeout/keep-alive + error-body handling we specifically need. Hand-rolled =
  ~50-line SSE parse + exact `read_timeout`/402/429 control + native lifecycle events. For THIS
  use case (REQ-DS-007 read_timeout, REQ-DS-006 error taxonomy, REQ-DS-019 lifecycle), hand-rolled
  reqwest is the better fit; async-openai's abstraction works against the control we need.

## R2-Q2 — Streaming SSE format (confirmed)
- Order: N× `delta.reasoning_content` chunks → N× `delta.content` chunks → (if
  `stream_options.include_usage`) one **empty-`choices`** chunk with `usage` → `data: [DONE]`.
- Usage: `prompt_tokens, completion_tokens, total_tokens, prompt_cache_hit_tokens,
  prompt_cache_miss_tokens` (prompt = hit + miss). Reasoning tokens counted in
  `completion_tokens` → fold into `output_tokens` (confirms A-04b).
- Parser MUST ignore lines starting with `:` (keep-alive comments) and the `[DONE]` sentinel
  (not JSON).

## R2-Q3 — thinking/reasoning over raw HTTP
- `thinking` (`{"type":"enabled"|"disabled"}`) and `reasoning_effort` (`high`|`max`; low/medium→high)
  are **top-level JSON body fields** (no SDK needed). When thinking enabled, API ignores
  `temperature`/`top_p`/`presence_penalty`/`frequency_penalty`.
- Keep-alive: streaming sends `: keep-alive` SSE comments while queuing (up to ~10 min);
  non-streaming sends empty `\n` lines. → streaming + read_timeout handles it (R2-Q8).

## R2-Q4 — Error taxonomy (for the breaker)
- Body: `{"error":{"message","type","code"}}` (OpenAI-style; `param` sometimes on 422).
- 400 `invalid_format` · 401 `invalid_api_key` · **402 `insufficient_balance`** · 422
  `invalid_parameter` · 429 `rate_limit_exceeded` · 500 `internal_error` · 503 `overloaded`.
- **429 returns `Retry-After`** (seconds). Missing → exp backoff 1s × 2 + jitter.
- **Classification:** HARD/no-retry = status < 429 (400/401/402/422) → 402 trips hard breaker
  (A-06). TRANSIENT = 429 (honor Retry-After) + ≥500 (backoff). Clean rule for REQ-DS-006.

## R2-Q5 — Headers (minimal)
- `Authorization: Bearer sk-…` (mandatory) + `Content-Type: application/json` (required); `Accept`
  recommended; `User-Agent` optional. **No** org/project/api-version headers (ignored if sent).
- Key format `sk-[alphanumeric]`. Versioning via URL (`/v1`, `/beta`), not a header.
- (DeepSeek also offers an Anthropic-style base `/anthropic` — NOT used; we use OpenAI `/v1`.)

## R2-Q6 — Cache + reasoning tokens (confirmed) → see R2-Q2. Fold reasoning into output_tokens.

## R2-Q7 / R2-Q11 — Verification probe battery (REQ-DS-017)
- Pattern: `daemon/.../tests/deepseek_contract.rs` with `#[ignore]` tests, env-gated
  (`TRIUMVIRATE_DEEPSEEK_API_KEY`), run `cargo test -- --ignored`. Use the free 5M-token grant.
- Probes: (1) auth ok (200 on a tiny request); (2) model resolves (`deepseek-v4-pro`); (3)
  streaming parses (reasoning_content then content, [DONE]); (4) reasoning separated from answer;
  (5) usage chunk present with cache-hit/miss fields; (6) 401 on bad key; (7) error-body shape on
  a malformed request (422). 402/429 are environmental — assert classification logic via unit
  tests with synthetic responses, not live.

## R2-Q8 — reqwest timeouts + concurrency (the REQ-DS-007 mechanism)
- **`read_timeout` (rolling, resets on each received byte) ≈ 60s** = idle detection; keep-alive
  comments are bytes → auto-reset. **`timeout` (hard ceiling) ≈ 900s** = absolute cap.
  **`tcp_keepalive` ≈ 30s** = kernel-level dead-socket detection. This IS REQ-DS-007's
  "idle-reset-by-keepalive + absolute ceiling + abort-on-drop" — native to reqwest.
- Concurrency cap: `tokio::sync::Semaphore` (permit held for request duration → backpressure);
  size to the account concurrency cap (v4-pro 500, but practically a small daemon-side cap, e.g.
  4-8, since these are interactive consults). Maps to A-05 concurrency-cap primitive.

## Net effect (R2)
- Almost all auto-resolve (mechanics facts that REDUCE build risk, change no decision).
- ONE new genuine decision **D-06: async-openai vs hand-rolled reqwest** — recommend hand-rolled
  reqwest for the control REQ-DS-006/007/019 require.
- Confirms/strengthens R1: REQ-DS-007 (read_timeout), REQ-DS-006 (error taxonomy + 402 hard),
  REQ-DS-009 (usage chunk + fold reasoning), REQ-DS-019 (streaming SSE parse), REQ-DS-013/015
  (minimal headers + sk- key + base_url), REQ-DS-017 (#[ignore] probe battery).
