# Round 2 — Interrogation (integration mechanics focus)

The R1 decisions are settled; R2 interrogates HOW to build against DeepSeek + the new REQs.
Tags: [RESEARCH] live search · [INTERNAL] code/architecture.

## Integration mechanics (the front-end research the pre-rodeo skipped)
- **R2-Q1 (REQ-DS-004/014) [RESEARCH]** — Rust client shape: hand-rolled `reqwest` + SSE vs a
  crate (`async-openai`)? Does `async-openai` (2026) support a custom `base_url`, DeepSeek's
  `reasoning_content`, and `stream_options.include_usage`? Maintenance/risk tradeoff.
- **R2-Q2 (REQ-DS-019) [RESEARCH]** — Exact DeepSeek streaming SSE chunk format:
  `delta.reasoning_content` then `delta.content` ordering, the `[DONE]` sentinel, and does the
  stream carry final `usage` (via `stream_options.include_usage`)? Are
  `prompt_cache_hit_tokens`/`miss` present in streamed usage?
- **R2-Q3 (REQ-DS-005/007) [RESEARCH]** — How are `thinking` / `reasoning_effort` passed over
  RAW HTTP JSON (not the SDK's `extra_body`)? Top-level fields? And does streaming emit
  SSE-comment keep-alives during the ~10-min queue (so the idle-reset timer in REQ-DS-007 works)?
- **R2-Q4 (REQ-DS-006) [RESEARCH]** — Error JSON shapes + codes for 402/429/400/401/5xx. Is
  `Retry-After` actually returned on 429? Is there an `error.code`/`error.type` taxonomy to
  classify hard (402/401/400) vs transient (429/5xx)?
- **R2-Q5 (REQ-DS-013) [RESEARCH]** — Required request headers beyond `Authorization: Bearer`
  (org/project/version headers)? Key format/prefix? Any deviation from OpenAI's header set?
- **R2-Q6 (REQ-DS-009) [RESEARCH]** — In STREAMED mode, where do cache-hit/miss tokens appear,
  and are reasoning tokens counted inside `completion_tokens` (confirming "fold into output")?
- **R2-Q7 (REQ-DS-017) [RESEARCH/INTERNAL]** — Minimal verification probe battery against the
  REAL endpoint: auth ok, model resolves, streaming parses, reasoning_content separated, usage
  present w/ cache fields, 402/429 reproducible. How do people contract-test OpenAI-compat?
- **R2-Q8 (REQ-DS-004/006) [RESEARCH]** — Rust concurrency vs the account-concurrency cap (500):
  semaphore + `reqwest` connection-pool sizing; how to configure reqwest idle/read timeouts to
  tolerate the 10-min keep-alive hold without leaking connections.

## New-REQ acceptance (for twins)
- **R2-Q9 (REQ-DS-020) [INTERNAL]** — synthetic `session_id` format; exact `require_reused_worker`
  handling so a stateless DeepSeek call isn't treated as a reuse failure.
- **R2-Q10 (REQ-DS-019) [INTERNAL]** — which existing `WorkingStateEvent` variants map to the
  lifecycle events (reuse vs add)? Don't invent a parallel event type if one exists.
- **R2-Q11 (REQ-DS-017) [INTERNAL]** — where does the probe battery live (a `cargo test`
  ignored-by-default integration test hitting the real API w/ the free grant? a script?)
  mirroring `research/antigravity/agy-verification/`.
