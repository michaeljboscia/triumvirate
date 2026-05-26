# DeepSeek Test Plan + Prompting Playbook

**Audience:** Operator about to run real DeepSeek traffic through the daemon.
**Status:** Live. Tracks gotchas verified against DeepSeek's own docs + Codex
sanity check + multi-source web research. Gemini deep-research is still
running (research-id `v1_ChZqbVVWYXRzVnZxV2EyUV9INmRqUUN3…`) and will be folded
in on completion.

**What we ALREADY ship and won't repeat here:** SSE keep-alive (`:`) handling,
chunk-boundary safety, finish_reason guards, ghost-success detection,
402-latches-HardOpen, 429 Retry-After single retry, absolute SLA timeout,
mid-stream disconnect → typed `NetworkMidStream` + estimated usage,
no-double-add usage mapping, per-request JSON log w/ `system_fingerprint`,
anti-bulk 16KB cap, CoT bifurcation, stateless single-turn synthetic
session IDs. The test plan focuses on what we DON'T already test.

---

## Section A — How the API actually behaves (canonical facts that don't show up in mock tests)

These are documented behaviors that ARE NOT exercised by the 8 contract
probes. Worth knowing before shipping traffic.

### A.1 Concurrency caps are STATIC, not dynamic
DeepSeek's official rate-limit doc says: `deepseek-v4-pro` allows 500
concurrent requests, `deepseek-v4-flash` allows 2,500 — calculated at the
**account level** regardless of which key. A request occupies a slot from
send-time until the response is fully complete. Bursting past this returns
HTTP 429 with no Retry-After header documented.[^rate-limit]

**Implication for our daemon:** our `max_concurrent` default of 8 is two
orders of magnitude below the 500/2500 account cap. We won't hit account
limits from a single instance. We MIGHT hit them if many parallel daemons
ever share a key — note for the multi-host story.

### A.2 The 10-minute pre-inference close
The docs explicitly say: "If the request has not started inference after 10
minutes, the server will close the connection." During the wait, the server
emits SSE `: keep-alive` comments (streaming) or empty lines (non-streaming).[^rate-limit]

**What we handle:** the keep-alive comments (T-006).
**What we DON'T explicitly test:** a 10-minute keep-alive trail followed by
EOF with zero data deltas. This is `StreamEndedWithoutDone` in our taxonomy
— Codex flagged it as a distinct class (`ProviderQueueTimeout`) worth its
own typed failure for incident triage.

### A.3 Off-peak discount window — 16:30–00:30 UTC daily
Confirmed for V3/R1 at 50%/75% off respectively, official DeepSeek
announcement.[^offpeak][^scmp] **V4-Pro/Flash off-peak is NOT yet officially
confirmed** but multiple sources expect the same window. Worth scheduling
any batch/non-latency-sensitive work in that window.

**Today:** 2026-05-26. V4-Pro is on a separate 75% promo until **2026-05-31
15:59 UTC**, after which pricing adjusts to 1/4 of the original. Our seeded
prices ($0.435 input / $0.87 output) are the post-discount rates we
currently see; they will likely DROP further when the promo ends and the
permanent 1/4 schedule kicks in.[^pricing]

### A.4 Thinking-mode silent parameter drops
`temperature`, `top_p`, `presence_penalty`, `frequency_penalty` are
**accepted but silently ignored** when thinking is enabled. No error. No
warning.[^thinking]

**We don't expose these knobs to callers today.** If we ever do, the
runner should reject (or warn) when thinking=enabled + any sampling param
is set — otherwise users will believe they're changing model behavior when
they aren't.

### A.5 Thinking defaults to ENABLED on v4-*
If you omit the `thinking` field entirely, V4 turns thinking on. Our
config sets `ThinkingMode::Enabled` as the default, so this matches —
but it means our `flash` consults are also paying reasoning-token costs
unless the caller explicitly sets `deepseek_thinking: "disabled"`.[^thinking]

### A.6 Cache prefix matching is BYTE-LEVEL + 64-token chunks
DeepSeek's KV-cache doc: matches in 64-token prefix units; a single
byte changing in the prefix fully invalidates the cache. Persistence is
"hours to days" — no SLA.[^kv-cache]

**Implication:** anything we put in the system prompt (or message head) MUST
be stable. A timestamp / request-id / uuid in the prompt head = full cache
miss every call. Cache hit price ($0.003625/M for v4-pro) is **120× cheaper**
than miss price ($0.435/M). Get this wrong and the bill is 120× higher.

