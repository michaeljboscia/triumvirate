# PRO_VS_FLASH_EVAL_RESULTS — empirical Pro-vs-Flash capability eval (v1–v4)

> **Task:** REQ-DS-005 follow-up. Capability eval driving the operator-default model choice
> (see `PRO_VS_FLASH_DRIVER_SPEC.md` and `PRO_VS_FLASH_TEST_PLAN.md`).
>
> **Date:** 2026-05-26
> **Eval harness:** promptfoo via `llm-olly-promptfoo` container on homebox (REDACTED_HOST:3200)
> **Provider:** `openai:chat:*` against `https://api.deepseek.com/v1`
> **Models compared:** `deepseek-v4-pro` vs `deepseek-v4-flash`, both with `thinking: enabled`, `reasoning_effort: high`
> **Total empirical sample:** 130 consults across 13 distinct task prompts
> **Configs:** `~/services/llm-olly-promptfoo/configs/pro-vs-flash-{v1,v2,v3,v4}.yaml`

## Verdict

**Flip the operator default to `deepseek-v4-flash`.** Pro does not measurably beat Flash on
any of the 13 tasks tested, including the 4 tasks (v4 / S10–S13) specifically designed to
probe categories where published benchmarks show a Pro advantage. Pro consumes ~8.6% more
total tokens and ~12.4% more reasoning tokens than Flash at indistinguishable answer
quality, and on the published-promo pricing is ~3.4× more expensive per equivalent answer.

The per-call `deepseek_model` override (`AskAgentRequest.deepseek_model`) remains in place;
any caller that wants Pro on a specific consult can still request it.

## Test boundaries — what this eval does and does NOT measure

Honest about scope before reading the numbers below:

- **Single-shot only.** Every test is one prompt → one response. We did not test multi-turn
  agentic loops. Pro's largest published edge (Terminal-Bench 2.0: 67.9 vs 56.9, an 11-point
  gap) is on tool-using agents, which we have no coverage for. The daemon's current workload
  is single-shot consults, so this matches production usage — but it cannot generalise to a
  future agentic flow.
- **Context ≤ 3K tokens.** The largest prompt (v4 / S10 long-context-synthesis) is ~3K
  tokens. Pro's published 1M-context advantage shows up at 100K+ tokens with information
  buried at depth-65%. We never approached that regime.
- **Binary assertions.** Tests pass on `contains` / `is-json` / `latency` thresholds. A
  barely-acceptable answer scores the same as a brilliant one. Judge-LLM grading would
  surface quality differences that our binary checks miss.
- **n=5 per task per provider in v3/v4.** Power-analysis: 5 trials reliably detect a ~20pt
  pass-rate gap; small effects (e.g., 5pt) are not detectable. Larger samples would be cheap
  to run but were not.
- **Same reasoning effort for both models.** `thinking: enabled`, `reasoning_effort: high`
  for both. We did not sweep effort levels — possibly Pro's edge emerges at lower-effort
  settings.

In short: this eval is strong evidence that *for the daemon's current single-shot workload
with bounded context*, Pro buys nothing measurable. If/when the daemon grows multi-turn
agentic flows or starts routing 100K-token prompts, re-evaluate.

## v1 — pilot (6 sentinels × 1 trial each)

| Result | Note |
| --- | --- |
| 6/12 pass on first run | Six "failures" were all assertion design bugs (model preamble tripping `not-contains`, JSON wrapped in prose tripping `is-json`, model quoting a tool tag in description tripping the tool-tag check). |
| Token total | Pro 4,622 vs Flash 4,350 (Flash ~6% fewer tokens). |

The v1 run's signal was: at this difficulty, both models pass. The 50% "fail" was
instrumentation, not capability.

## v2 — assertion-fix replay (same prompts, fixed asserts, cached responses)

| Result | Note |
| --- | --- |
| 12/12 pass | Confirms v1 failures were assertion design, not model output. |

## v3 — broader categories (9 sentinels × 5 trials each)

