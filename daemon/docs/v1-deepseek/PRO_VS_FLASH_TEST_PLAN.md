# DeepSeek Pro vs Flash — Capability Eval Plan

**Status:** Plan-only. Execute after PR `feat/deepseek-per-call-model-override` merges.
**Author of this plan:** Claude Opus 4.7
**Cost ceiling:** ~$0.50 across all 10 buckets, both models, 2 rounds. ~5× headroom for retries.
**Total wall-clock estimate:** ~30 min execution; ~2 hr including manual judgment passes.

---

## Executive summary

After this eval we should be able to answer the **only question that matters**: for the kinds of tasks we actually use DeepSeek for (code generation, code review, bug analysis, structured output, council-style decisions), **where does Flash suffice and where does Pro materially pay for itself?**

The plan does NOT try to reproduce published benchmark numbers. It tests OUR usage patterns, against TASKS WITH KNOWN-CORRECT OUTCOMES drawn from this codebase, with **comparative scoring against either programmatic ground truth or a 3-agent judge council** (Claude + Codex + Gemini) so the answer doesn't depend on a single LLM's taste.

**Output:** a decision matrix the operator references at call-site:

| Task type | Flash sufficient? | If escalate, when? |
|---|---|---|
| (filled in by the eval results) | | |

---

## Why this matters (published signal we're pressure-testing)

After two parallel research passes (WebSearch + gemini-search grounded) the
public signal is concrete and the eval buckets below test against it directly.

### Where benchmarks say Flash holds up
- **LiveCodeBench Pass@1:** Pro **93.5** vs Flash **91.6** — 1.9pt gap.[^codersera]
- **SWE-bench Verified:** Pro **80.6** vs Flash **79.0** — 1.6pt gap.[^codersera][^macaron]
- Within 1-3 pts on **most coding, knowledge, and bounded-reasoning** benchmarks.[^codersera]

### Where benchmarks say Flash materially loses (the gaps we test)
- **Terminal-Bench 2.0** (agentic tool-use loops): Flash **56.9** vs Pro **67.9** — **11pt gap**. Developers report Flash loops "derail" after **5–10 tool calls**, hallucinating shell outputs or failing to recover from malformed CLI arguments.[^wavespeed]
- **SimpleQA-Verified** (factual recall): Flash **34.1** vs Pro **57.9** — **23.8pt gap**. Manifests as **identifier hallucination** in code (Flash makes up library calls / non-existent crate methods when context is sparse).[^towardsai]
- **AA-Omniscience hallucination rate:** Flash **96%** (guesses rather than admits ignorance).[^artificialanalysis]
- **Multi-file refactor / cross-file reasoning:** Pro maintains 97% on multi-query "needle-in-1M-haystack"; Flash drops significantly in the MIDDLE of the 1M context.[^lightning]
- **JSON schema violations on complex schemas (10+ fields):** Flash has materially higher violation rate; Pro stays stable.[^kilo]

### Architecture-specific failure modes (worth testing for)
- **"Identifier bleeding":** Flash's hybrid attention (CSA/HCA) reuses variable names from earlier unrelated turns in long conversations.[^outcomeschool]
- **"Sliding window blindness":** Flash's first 2 layers use pure sliding-window attention — so critical info in the FIRST TOKENS of a 1M sequence may be ignored.[^outcomeschool]

### Where Flash matches or BEATS Pro (cost-adjusted)
- **One-shot implementation** (write trait, write fn from spec): Flash often produces same code as Pro, 4× faster, ~120× cheaper at value-per-success.[^towardsai]
- **Documentation / commit messages / PR summaries from diffs.**
- **High-volume classification** (error log triage, security scanning).