### A.7 `reasoning_content` round-trip rule (single-turn safe; multi-turn trap)
For non-tool multi-turn: replayed `reasoning_content` is silently IGNORED
(false sense of preserved state).
For tool-using multi-turn: omitting `reasoning_content` from prior
assistant messages → **HTTP 400**.[^thinking]

**Our v1 is stateless single-turn — neither case applies today.** When we
eventually add tool-calling: this is the #1 trap. We need a path that
preserves `reasoning_content` in the conversation history.

---

## Section B — Test scenarios, ordered cheap → expensive → destructive

Each scenario lists what to set up, what the right behavior looks like,
and what the failure signature would look like.

### TIER 1 — Free / local-only (no API spend)

**B.1 — Thinking-mode silent-param drop is documented in the consult log**
Set `temperature: 2.0` in the request body alongside thinking enabled.
DeepSeek will accept and ignore. Expected: our per-request log should
either omit sampling params (since we don't expose them) OR our runner
should log a `tracing::warn!` when thinking=enabled + sampling=set.
**Failure tell:** a caller bug report claiming "I set temperature=0 but
results vary." Without a warn, we can't repro.

**B.2 — Default thinking-mode is on**
Send a v4-flash request with `thinking` field omitted from our payload.
Expected: response carries `reasoning_content` (thinking enabled by default).
**Failure tell:** silent reasoning-token billing on what callers think is a
"fast cheap flash query." Mitigation: when `cfg.model` is flash, default
`cfg.thinking` to `Disabled` unless caller opts in. Currently we don't —
we treat them symmetrically.

**B.3 — Cache-miss invariant: `hit + miss == prompt_tokens`**
For every consult, assert in tests:
`usage.prompt_cache_hit_tokens + usage.prompt_cache_miss_tokens == usage.prompt_tokens`.
**Failure tell:** the invariant doesn't hold → DeepSeek changed the
billing model. Surface as a typed `UsageInvariantViolated` warning.

**B.4 — `system_fingerprint` change = backend rollover signal**
Add an alerting hook: when two consecutive consults emit DIFFERENT
`system_fingerprint` values, log it. DeepSeek doesn't announce rollovers
publicly — fingerprint changes are how we detect them.
**Failure tell:** a sudden quality regression that correlates with a
fingerprint change → roll back any prompts pinned to the old model variant.

**B.5 — Reasoning-only with empty content + finish_reason=stop**
Mock server stream: only `delta.reasoning_content` deltas, no
`delta.content`, then `finish_reason=stop` + usage + `[DONE]`. Expected:
our runner returns Ok with `response_text == ""`. This is a "200-OK but
semantically empty" case Codex flagged.
**Failure tell:** caller code that downstreams the response into something
that breaks on empty strings.
**Mitigation we should add:** typed `EmptyFinalAnswer` failure variant for
the case `finish_reason=stop && content.is_empty() && reasoning.is_some()`.

**B.6 — `insufficient_system_resource` finish_reason**
DeepSeek's API ref lists this as a valid finish_reason (provider capacity
signal). Our `BadFinishReasonKind::Unknown(String)` catches it but as a
generic Unknown. Add an explicit variant + classify as transient (retry
allowed) per Codex's recommendation.
**Failure tell:** capacity-driven failures get misclassified as caller
bugs and don't trigger transient-class breaker arithmetic.

**B.7 — Streaming with no usage chunk despite clean stop+DONE**
Mock the stream: reasoning + content + `finish_reason=stop` + `[DONE]`
but NO usage object/chunk anywhere. Expected: our `finalize_stream`
returns Ok (because finish_reason is stop) and we fall back to
`estimate_usage`. Test this path — our existing test exercises clean
disconnect (no DONE), not clean DONE without usage.

**B.8 — Anti-bulk cap honors message-only, not full-payload size**
Confirm via test: a 10KB `message` + 6KB `cwd`/`repo`/`branch` strings
PASSES the 16KB cap (our check is on `req.message.len()` only, not on
full serialized payload). Document this is the intended semantics.

### TIER 2 — Cheap live (~$0.01 each)

**B.9 — Live thinking=disabled produces empty reasoning_content**
Set `deepseek_thinking: "disabled"`, send a short prompt, confirm
`reasoning_content` is absent or empty in the wire stream. (Our probe-08
covers flash + thinking=disabled, but specifically with the `extra_body`
shape DeepSeek's docs call for. Worth verifying our request body uses the
shape they accept.)