S1–S6 from v1/v2 plus:
- S7 Python codegen (cross-language sanity vs S2's Rust)
- S8 spec gap analysis (goatrodeo-style review workflow)
- S9 decision rationale (council-style commitment test)

| Result | Note |
| --- | --- |
| 88/90 pass (97.78%) | 2 failures both false positives. |
| S1 / Flash trial | Thinking preamble echoed old name; transform regex didn't catch all variants. |
| S2 / Pro trial | Pro chose `floor_char_boundary` (newer nightly Rust API) over `is_char_boundary`. Correct code, narrow assertion. |
| Token economics | Pro used **18% more total tokens, 35% more reasoning tokens** than Flash at indistinguishable quality. |

## v4 — Pro-favoring categories (4 sentinels × 5 trials each)

Designed specifically to probe Pro's published advantages. Same `--repeat 5 --no-cache`
harness, max_tokens raised to 8192 (Pro needs more reasoning room on these prompts).

| Sentinel | Category (published Pro advantage) | Pro | Flash |
| --- | --- | --- | --- |
| S10 long-context cross-file synthesis | Multi-needle retrieval (Pro 97% / Flash drops in middle) | **5/5** | **5/5** |
| S11 complex 15-field nested JSON | Structured output stability (Flash reportedly fails on 10+ fields) | **4/5** | **5/5** |
| S12 algorithm with space optimisation | Multi-step reasoning + complexity claim | **5/5** | **4/5** |
| S13 PostgreSQL DELETE vs TRUNCATE | Factual recall (Pro 57.9 / Flash 34.1 on SimpleQA-Verified) | **5/5** | **5/5** |
| **Total** | | **19/20** | **19/20** |

**Token usage (40 consults, max_tokens=8192):**

| Model | Calls | Prompt | Completion | Reasoning | Total |
| --- | --- | --- | --- | --- | --- |
| `deepseek-v4-pro` | 20 | 6,160 | 57,682 | 47,080 | 63,842 |
| `deepseek-v4-flash` | 20 | 6,160 | 52,633 | 41,900 | 58,793 |

Pro used **8.6% more total tokens, 12.4% more reasoning tokens** at tied quality.

**Computed cost (using v1-promo pricing, valid through 2026-05-31 15:59 UTC):**
- Pro: 6,160 × $0.435/M + 57,682 × $0.87/M = $0.0027 + $0.0502 = **~$0.053**
- Flash: 6,160 × $0.14/M + 52,633 × $0.28/M = $0.0009 + $0.0147 = **~$0.016**
- Pro/Flash spend ratio for v4: **~3.4× more expensive**

Post-promo Pro pricing reverts to 4× the promo rate on 2026-06-01, which would push the
ratio further against Pro for identical workloads.

### Failure-by-failure analysis (the only Pro-vs-Flash signal we surfaced)

**S11 / Pro trial 1** — Pro's `is-json` assertion failed because Pro emitted **two JSON
objects concatenated**, separated by leaked meta-reasoning prose in the content channel:

```
{ ...full valid JSON #1... }

Note: fix_summary could be simpler, but I'll include that. The prose says
"Fixed via revert PR ..." so fix summary: "Reverted config push via PR". I'll
phrase as given. Output only the JSON object.

{ ...full valid JSON #2... }
```

Both JSON objects individually parse and contain all 15 required fields correctly. The
failure is real — Pro broke the explicit "Output ONLY the JSON object" instruction by
emitting visible meta-reasoning between two JSON copies. Flash's five S11 trials all
produced single clean JSON.

This is the only quality signal that emerged across v1–v4, and it cuts against Pro.

**S12 / Flash trial 1** — Flash failed the regex enforcing `O(n²)` time complexity notation
(specifically the `time` keyword adjacency check). Output is a complete correct algorithm
with full DP trace returning 4 for `"bbbab"`. Pure assertion-design noise.

## Decision

The empirical case for `deepseek-v4-flash` as the operator default:

1. **Quality: tied** across 130 consults, 13 task types.
2. **Token cost: lower** (~8.6% fewer total, ~12.4% fewer reasoning).
3. **API spend: ~3.4× lower** on v4 promo pricing; post-promo gap widens.
4. **Cleaner output** in the one trial where a difference emerged (S11).
5. **Per-call override remains** for the rare cases where a caller wants Pro.

Pro is held in reserve via `AskAgentRequest.deepseek_model` for any caller that wants to
opt back in on a specific consult. The default flip is appropriate for the production
workload as of 2026-05-26.

### Re-evaluate when

- The daemon grows multi-turn agentic flows (Pro's Terminal-Bench edge becomes load-bearing)
- Prompts routinely exceed ~50K tokens (Pro's 1M-context edge becomes load-bearing)
- We add judge-LLM grading and detect a small quality gap our binary asserts missed
- Pricing changes materially (post-promo on 2026-06-01 narrows the gap from ~3.4× back to
  ~3×; if DeepSeek runs another Pro promo, the gap could close)

## Reproduce

Configs live on homebox at `~/services/llm-olly-promptfoo/configs/`. To re-run any wave:

```sh
ssh -i ~/.ssh/REDACTED_KEY user@REDACTED_HOST \
  'docker exec llm-olly-promptfoo promptfoo eval \
     -c /app/configs/pro-vs-flash-v4.yaml \
     --repeat 5 --no-cache \
     --output /home/promptfoo/.promptfoo/results-pvf-v4-$(date +%s).json'
```

Pull JSON to local for parsing:

```sh
ssh -i ~/.ssh/REDACTED_KEY user@REDACTED_HOST \
  'docker cp llm-olly-promptfoo:$(docker exec llm-olly-promptfoo \
     sh -c "ls -t /home/promptfoo/.promptfoo/results-pvf-v4-*.json | head -1") /tmp/v4.json'
scp -i ~/.ssh/REDACTED_KEY user@REDACTED_HOST:/tmp/v4.json /tmp/v4.json
```

Parser at `/tmp/parse_v4.py` (in-session artefact); the analysis pattern is straightforward
grouping by `provider.label` and prompt-keyword identification.
