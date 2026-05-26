# Round 1 — Research answers (live Gemini search, May 2026)

All answers from live `gemini-search` (DeepSeek moves fast — re-verify at build). Citations in
session transcript.

## A — OpenAI compatibility (Q3) → access is largely a config boundary
- Base URL `https://api.deepseek.com/v1` (or `/beta` for FIM/prefix). Auth = `Authorization:
  Bearer <key>` (standard OpenAI-style).
- `POST /chat/completions` highly complete: `model`, `messages` (system/user/assistant/tool),
  `stream`, `max_tokens`, `tools`/`tool_choice`, `stop` all FULL. `response_format json_object`
  partial (needs "json" in prompt or 400). **Unsupported:** `n>1`, `logprobs`, native
  `json_schema` structured outputs. `temperature`/`presence_penalty` ignored in reasoning mode.
- **Implication:** cloud API, self-host (vLLM/ollama), and CLI-proxy all expose the SAME
  OpenAI-compatible contract. Access path ≈ `base_url`+auth config, deferrable behind one HTTP
  client — **Fork A/B partly collapse at the integration layer.**

## B — Reasoning trace format (Q7) → access path leaks into parsing
- **Cloud API:** clean separate field `choices[].message.reasoning_content` (and
  `delta.reasoning_content` when streaming). NO `<think>` tags. Clean separation.
- **Self-host (vLLM/ollama):** often emits `<think>…</think>` INSIDE `content` → requires regex
  parsing. So the consult-output parsing differs by access path.
- Multi-turn: must STRIP `reasoning_content` from history or 400. (Triumvirate consults are
  single-turn per the agy pattern → not a problem.)

## C — Models & roles (Q6) → naming changed; reasoning is a MODE
- **Current models: `deepseek-v4-pro` (reasoning flagship) + `deepseek-v4-flash` (fast).**
  Legacy `deepseek-chat`/`deepseek-reasoner` (and R1/V3.2 framing) **retire 2026-07-24.**
- Reasoning is a **mode/param** on v4-pro: `thinking:{type:enabled|disabled}` +
  `reasoning_effort:high|max` (low/medium auto-mapped to high). NOT a separate model.
- Context: v4-pro 1M, v4-flash 256K. Max output 384K. Cache automatic.
- **Implication:** Fork C "which model" → "v4-pro (thinking on) for reasoning seat" vs
  "v4-flash for cheap throughput." The diversity thesis → v4-pro reasoning.

## D — Rate limits / failure (Q8) → agy resilience partly maps
- DeepSeek caps on **account concurrency** (v4-pro **500**, v4-flash **2,500** concurrent), NOT
  static RPM/TPM. Dynamic "traffic-pressure" throttling.
- **429** carries `Retry-After`; **NO** granular `x-ratelimit-remaining` headers → client must
  self-manage concurrency (semaphore).
- **CRITICAL:** before a 429, DeepSeek may **hold the connection open up to ~10 min** with
  keep-alive (empty lines non-stream / SSE `: keep-alive` comments) while queuing. A naive
  short SIGKILL/timeout would kill LEGITIMATE queued requests.