**B.10 — Reasoning tokens billed at output rate, not separately**
After a real consult, decode the price math:
`cost ≈ (prompt_cache_miss × $0.435 + prompt_cache_hit × $0.003625 + completion_tokens × $0.87) / 1M`.
Reasoning_tokens are INSIDE completion_tokens, not added. Compute expected
cost from the response, then check it against the DeepSeek balance
decrement after the call (PROBE-01 reads `/user/balance` cheaply). The
delta should match within rounding.

**B.11 — Cache hit on identical prompt repeat**
Send the same exact prompt twice within ~1 minute. Expected: the second
call shows `prompt_cache_hit_tokens > 0`. Per the docs cache is
"best-effort" so don't assert strict equality, but DO assert `hit > 0`.
**Failure tell:** consistent 0% hit → our prompts contain
non-deterministic prefix bytes (timestamps, uuids).

**B.12 — Single-byte prefix change = full miss**
Run B.11 again with one extra leading character. Expected: hit=0,
miss=full. Confirms our cache model is correct.

**B.13 — V4-Pro 1M context tail behavior**
Send a prompt approaching the 1M context window (don't have to fill it,
but try 100K+ to see latency curve). Expected: latency rises but no
errors. Confirms we don't have any silent truncation upstream.

**B.14 — `max_tokens` low + thinking → reasoning fills budget, empty content**
Set `deepseek_max_tokens: 50` + thinking enabled. Send a prompt that needs
real thinking. Expected: response with `reasoning_content` populated,
`content` empty/short, `finish_reason: "length"`. Our T-007 guard catches
this as `BadFinishReason::Length`. **Verify the per-request log captures
the partial reasoning** — that's the operator's only window into "why did
this fail."

### TIER 3 — Adversarial / capacity (~$0.05 each)

**B.15 — Slow-loris: open consult, drip bytes, never `[DONE]`**
This is hard to trigger from the client side (the server controls dribble
rate). Substitute: confirm our `read_timeout` (60s default) fires when no
byte arrives in that window. Our existing test
`rolling_read_times_out_when_gap_exceeds_read_timeout` covers this for
client-side; the server-side variant is what the 10-minute close protects
against.

**B.16 — Burst-to-429 then verify Retry-After honored**
Fire ~50 concurrent flash requests (cheap model) to deliberately trip a
429. Confirm our runner honors the Retry-After header on retry and
classifies as transient (not hard). Cost cap: ~$0.05 total. Won't be
representative of dynamic peak-hour 429s, but proves the path.

**B.17 — Bad-key 401 latches breaker correctly**
Set `TRIUMVIRATE_DEEPSEEK_API_KEY=sk-deliberately-bad-key`, send one
consult. Expected: `HardProvider(401)` returned, breaker records
HardError(401). Note: 401 is NOT 402, so the breaker should NOT go to
`HardOpenInsufficientBalance` — it stays Closed (just records the
streak). Confirm this is the correct behavior in our T-007 classifier.
(Codex called out this distinction.)

**B.18 — 422 invalid-parameter shape**
Send a deliberately bad payload (e.g. unsupported `response_format`).
Expected: 422 returned, classified as Hard (caller bug), breaker
unaffected. Body should contain `invalid_request_error` type.

### TIER 4 — Destructive (run sparingly)

**B.19 — Drain account balance to <$0.01, confirm 402 latches**
Don't actually do this on the funded account unless we're going to top up
again immediately. The probe battery + intentional 402-bait calls would
consume the remaining balance. If we DO run it: confirm that once 402
fires, the breaker latches `HardOpenInsufficientBalance` and ALL
subsequent calls in the same daemon process return without hitting the
network. Top up and restart daemon → confirm recovery.

**B.20 — Daemon restart preserves cost-attribution across the boundary**
After running B.10, restart the daemon and run again. The
`token-economics` SQLite DB should accumulate records across restarts.
Confirm `attribute_records` sees both calls.

---

## Section C — Code generation prompting playbook (V4-Pro specifically)

Since we don't have a Codex subscription, V4-Pro is our primary code
generator going forward. V4-Pro has 1.6T total parameters / 49B active
per token, and (per multiple sources) responds DIFFERENTLY than the V3
generation to prompts.[^prompt-skywork]

