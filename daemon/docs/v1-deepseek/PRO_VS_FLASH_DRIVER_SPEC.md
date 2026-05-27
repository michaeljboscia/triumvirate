# Pro-vs-Flash Eval Driver — Spec + Outcome Definition

**Status:** Draft for 4-agent adversarial review (Claude, Codex, Gemini, DeepSeek).
**Author:** Claude Opus 4.7 (in session 2026-05-26).
**Reviewers (independent, no cross-talk):** Codex, Gemini, DeepSeek (via `ask_agent`); my own critique appended at the end of THIS file.

---

## 1. Why this exists

We just merged a DeepSeek integration that defaults to `v4-pro`. We don't know empirically whether `v4-flash` would be good enough for most of our workflows at ~12× lower output cost. The integration's per-call `deepseek_model` override (PR #39) is the mechanism — this eval generates the DATA that justifies the default.

The eval design is in `PRO_VS_FLASH_TEST_PLAN.md` (Categories A-G, ~$0.50 ceiling). This document specs **the driver that executes that plan**.

## 2. Inputs

### 2.1 Task definitions (`tasks/{category}/{task_id}.toml`)
One TOML file per task. Schema:

```toml
[task]
id = "A.1"
category = "trivial-edit"
description = "Rename a function across a file (Rust)"
ground_truth_kind = "diff_match"  # one of: diff_match | compile_ok | parse_match | assertion_pass | council_judge

[prompt]
system = ""  # optional override; defaults to runner's NO_TOOL_EMULATION_SYSTEM
user_template = "Rename `{old_fn}` to `{new_fn}` in the attached file..."
input_files = ["fixtures/A.1/deepseek.rs"]  # absolute or relative to driver

[scoring]
# Per ground_truth_kind:
# diff_match: expected diff fixture path
expected_diff = "fixtures/A.1/expected.diff"
# OR for council_judge:
council_rubric = "fixtures/E.1/rubric.md"
council_judges = ["codex", "gemini"]  # internal Claude is implicit 3rd

[budget]
max_tokens = 2048
reasoning_effort = "high"  # one of: high | max | (omit for thinking=disabled)
thinking = "enabled"
```

### 2.2 Model matrix (`models.toml`)
```toml
[[models]]
id = "pro-thinking-high"
deepseek_model = "deepseek-v4-pro"
deepseek_thinking = "enabled"
deepseek_reasoning_effort = "high"

[[models]]
id = "flash-thinking-high"
deepseek_model = "deepseek-v4-flash"
deepseek_thinking = "enabled"
deepseek_reasoning_effort = "high"

# Optional: cheap-mode comparisons
[[models]]
id = "flash-no-thinking"
deepseek_model = "deepseek-v4-flash"
deepseek_thinking = "disabled"
```

### 2.3 Environment
- `~/.triumvirate/deepseek.key` resolved via the daemon's file fallback (PR #38)
- Daemon binary at `daemon/target/release/triumvirate` running and reachable on `127.0.0.1:18180`
- Driver runs as a separate process; uses `ask_agent` HTTP route directly (not MCP, to avoid Claude-Code-window coupling)

## 3. Outputs

### 3.1 `results/{run_id}/results.jsonl`
One row per (task × model) consult. Atomic-write (tmp + rename) so a crash mid-run leaves at most the in-flight row uncommitted.
```json
{
  "ts": "2026-05-26T17:00:00Z",
  "run_id": "20260526T170000Z-abc123",
  "task_id": "A.1",
  "model_id": "pro-thinking-high",
  "model": "deepseek-v4-pro",
  "request_id": "chatcmpl-...",   // from DeepSeek
  "response_text": "...",          // full body (capped at 32KB; truncation flagged)
  "reasoning_chars": 1842,
  "completion_tokens": 423,
  "reasoning_tokens": 387,        // from completion_tokens_details
  "prompt_tokens": 1206,
  "cache_hit_tokens": 0,
  "cache_miss_tokens": 1206,
  "wall_clock_ms": 8432,
  "cost_usd": 0.000526,
  "score_kind": "diff_match",
  "score": 1.0,                    // 0.0 or 1.0 for binary; 0..1 for council
  "score_details": { ... },        // method-specific
  "failure_kind": null             // populated on Err; null on Ok
}
```

### 3.2 `results/{run_id}/cost_ledger.json`
```json
{
  "run_id": "...",
  "started": "...",
  "ended": "...",
  "phases": [
    {"name": "phase-1-trivial", "start_balance": 9.98, "end_balance": 9.97, "delta": 0.01},
    ...
  ],
  "total_spend": 0.18,
  "ceiling": 0.50,
  "hit_ceiling": false
}
```

### 3.3 `results/{run_id}/decision_matrix.md`
Final human-readable summary. One row per task:

| Task | Pro | Flash | Winner | Cost ratio | Recommendation |
|---|---|---|---|---|---|
| A.1 rename | ✅ pass | ✅ pass | tie | Pro 10× more | **Flash sufficient** |
| C.3 reasoning trace quality | (4.6/5) | (3.1/5) | Pro | Pro 8× more | **Pro for trace tasks** |

### 3.4 `results/{run_id}/failures/{task_id}-{model_id}.json`
For any task where score < 1.0, capture the full per-request log path + response + reasoning trace excerpt for post-mortem.

### 3.5 `results/{run_id}/MANIFEST.md`
- Driver SHA
- System-prompt SHA
- Model versions (from API `/models` endpoint at run start)
- Task definitions SHA (hash of all `.toml` files)
- Daemon binary SHA
- DeepSeek balance start/end

## 4. Execution model

### 4.1 Phases (cheap → expensive)
```
phase-1-trivial      = A.1, A.2, D.1                            # ~$0.03
phase-2-codegen      = B.1, B.2                                 # ~$0.04
phase-3-bug-analysis = C.1, C.2, C.3                            # ~$0.06
phase-4-structured   = D.2                                       # ~$0.02
phase-5-prod-modes   = F.1 (5 trials each), F.2 (10 trials)     # ~$0.08
phase-6-council      = E.1, E.2                                  # ~$0.10
phase-7-judgment     = council-judging pass for C.3/E.1/F.1     # ~$0.10
                                                          TOTAL  ~$0.43
```

### 4.2 Between-phase circuit breakers
After EACH phase:
1. Query `/user/balance` via daemon's runner
2. Compute actual spend = pre-phase balance - current balance
3. If actual_spend > phase_budget × 1.5 → abort with diagnostic, don't proceed
4. If total_spend > ceiling × 0.9 → abort even if next phase is small (safety margin)
5. Write phase row to `cost_ledger.json` immediately (don't batch)

### 4.3 Per-consult fairness controls
- **Alternating order:** Pro first on odd tasks, Flash first on even. Defeats any caching warmup that would favor whichever ran first.
- **Idempotency:** same task definition (id + prompt + inputs) hashes to the same task_hash; driver refuses to re-run a task_hash in the same run_id (manifest-tracked).
- **Cache poisoning prevention:** identical prompt across both models means cache may hit on second consult; record `cache_hit_tokens > 0` honestly, don't normalize. (DeepSeek cache is best-effort per B.11 anyway.)

### 4.4 Council-judging pass design
For tasks with `ground_truth_kind = "council_judge"`:
1. Collect Pro and Flash responses
2. Build a JUDGE prompt:
   ```
   You are an independent judge. Score these two responses on a rubric of 5 dimensions (1-5 each).
   The responses come from two models you DO NOT KNOW the names of (presented as "Response A" and "Response B" in randomized order).
   Rubric: {load from fixtures/{task}/rubric.md}
   Task: {original prompt}
   Response A: {Pro-or-Flash, depending on randomization}
   Response B: {the other}
   Output JSON: {"a_total": int, "b_total": int, "per_dim": {...}, "winner": "a" | "b" | "tie", "rationale": "..."}
   ```
3. Send same judge prompt to **Codex** AND **Gemini** via existing `ask_agent`. (Internal Claude judge OPTIONAL — see §6 bias).
4. De-randomize: "Response A" → which model? Capture both judges' scores.
5. **Aggregate:** majority vote on winner; average per-dim scores.
6. If judges disagree (one says A, one says B): flag as `judge_disagreement`, count as tie.

### 4.5 Anti-bias measures
- **Blind judging:** judges see "Response A" / "Response B", not "Pro" / "Flash". Order randomized per task.
- **Same effort levels:** both models get the same `reasoning_effort` per task (Pro+high vs Flash+high, NOT Pro+high vs Flash+max — that's its own experiment).
- **Reasoning trace EXCLUDED from response shown to judge** by default. Judge sees only the model's `content`, not the `reasoning_content`. (Some tasks may opt INTO showing reasoning — explicit per-task flag.)

## 5. Failure handling

| Failure | Driver response |
|---|---|
| Single consult returns typed `DeepSeekFailure` | Record `failure_kind`, score=0, continue |
| Two consecutive `BadFinishReason::Length` for same task | Bump `max_tokens` for that task once, retry; if STILL fails, score=0 |
| `BreakerOpen` | Wait 60s, retry once; if still open, abort entire phase |
| `MissingApiKey` | Abort entire run immediately (operator setup error) |
| Cost ceiling breach mid-phase | Finish in-flight consult, write what we have, abort |
| Judge disagreement on council-judge tasks | Flag, count as tie, surface in decision_matrix |

## 6. Known risks (where this design might be wrong)

### 6.1 Judge bias
Codex and Gemini are independent of DeepSeek, but they have their own training-data preferences. A Gemini judge may prefer responses that "sound more Google-like." Mitigations attempted: blind-order randomization, multi-judge aggregation, dimensional rubric.

**Open question:** should we ALSO include an internal Claude judge (this session)? Pro: third independent vote. Con: Claude knows it ran this driver; not truly blind. **Recommendation: include but weight at 0.5 (Codex/Gemini full vote; Claude tie-breaker).**

### 6.2 Cache contamination
If we run Pro first on task X, then Flash on task X, the API might hit cache on the second call (cheaper input cost). We record this honestly but DON'T normalize cost comparisons. The COST column should reflect what the operator would actually pay — including realistic cache hits.

### 6.3 Stochasticity
Even with thinking mode (which ignores temperature), DeepSeek's outputs aren't fully deterministic per their docs. Single-trial results are noisy. **Mitigation: F.1 runs 5 trials per model**; aggregate score for that task. Other tasks: single trial unless a draw needs disambiguation.

### 6.4 Eval drift
Our task fixtures are drawn from this session's actual work. They may not represent FUTURE workflows. We document this in the decision matrix — "valid as of 2026-05-26 / Rust daemon + LCB harness work".

## 7. Implementation language

**Bash + Python (no new Rust).** Rationale: driver is a 1-off / occasional-rerun tool, not a production daemon component. Python `requests` for the daemon HTTP calls. TOML via `tomllib` (stdlib). JSON output via `json` (stdlib). Plain `git` for capturing SHAs. Total LoC budget: ~400 lines.

## 8. What this driver IS NOT

- Not a continuous benchmark service
- Not part of CI (one-off run, decision-time tool)
- Not opinionated about which model wins — only generates the data
- Not a substitute for production observability (the per-request log already exists)

---

## Claude's (my own) initial critique

After writing this I see these holes I want the other 3 agents to chew on:

**C-1. Cost ceiling is too generous AND too tight.** The $0.50 budget assumes ~$0.005/consult on average. But Pro consults with thinking+high regularly burn 5K-20K reasoning tokens. At $0.87/M output (Pro discounted), that's $0.004-$0.017 per consult JUST for thinking. With 10+ tasks × 2 models × 1 trial + 5-trial F.1 + judging pass, the realistic spend could be 2-5× the ceiling. The driver MUST poll balance mid-phase, not just between phases. **Fix: balance-check after every 3rd consult.**

**C-2. Council judging risks "majority of two."** Codex and Gemini are two judges. If they split, we count as tie. But two-judge "majority" is just unanimity. We need a tie-breaker. My proposal (Claude with 0.5 weight) feels wrong because I'm the driver author. **Better fix: ALL THREE OF (Codex, Gemini, Claude-Claude-Code-instance) judge, with Claude weighted 0.33 like the others. Three judges, simple majority.**

**C-3. "Blind judging" doesn't generalize.** I randomize A/B order, but the model's response STYLE may reveal which it is (Pro tends to be more verbose; Flash tends toward terseness). A judge that gets ~70% accurate at guessing-by-style negates the blinding. **Fix: explicitly tell judges in the prompt: "DO NOT try to guess which is which; score on the rubric alone." Maybe also: post-test, ask judges if they could identify which was which; if yes, that's a methodology problem.**

**C-4. Programmatic scoring is fragile.** "compile_ok" / "diff_match" / "parse_match" — what if the model produces a CORRECT solution that just differs syntactically from the expected? E.g. A.1 rename might use a different shell command. **Fix: each "expected" fixture must include multiple acceptable forms (`diff_alternatives: [...]`) OR fall back to "diff-AST-match" via tree-sitter when available.**

**C-5. Phase ordering is wrong.** Cheap → expensive is the COST order, but for SCIENTIFIC ORDER, we should run the FAILURE-PRONE tasks (F.1 tool-tag-leak) FIRST, because if they fail consistently on both models, the rest of the eval changes interpretation. **Fix: F.1 + F.2 run as phase 0, cheaper than phase 1 cost-wise, and inform the ceiling for subsequent phases.**

**C-6. Driver SHA isn't enough for reproducibility.** If the daemon's NO_TOOL_EMULATION_SYSTEM prompt changes between runs, results differ. **Fix: capture the system prompt SHA explicitly in the manifest (not just the daemon binary SHA — those couple but the prompt is the load-bearing artifact for F.1 specifically).**

**C-7. We don't measure RETRY behavior.** The whole point of category G was to test mutation-retry. The current driver only does single-shot per consult. **Fix: add a `retry_policy` field per task; tasks tagged with `retry_eligible = true` get the mutation-retry test (first attempt verbatim, second attempt with mutation prefix), and we score whether mutation actually escapes the failure attractor.**

**C-8. F.1 prompt risks the same self-fulfilling trap as the production failure.** If we include `<triumvirate_tool>` in the F.1 prompt as the "thing to NOT do," the model may emit it. **Fix: explicit test design — F.1 should test with AND without the literal tag in the prompt, comparing rates. The system prompt should suppress in both cases, but we measure how much.**

---

## Questions for the 3 other agents

Independent review only. No coordination. Each of you sees this document and my critique.

1. **What's structurally broken that I missed?** Methodology flaws, scoring errors, design holes my critique didn't already catch.
2. **What's overspec'd?** What's defensive-engineered to a degree that adds complexity without measurement value?
3. **Where's the BIAS** — in the spec, in the scoring, in the judging?
4. **Concrete code-level concern** — when you imagine yourself implementing this driver, where does the code go wrong?
5. **Is the cost model right?** $0.50 ceiling realistic, or am I underestimating thinking-token cost?
6. **What's a CHEAPER way to get the same answer?** If we could do this for $0.05 with the same decision power, what would change?
7. **Sign-off:** if I implement this exactly as spec'd, do you believe the resulting data is enough to flip the default model? Or is more rigor needed?

Be brutal. Disagreement with me AND with each other is the point.