- **`HTTP 402 Insufficient Balance`** when prepaid balance hits $0 (distinct hard failure).
- **Maps from agy_resilience:** concurrency-cap (DIRECT — DeepSeek's own limit is concurrency),
  breaker, rate-limit. **Does NOT map:** reset-window-cooldown (no daily quota reset; it's
  prepaid balance + dynamic throttle). **New needed:** 402 → hard breaker (not transient);
  timeout must tolerate keep-alive (reset on keep-alive signal, or generous ceiling).

## E — Token accounting (Q11) → "exact" needs cache tiers
- `usage.prompt_cache_hit_tokens` + `usage.prompt_cache_miss_tokens` reported separately.
- Cache hit ≈ 50× (flash) to 120× (pro) cheaper than miss. Flat costing OVER-bills hits.
- **`price_table`/`calculate_cost_usd` need tiered input pricing** (cache-hit vs miss) for true
  `usage_source=exact`. Without it, "exact" is wrong.

## F — Pricing (§5 verified; brief was stale)
| Model | In (miss) /M | In (hit) /M | Out /M | Context |
|-------|------|------|------|---------|
| v4-flash | $0.14 | $0.0028 | $0.28 | 256K |
| v4-pro | $0.435 | $0.003625 | $0.87 | 1M |
(v4-pro promo → permanent 2026-06-01.) Free grant: **5M tokens — UNVERIFIED** (not in official docs; Codex flagged; do NOT assume — the
Wave-0 probe checks real balance; see `research-verification.md`). Prepaid top-up model; no auto-recharge by default.

## G — Self-host feasibility (Q2 / Fork A) → quality-vs-sovereignty, NOT all-or-nothing
**CORRECTION (post-user-challenge, 2026-05-25):** an earlier draft of this section claimed
"v4-flash is non-reasoning" and concluded a reasoning seat is "cloud-only." **That was wrong**
and it contaminated the R1 twin review (both twins reasoned from the false premise). Live data:
- **v4-flash IS reasoning-capable** (released 2026-04-24, GRPO-trained, full thinking mode:
  `reasoning_effort` non-think/high/max, `reasoning_content` field). It trails v4-pro by only
  **1-3 pts** on reasoning benchmarks (GPQA 88.1 vs 90.1; MMLU-Pro 86.2 vs 87.5; SWE-bench 79.0
  vs 80.6; LiveCodeBench 89.4 vs 93.5).
- V4 = **MIT open weights**, HuggingFace, day-0 vLLM/SGLang/llama.cpp.
- **VRAM:** v4-flash ~175GB FP8 (2×H200 / 4×A100) or **~90-100GB INT4 (4×RTX4090 / 2×RTX6000Ada);
  Q4 GGUF on a 192GB Mac** — i.e. **prosumer / vulcan-1-class reachable.** v4-pro ~900GB-1TB FP8
  / ~500GB INT4 (8×A100) — **datacenter-only.**
- **Corrected conclusion:** there are TWO viable reasoning seats, a quality-vs-sovereignty fork —
  (a) **cloud v4-pro** = best reasoning, needs API key, metered; (b) **self-host v4-flash +
  thinking** = sovereign, ZERO API key (fits the user ethos), ~1-3pt quality hit, blocked on GPU
  hardware (vulcan-1). Because both speak OpenAI-compatible HTTP, access is a `base_url`+auth
  config — ship cloud now, swap to self-host later behind the same code (REQ-DS-002 abstraction
  boundary). Self-host parsing wrinkle remains (local may emit `<think>` tags in `content`).

## H — CLI tooling (Q4) → native HTTP wins for a Q&A seat
- No official first-party DeepSeek CLI. aider/OpenCode = heavy coding agents (Q&A modes exist:
  `aider --message`, `opencode -p`, `-f json`). Lighter: `deepseek-cli`, Simon Willison's
  `llm` (swap base_url), `sgpt`, or `curl|jq`. Because API is 100% OpenAI-compatible, **native
  HTTP is the lightest path** for a reasoning/Q&A seat — no heavyweight CLI dependency, no
  sandbox.

---

## Net effect on the forks (research-driven, CORRECTED post-challenge)
- **Settled by research/twins (auto-resolvable):** native HTTP returning `ParsedAgentResult`
  (no fork of `execute_ask_agent`); token-economics needs NO schema change (map miss→input,
  hit→cached); concurrency-cap+breaker+token-bucket map, reset-window cooldown does not; 402=hard
  trip / 429=transient; REQ-DS-007 → keep-alive-aware HTTP timeout (not SIGKILL); fail-loud, no
  silent substitution; explicit-route (no router exists); no sandbox; stateless single-turn; full
  first-class surface; `deepseek` = top-level name not a GeminiBackend.
- **Build against the OpenAI-compatible HTTP contract ONCE** — then access path (cloud vs
  self-host) is a `base_url`+auth config, deferrable. This dissolves Fork B (native HTTP) and
  makes Fork A a swappable runtime choice rather than a build-time fork.
- **The genuine user decisions that remain:**
  - **REQ-DS-003 + REQ-DS-002 (linked, the crux):** quality-vs-sovereignty. (a) cloud **v4-pro**
    = best reasoning, needs an **API key** (collides with "subscriptions only, never API keys"),
    metered spend; vs (b) self-host **v4-flash + thinking** = sovereign, **zero API key** (fits
    the ethos), ~1-3pt reasoning hit, **blocked on GPU hardware (vulcan-1)**; vs (c) **ship cloud
    v4-pro now behind the OpenAI-compat boundary, swap to self-host v4-flash when vulcan-1 lands.**
    Needs the user's actual *rationale* for the API-key rule (Q1) to choose.
  - **REQ-DS-005 (role):** reasoning specialist (the diversity thesis) vs general vs coding.
  - **REQ-DS-010 (spend cap):** only relevant on the cloud/metered path; prepaid+402 already
    hard-caps — add a client-side breaker too, or rely on prepaid+alerts?