### C.1 — V4-Pro likes structure. Use it.
The CO-STAR framework (Context, Objective, Style, Tone, Audience,
Response) is the most-cited pattern across V4-specific prompting guides.
For code:

  - **Context:** what project / language / runtime / library versions
  - **Objective:** the actual code change ("write a Rust async function
    that…", "refactor this fn to use rusqlite's prepared statements…")
  - **Style:** "match existing surrounding code", "use thiserror not anyhow",
    "no allocations in the hot path"
  - **Tone:** N/A for code
  - **Audience:** "this code will be reviewed by a senior Rust engineer who
    cares about correctness over brevity"
  - **Response:** "return ONLY the diff, no commentary", or "return the
    full file with comments explaining each section"[^prompt-veo4]

### C.2 — Provide version-pinned context
"It is helpful to specify the language and the library versions you are
using." V4-Pro hallucinates dependency surfaces less when versions are
pinned. For our codebase, include:
  - `edition = "2024"`
  - `tokio.workspace = true` features list when relevant
  - `reqwest 0.12, default-features=false, rustls-tls`
  - `serde = "1", serde_json = "1"`[^prompt-skywork]

### C.3 — Structure with explicit markers
V4-Pro responds well to triple-quoted blocks, XML tags, and Markdown
headers as delimiters between instruction and content. Example:

```
<existing_code>
// the function we want to modify
async fn foo() { ... }
</existing_code>

<task>
Add a 60s timeout via tokio::time::timeout, returning a typed Err on expiry.
</task>

<constraints>
- match the surrounding style (no new clippy warnings)
- no new crate deps
- preserve the existing error-message text
</constraints>
```

This is FAR more reliable than blob-of-prose prompts.[^prompt-lightrains]

### C.4 — Thinking mode is your friend for code reviews / debugging
Per the sources: "If it produces code that doesn't work, you can feed the
error message back and it will typically identify the mistake and provide
a correction." Thinking mode helps here — the reasoning_content gives you
the trace of WHY it's making the suggested change, which is critical for
debugging-class tasks.[^prompt-veo4]

  - For code GENERATION from scratch: `effort=high` is usually enough.
  - For BUG hunts / code REVIEW: `effort=max` (or `xhigh`, which maps to
    max). The extra reasoning budget pays for itself.
  - For TRIVIAL edits / search-and-replace: `effort=high` is overkill;
    consider `deepseek_thinking: "disabled"` to skip reasoning entirely.

### C.5 — Cache-friendly prompt structure (= cheaper bills)
Cache hits price at $0.003625/M vs $0.435/M for v4-pro. To maximize hits:

  - **Put stable content FIRST.** System prompt → project context → file
    excerpts → THEN the variable task at the very end.
  - **No timestamps / uuids / per-request IDs in the prefix.** A single
    char change = full miss.
  - **Round prompt structure to 64-token boundaries when possible** (the
    cache chunk size). Pad with `\n` if you have to.
  - **Reuse the same system prompt verbatim across consults.** Even
    whitespace changes invalidate the cache.[^kv-cache]

### C.6 — JSON-mode + tool-use needs explicit schema hints
"JSON mode needs explicit schema hints in the user message." Don't rely on
"return JSON" — paste the actual schema or a minimal example. V4-Pro is
better than V3 here but not bulletproof.[^prompt-deepseek-guide]

### C.7 — DeepThink occasionally skips reasoning
"When DeepThink occasionally skips its reasoning phase, add 'Please start
your response with the <think> tag' to re-activate the chain-of-thought."
Operationally: if you NEED the reasoning trace and it's empty, retry with
that prompt prefix.[^prompt-skywork]

### C.8 — Prompt determinism test
"Run prompts five times at temperature 0 and at 1.3. If outputs vary
wildly at 0, your prompt is ambiguous, not the model." NOTE: thinking
mode IGNORES temperature, so this test only works with
`deepseek_thinking: "disabled"`.[^prompt-deepseek-guide]

### C.9 — V4-Pro for code: comments are context, not decoration
"To get the best technical results from DeepSeek V4, try to provide
comments in your prompt explaining what each section of the code is
intended to do, as this gives the context it needs to ensure generated
code integrates perfectly with your existing project." Read: when pasting
existing code as context, KEEP the comments. Don't strip them to save
tokens.[^prompt-skywork]

---

## Section D — Operational watch list (things we should monitor but can't pre-test)

Things that ONLY show up in production traffic. Add to incident triage:

- **system_fingerprint drift** — log every consult's fingerprint; alert on
  changes. Pre-existing prompts may degrade across rollovers.
- **Cache-hit rate trending toward 0** — a refactor that injects a
  per-call uuid into the prompt head can drop hit rate from 80%+ to 0%
  silently. Track `prompt_cache_hit_tokens / prompt_tokens` rolling
  average per prompt template.
- **Reasoning-token spikes** — Codex's flag: "Alert on reasoning-token
  spikes; they signal prompts that drifted into Think Max territory by
  accident." Add a metric: `completion_tokens_details.reasoning_tokens /
  completion_tokens` ratio per template.
- **Balance decay rate** — `/user/balance` polled hourly during high
  traffic; alert when forecast-to-zero drops below 7 days.
- **Off-peak shift** — if V4 off-peak pricing lands (16:30–00:30 UTC),
  schedule batch work into that window. Worth a `/schedule` job that
  flips a `TRIUMVIRATE_DEEPSEEK_OFFPEAK_OK` env flag at 16:30 UTC.

---

## Section E — Known unknowns (verifiable only by running)

Codex was explicit about what he COULDN'T verify from the docs:

  - "I cannot verify a reliable '02:00-14:00 UTC peak hours' rule from
    official docs." (We previously documented this in the operator
    runbook; it's anecdotal from third-party guides, not from DeepSeek's
    own publication. Treat as advisory.)
  - "I cannot verify cache hit/miss tokens are 'unreliable' as a provider
    accounting defect. Phrase it as: cache behavior is opportunistic and
    should not be assumed deterministic."
  - "I cannot verify model rollovers solely from system_fingerprint
    patterns, only that the field represents backend configuration and
    should be logged for correlation."

Translation: our runbook says "expect 503 in 02:00–14:00 UTC" but
DeepSeek doesn't publish a specific window. The off-peak DISCOUNT window
(16:30–00:30 UTC) is official; the peak-HOUR-503 window is community
wisdom.

---

## Suggested next actions (concrete and bounded)

1. **Run TIER 1 tests now** (no API spend). They surface 5 useful gaps —
   add the regressions to `daemon/crates/mcp-bridge/src/deepseek.rs::tests`.
2. **Run TIER 2 tests in one session** (~$0.10 total). Confirms the
   cache, thinking-mode-disable, and reasoning-budget math against the
   live API. Capture results in a new section of `PROBE_RESULTS.md`.
3. **Defer TIER 3/4 until** there's a specific reason (incident, scale-up,
   integration to a new client). Don't burn money on speculative probes.
4. **Add the watch-list metrics** (Section D) to whatever metrics surface
   the daemon exposes (Prometheus, JSON dump, whatever). Mostly cheap.
5. **Rotate the API key** before the next big traffic session — the
   current key is in the goatrodeo transcript.

---

## Citations

[^rate-limit]: DeepSeek official rate-limit & isolation doc — https://api-docs.deepseek.com/quick_start/rate_limit
[^thinking]: DeepSeek official thinking-mode guide — https://api-docs.deepseek.com/guides/thinking_mode
[^kv-cache]: DeepSeek official context caching guide — https://api-docs.deepseek.com/guides/kv_cache
[^pricing]: DeepSeek official pricing — https://api-docs.deepseek.com/quick_start/pricing
[^offpeak]: DeepSeek official off-peak discount announcement — https://x.com/deepseek_ai/status/1894710448676884671
[^scmp]: South China Morning Post coverage of the off-peak discount — https://www.scmp.com/tech/tech-trends/article/3300264/ai-night-chinas-deepseek-offers-peak-75-discount-demand-strains-servers
[^prompt-skywork]: Skywork — "Mastering DeepSeek V4 Prompt Engineering" — https://skywork.ai/skypage/en/mastering-deepseek-prompt-engineering/2047585323291725824
[^prompt-veo4]: Veo4 — "Deepseek V4 Prompt Engineering: Tips for Better Results" — https://veo4.dev/blogs/deepseek-v4-prompt-engineering-tips-for-better-results-20260201
[^prompt-lightrains]: Lightrains — "DeepSeek V4 Prompt Engineering: What Actually Works in Production" — https://lightrains.com/blogs/deepseek-prompt-engineering-best-practices
[^prompt-deepseek-guide]: deepseekai.guide — "DeepSeek Prompt Engineering: A V4 Practitioner's Guide" — https://deepseekai.guide/tutorials/deepseek-prompt-engineering/

**Third-party error-code reference (helpful but not canonical):**
- chat-deep.ai error-codes — https://chat-deep.ai/docs/deepseek-error-codes/

**GitHub issues confirming the reasoning_content round-trip 400:**
- https://github.com/nearai/ironclaw/issues/3436
- https://github.com/anomalyco/opencode/issues/24190