### Production routing methodology (well-documented)
"Phase-Aware Routing" — default to Flash, escalate to Pro on concrete triggers:[^coderouter]
- **>3 intermediate reasoning steps** in the task → escalate
- **>10 distinct source files or >200K tokens of unstructured context** → escalate
- **>4 tool calls per turn** expected → escalate (avoid Flash's state-drift)
- **Tool-call-failure** or confidence-threshold-missed in-flight → retry on Pro

### Prior-art eval results we can compare against
- **Cheu Loong Nian's 20-task eval:** Flash won 7/20 (isolated code); Pro-Max swept 1M-context + agentic chains.[^towardsai]
- **Kilo.ai FlowGraph Spec** (full project generation): Pro **77/100**, Flash **60/100** — Flash failed on subtle spec-level gaps and malformed config files. **17pt gap.**[^kilo]

If (1) holds for our workloads → Flash-as-default is correct, opt-into-Pro is rare.
If (2) and the architecture quirks bite us → router rule needed (escalate on
bug-hunt / cross-file / agentic-tool patterns per the threshold above).

---

## Bucket design — 10 tasks across 5 categories

All 10 are drawn from real session artifacts so we have ground truth. Each task ran against both `deepseek-v4-pro` AND `deepseek-v4-flash` at multiple `reasoning_effort` levels.

### Category A — Trivial edits (Flash should dominate; this is the floor test)

**A.1: Rename a function across a file (Rust)**
- **Prompt:** "Rename `record_fingerprint_for_model` to `track_model_fingerprint` in the attached file. Return the full file. Touch only what the rename requires."
- **Input:** `daemon/crates/mcp-bridge/src/deepseek.rs` (the file containing the helper)
- **Ground truth:** mechanical — every call site + the definition updates; nothing else changes
- **Scoring:** diff against expected; binary pass/fail. Diff = a single `s/record_fingerprint_for_model/track_model_fingerprint/g` should be sufficient.
- **Expected cost:** Flash ~$0.0005, Pro ~$0.005 (10× cheaper for the same right answer)
- **Hypothesis:** Flash crushes this. Pro is wasted spend.

**A.2: Add a field to a struct with serde tag (Rust)**
- **Prompt:** "Add an optional `notes: Option<String>` field to `RawUsage` with `#[serde(default, skip_serializing_if = \"Option::is_none\")]`. Keep all other fields unchanged."
- **Ground truth:** trivial; we verify by `cargo check` afterwards
- **Scoring:** does the produced file compile? Does the diff match the expected shape?
- **Hypothesis:** both succeed; Flash chosen.

### Category B — Code generation from spec (the daily workflow)

**B.1: Write a small function from a one-paragraph spec (Rust)**
- **Prompt:** "Write a Rust function `cap_reasoning(s: &str, cap_bytes: usize) -> &str` that returns a UTF-8-safe truncated prefix of `s` no longer than `cap_bytes` bytes. If the truncation would split a multi-byte char, back up to the previous char boundary. Tests must verify: short string returns the original; 🦀 (4 bytes) capped at 3 returns empty; 🦀abc capped at 4 returns 🦀."
- **Ground truth:** the existing `cap_reasoning` in `daemon/crates/mcp-bridge/src/deepseek.rs` — we know what the right implementation looks like
- **Scoring:** does the generated code (a) compile, (b) pass the 3 acceptance tests in the prompt? Binary pass/fail.
- **Hypothesis:** Flash should pass. Pro should pass. The question is response quality (comments, edge case handling).

**B.2: Write an integration test from a contract description**
- **Prompt:** "Write a `#[tokio::test]` for a Rust function that hits a local mock HTTP server and asserts the response. The mock server should return HTTP 429 with `Retry-After: 1` on the FIRST request and HTTP 200 with body `{\"ok\":true}` on the second. Assert that the test client honored the retry-after wait (elapsed >= 900ms) and got the 200 response. Use `tokio::net::TcpListener` for the mock; no hyper dep."
- **Ground truth:** the existing `deepseek_runner_429_with_retry_after_then_succeeds` test in this codebase
- **Scoring:** does the test pass against the production runner? Does it actually wait the 900ms? Does it use raw TCP (not hyper)?
- **Hypothesis:** Pro probably edges out — this is multi-step and the constraint set is real.

### Category C — Bug analysis (where this session showed real edges)

**C.1: The ABE pattern-drift bug**
- **Prompt:** Identical to the one we sent the 3-agent council earlier this session (DeepSeek/Codex/Gemini all converged on Option B — "// pending"/"// stub" comment-prefixed patterns). Include the test code + the validator code + the 5 options.
- **Ground truth:** Option B (no colon). The 3-agent council already agreed; Pro AND Flash should arrive at the same answer.
- **Scoring:** does the response pick Option B? Does the reasoning trace cite the false-positive risk on bare identifiers? Does it mention the colon trade-off?
- **Hypothesis:** Flash should still arrive at Option B. The reasoning trace quality MAY differ — Pro's tends to be more thorough.

**C.2: The B.9 wire-shape bug** (multi-step inference from sparse evidence)
- **Prompt:** "Here's a Rust function `build_request_body` that produces JSON for an HTTP POST to `api.deepseek.com/v1/chat/completions`. Live testing shows the API returns HTTP 400 with this body: `{\"error\":{\"message\":\"Failed to deserialize the JSON body into the target type: thinking: invalid type: string \\\"enabled\\\", expected struct ThinkingOptions\"}}`. What's wrong and how do you fix it? Give the exact code change."
- **Ground truth:** the `thinking: {"type": ...}` nested-object fix from commit `aae4eb4`
- **Scoring:** does the response identify nested vs flat? Does it give the exact replacement code (or close enough to copy)?
- **Hypothesis:** Pro and Flash should both get this. The API error message is unambiguous.

**C.3: Reasoning trace quality on hard debugging** (the long-horizon test)
- **Prompt:** "A Rust test `abe_red_team_enforcement_blocks_non_compliant_worker` fails. Three sub-cases dispatch worker scripts; the first two pass (they write forbidden files or use bad commit messages, both caught by the validator). The third writes `// pending: stub` to an allowed file and is supposed to be rejected by stub-detection. The validator's stub_patterns list contains only `[\"todo!()\", \"unimplemented!()\", \"TODO\", \"FIXME\", \"XXX\", \"HACK\", \"NotImplementedError\", \"placeholder\", \"not implemented\", \"implement me\"]`. Walk me through the root cause."
- **Ground truth:** the pattern-drift explanation — the test's marker is `// pending: stub`, none of the patterns substring-match `pending` or `stub`. The reasoning trace should arrive at "neither 'pending' nor 'stub' appears in the pattern list".
- **Scoring:** does the response reach the correct conclusion? Does the reasoning trace actually walk the substrings rather than handwave? Score on a 1–5 trace-quality scale judged by a 3-agent council (see §judging).
- **Hypothesis:** Pro's reasoning trace will be MORE THOROUGH on this. The conclusion may match between Pro and Flash, but the TRACE quality (the artifact we keep in the per-request log) likely differs.

### Category D — Structured output (JSON / schema adherence)

**D.1: Extract structured data from a real per-request log**
- **Prompt:** "Given this per-request DeepSeek log JSON, extract `{request_id, model, total_completion_tokens, reasoning_tokens, cost_usd}` as strict JSON. No prose, no markdown fences. If a field is absent, set it to null."
- **Input:** an actual JSON log file from `~/.triumvirate/deepseek-logs/`
- **Ground truth:** the field values themselves — deterministic check
- **Scoring:** is the response (a) parseable JSON, (b) every field-value pair matches expected, (c) no extra text. Binary on parseability + correctness.
- **Hypothesis:** both pass; Flash chosen for cost. Per published benchmarks Flash should be fine on extraction.

**D.2: Generate a schema from a Rust struct**
- **Prompt:** "Given this Rust struct definition (with serde attrs), produce the corresponding JSON Schema (draft-07) that the API would accept as the request body. Return ONLY the schema JSON."
- **Input:** `RunRequest` struct from `daemon/crates/mcp-bridge/src/deepseek.rs`
- **Ground truth:** a hand-written schema we verify is correct
- **Scoring:** parseable schema; required fields right; optional fields right; nested object handling right.
- **Hypothesis:** Pro likely wins on this — JSON Schema has subtleties about $defs, refs, etc.

### Category F — Production failure reproductions (drawn from real session symptoms)

These are scenarios we've actually hit in production this session. We test whether each model falls into the trap more or less often.

**F.1: Truncated / tool-tag-only response**
- **Symptom hit 2026-05-26:** active session sent a code-review request to DeepSeek; daemon returned `<triumvirate_tool name="ledger_session">` tag with no actual review content. Lifecycle reported "responded on attempt 1" — daemon thought it succeeded; response was unusable.
- **Test:** send a code-review prompt with a system message that mentions Triumvirate tool tags (`ledger_session`, `lesson_add`, etc. — the daemon's actual tool surface). Measure: how often does the model emit ONLY a tool tag instead of the requested review content?
- **Both models, 5 trials each.** Score: out of 5, how many produced unusable tool-tag-only responses?
- **Hypothesis:** Flash falls into this trap more (per Gemini-search finding on "identifier bleeding" and 96% AA-Omniscience hallucination rate — Flash mimics surface patterns rather than answering).
- **Mitigation candidate:** add a typed `ToolTagOnlyResponse` failure variant in the runner — detect when content is pure `<...>` with no body text. Or filter system prompts that mention tool names without explicit "do not emit these tags in your response" guard.

**F.2: Reasoning trace longer than content**
- **Symptom from probe-08 + B.14:** model emits 200 reasoning tokens, 0 content tokens, finish_reason=length. Caller paid for reasoning that produced no answer.
- **Test:** send prompts with `max_tokens=64` + thinking=enabled. Measure: how often does each model spend the budget on reasoning before producing any content?
- **Both models, 10 trials each.**
- **Hypothesis:** Flash and Pro both fall into this; the question is rate. May warrant a daemon-side warning when ratio reasoning_tokens/completion_tokens > 0.8.

**F.3: Cost overshoot on simple tasks (Pro overkill detector)**
- **Test:** send the SAME trivial task (A.1 rename) through Pro and Flash. Compare total cost.
- **Hypothesis:** Pro's reasoning trace adds 5–10× cost for ZERO quality gain on trivial tasks. This is the data point that would justify Flash-as-default for the rename/edit task class specifically.

### Category G — Retry behavior with idempotency (design + measurement)

Open design question raised this session: **"we need some sort of retry if we get a failed tool call — but we have to ensure we don't accidentally send the same request twice."**

This is two questions:

**G.1: What constitutes "failed" at the daemon level?**
Currently the runner returns typed `DeepSeekFailureKind`. Tests should measure how OFTEN each model produces these "success-but-unusable" outputs (F.1) that don't trip any existing typed failure:
- Pure tool-tag responses
- Single-word responses to multi-line prompts
- Code responses that don't compile
- JSON responses that don't parse

**G.2: Idempotency under retry — can we re-call safely?**
DeepSeek's API does NOT currently expose an `Idempotency-Key` header equivalent (verify in eval). Therefore EVERY retry bills separately. The right shape for safe retry:
- **Don't auto-retry inside the runner** (T-010 already constrains in-flight retries to pre-first-byte + 429-with-Retry-After).
- **Don't auto-retry at the dispatch layer** (T-013 already sets attempt_schedule=1 for deepseek).
- **For caller-driven retry:** the second call SHOULD MUTATE the prompt to escape the attractor that produced the failure (e.g. "Please respond with actual content, not a placeholder or tool tag" prepended to the user message). Otherwise thinking mode (effectively deterministic — ignores temperature) gives same garbage.

**Eval task G.2:** measure retry effectiveness. For a known failure (e.g. F.1 tool-tag-only), retry the same prompt verbatim — does it produce different output? Then retry with the mutation prefix — does THAT produce usable output? Quantify the win.

**Cost concern:** every retry bills. The mutation strategy + caller-driven retry (no daemon auto-retry) keeps the cost contract: 1 ask_agent call = 1 API call, charged exactly once. Caller decides whether to spend on a second attempt.

### Category E — Council-style decisions (the high-stakes path)

**E.1: Architecture trade-off question with 4 options**
- **Prompt:** Identical to the test-plan's `B.X` decisions — give it the same "rank these options with rationale" framing we used with the 3-agent council on the ABE bug. Different topic: pick a backend approach (e.g. SSE parser implementation: regex-split vs windows-scan vs hand-rolled state machine).
- **Ground truth:** No single "right answer" — score on quality of analysis.
- **Scoring:** 3-agent council judges Flash's response vs Pro's response on the same 5 dimensions: technical accuracy, trade-off explicit naming, recommendation strength, code change concreteness, false-positive avoidance.
- **Hypothesis:** Pro materially wins. This is where we'd actually pay for Pro.

**E.2: Spec gap analysis** (paste a 30-line spec, ask "what's missing?")
- **Prompt:** Use an excerpt from the early `deepseek-integration-spec.md` BEFORE the goatrodeo rounds added the round-3 paradox guard, the ghost-success detection, the 10-min server-close handling, etc. Ask "What edge cases or failure modes does this spec miss?"
- **Ground truth:** the LIST of gaps the goatrodeo rounds actually surfaced — we KNOW the real gaps because Codex+Gemini found them
- **Scoring:** how many of the known gaps does the response identify? Out of (say) 8 known gaps, count hits.
- **Hypothesis:** Pro materially wins. This is the goatrodeo workflow.

---

## Execution playbook

### Phase 0 — Setup (~$0; ~5 min)
- Confirm PR `feat/deepseek-per-call-model-override` is merged on main
- Confirm binary rebuilt + `~/.triumvirate/deepseek.key` present
- Smoke-test: `ask_agent({agent:"deepseek", deepseek_model:"deepseek-v4-flash", message:"reply with: ok"})` returns `ok`
- Smoke-test: same with `deepseek-v4-pro`

### Phase 1 — Cheap buckets first (A, B, D) (~$0.10; ~10 min)
Run buckets A.1, A.2, B.1, B.2, D.1, D.2 — 6 tasks × 2 models = 12 consults. These are short (~500–2000 completion tokens). Score programmatically. Operator runs a Python script that:
1. Reads the task prompt + input file
2. Calls `ask_agent` twice (flash, then pro)
3. Captures response + token usage + per-request log path
4. Runs the programmatic check (does the code compile? does the JSON parse? do the assertions pass?)
5. Records to `pro_vs_flash_results.jsonl`

### Phase 2 — Bug-analysis buckets (C) (~$0.10; ~10 min)
C.1, C.2, C.3. Same flow but the scoring needs human/LLM judgment for trace-quality (C.3 especially).
- For C.1 / C.2: programmatic check whether the suggested fix matches the known correct shape
- For C.3: capture Flash's reasoning_content + Pro's reasoning_content; the council scoring happens in Phase 4

### Phase 3 — Council-style decisions (E) (~$0.20; ~10 min)
E.1, E.2. These take ~5K tokens each because the prompts are bigger and the responses are longer.
- E.2 has a known gap list; the scoring is "how many of N hits" (programmatic)
- E.1 needs the 3-agent council to judge — see Phase 4

### Phase 4 — Judgment pass on subjective scoring (~$0.05; ~10 min)
For C.3 (reasoning trace quality) and E.1 (architecture analysis quality), run a 3-agent council via `ask_agent` to Codex + Gemini + (Claude in-session). Each judge gets:
- The original prompt
- Flash's response
- Pro's response  
- The 5-dimension rubric
- Asked to pick a winner with rationale (no ties)

Final score is majority vote across 3 judges. Cost is just 3 council consults total (~$0.05).

### Phase 5 — Synthesize the decision matrix
Build the final table:

| Task | Flash result | Pro result | Winner | Cost ratio | Recommendation |
|---|---|---|---|---|---|
| A.1 | ✅ pass | ✅ pass | tie | Pro 10× cost | **Use Flash** |
| A.2 | ✅ pass | ✅ pass | tie | Pro 8× cost | **Use Flash** |
| B.1 | ✅ pass | ✅ pass | (judge) | Pro 10× cost | TBD |
| ... | | | | | |

---

## Scoring rubric (for subjective dimensions)

For C.3 and E.1 the 3-agent council judges on **5 dimensions, 1–5 each**:

1. **Technical accuracy** — does the response correctly characterize the problem?
2. **Trade-off explicitness** — does the response name the trade-offs the option implies (false-positive risk, perf cost, etc.)?
3. **Recommendation strength** — does the response commit to one answer, or hedge?
4. **Code change concreteness** — exact lines or generic gestures?
5. **False-positive avoidance** — does the response flag risks the prompt's constraints don't cover?

Each judge scores Flash AND Pro on all 5 dimensions. Total: 25-point ceiling per response. Council aggregates by averaging across judges; report the per-dimension mean and the winner.

---

## Cost budget + circuit breakers

| Phase | Per-task | Tasks | Models | Total |
|---|---|---|---|---|
| 1 (cheap) | ~$0.005 | 6 | 2 | ~$0.06 |
| 2 (bug) | ~$0.01 | 3 | 2 | ~$0.06 |
| 3 (council) | ~$0.02 | 2 | 2 | ~$0.08 |
| 4 (judge) | ~$0.015 | 3 | 1 (judges only) | ~$0.05 |
| Retries / buffer | | | | ~$0.25 |
| **Total ceiling** | | | | **~$0.50** |

Hard stop at $0.50 (poll `/user/balance` between phases — easy on the daemon side now that the runner is wired). Half a day's worth of normal DeepSeek usage; bounded enough to be no-questions-asked.

---

## What we'll do with the results

Three concrete outputs:

1. **Update the runbook §3.5** with the operator-side decision matrix.
2. **Maybe** propose a router rule: "if `deepseek_model` is unset AND the task matches one of these patterns (architecture decision, bug hunt, spec review), upgrade to Pro automatically." Or maybe NOT — operator-controlled is cleaner. Decide based on the eval results.
3. **Adjust the default** if Flash beats Pro on enough tasks to merit it. Default is already Flash post-PR-#39; if Pro materially wins on the categories we care about, we may add a `TRIUMVIRATE_DEEPSEEK_DEFAULT_TO_PRO=1` operator opt-in.

---

## What this plan does NOT test

Calling out scope cuts honestly:

- **Long-context** (>200K tokens) — out of scope; we don't currently run loads that big
- **Multi-turn / tool calls** — we're stateless single-turn v1; the `reasoning_content` round-trip rule (Codex's note) doesn't apply
- **Latency under load** — single-shot per-task; concurrency/queueing perf is a separate stress test
- **Streaming UI quality** — we capture full completions, not the per-chunk experience
- **Non-English prompts** — operator workflow is English

These belong in a follow-up if/when they become relevant.

---

## Citations (web research, 2026-05-26 quicksearches)

[^codersera]: codersera.com — DeepSeek V4 Pro vs Flash benchmarks (LiveCodeBench, SWE-bench, Terminal-Bench, SimpleQA gaps): https://codersera.com/blog/deepseek-v4-pro-vs-flash/
[^thesys]: thesys.dev — DeepSeek V4 Pro vs V4 Flash features & benchmarks: https://www.thesys.dev/blogs/deepseek-v4-pro
[^wavespeed]: wavespeed.ai — DeepSeek V4 Pro vs Flash production routing patterns (Flash with bigger thinking budget approaches Pro): https://wavespeed.ai/blog/posts/deepseek-v4-pro-vs-flash/
[^medium-mehul]: medium.com — Pro vs Flash production deployment patterns: https://medium.com/data-science-in-your-pocket/deepseek-v4-pro-vs-deepseek-v4-flash-9e235b74b0d0
[^huggingface-flash]: huggingface.co/deepseek-ai/DeepSeek-V4-Flash — official model card
[^huggingface-pro]: huggingface.co/deepseek-ai/DeepSeek-V4-Pro — official model card

**Independent eval methodology references:**
- StructEval paper (LLM structured-output benchmarking): https://arxiv.org/html/2505.20139v1
- CodEv paper (smaller LLMs as code-review evaluators): https://arxiv.org/pdf/2501.10421
- bytebytego "Guide to LLM Evals": https://blog.bytebytego.com/p/a-guide-to-llm-evals

**Kilo blog ran their own Pro vs Flash test:** https://blog.kilo.ai/p/we-tested-deepseek-v4-pro-and-flash — worth reading for prior-art comparison after we run ours.
