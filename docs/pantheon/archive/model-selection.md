# PANTHEON Model Selection — April 2026

> **ARCHIVED 2026-08-26. Do not act on this document.** It is an April 2026 model-landscape snapshot serving a
> hardware purchase that is permanently cancelled. Standing policy is `../POLICY-rent-first.md`.
>
> A three-peer review found: the model table is stale in deployment-relevant ways (a mixture-of-experts model
> described as dense, a context window since doubled, a licence listed as unknown that has been published); the VRAM
> budgets contain **real technical errors**, not just dead premises, because active parameters in a sparse model do
> not mean only active weights need to be resident, and one FP8 figure is off by roughly half; and the "all clean for
> commercial use" licence heading is contradicted by an entry in its own table.
>
> **What was extracted before archiving:** capability-aware endpoint routing, the caveat that search-derived model
> data must be checked against primary model cards, and the framing that model selection is a tuning knob rather than
> an architectural commitment. See `../EXTRACTED-from-archive.md`.

---

**Purpose:** Comprehensive model inventory for the three PANTHEON roles (Zeus/Athena/Vulcan), grounded in Gemini quicksearch results from April 16, 2026. All benchmark numbers, context windows, and licensing terms are search-verified, not from training data.

**Created:** 2026-04-16
**Companion docs:**
- `docs/pantheon/graduated-gcp-validation-plan.md` — GCP test tiers where these models get evaluated
- `/Users/you/PANTHEON_ARCHITECTURE.md` — the $20K hardware blueprint

**Key benchmark context:** HumanEval is considered saturated/contaminated in 2026. The industry gold standards are now **SWE-bench Verified** (single-file patches, 500 human-curated tasks) and **SWE-bench Pro** (multi-file, avg 4.1 files per task). **LiveCodeBench v6** is the primary speed+quality composite for local models.

> **Superseded on models and VRAM math (2026-08-23).** This inventory is search-verified as of April 16, 2026 and has been
> overtaken: GLM-5.2, Kimi K2.6/K2.7/K3, and DeepSeek V4-Pro all shipped after it. Kimi K2.6 at native INT4 is roughly
> 594GB and does not fit 512GB; the GLM-5.2 oQ4 build is 418GB and does not fit 4x RTX PRO 6000. See
> `docs/pantheon/local-inference-buy-vs-rent.md` section 2 for the current landscape and section 3 for measured
> throughput. Note also the arXiv GH200 result cited there: SWE-bench rank did not predict per-task outcomes, so treat
> the tables below as screening data, not selection criteria.

---

## The complete landscape (search-verified April 2026)

### Tier 1 — Frontier generalists (400B+ class, Zeus candidates)

| Model | Total params | Active params | Context | SWE-bench Verified | SWE-bench Pro | License | Best at |
|---|---|---|---|---|---|---|---|
| **Qwen 3.5 397B** | 397B MoE | 17B | 256K→1M | ~76% | ~51% | Apache 2.0 | Instruction following, structured output, agent orchestration |
| **DeepSeek V3.2 Speciale** | 671B MoE | 37B | 160K | ~73% | ~48% | MIT | Deep reasoning, "thinking" mode, catches subtle bugs, IOI Gold |
| **Llama 4 Maverick** | 400B MoE | 17B | 1M→10M | ~70% | ~32% | Llama 4 License (700M MAU cap) | Massive context (entire repo in one prompt), generalist |
| **GLM-5.1** | 754B | ~40B | 200K | ? | **58%** | MIT | Multi-file repo edits, tool-use, agentic. Beats GPT-5.4 on Pro. |
| **Kimi K2.5** | 1T MoE | 32B | 256K→10M | **77%** | **54%** | Modified MIT | Multi-step reasoning, agent swarms, Python-dominant |
| **MiniMax M2.5** | ? (unreported) | ? | ? | **80.2%** | **56%** | ? | Highest open-weight SWE-bench Verified. Process Reward Model self-verification. |
| **DeepSeek V4** | 1T+ MoE | ? | 1M | TBD (April 2026 release) | TBD | Apache 2.0 | Successor to V3.2, native multimodal, 1M context |
| **gpt-oss 120B** | 120B dense | 120B | 128K | ? | ? | Apache 2.0 | OpenAI's first open model. Fits 1× A100 80GB. Strong instruction following. |

### Tier 2 — Mid-size specialist coders (27B-80B, Athena candidates)

| Model | Total params | Active params | Context | HumanEval | LiveCodeBench v6 | License | Best at |
|---|---|---|---|---|---|---|---|
| **Qwen3-Coder 480B** | 480B MoE | 35B | 256K→1M | 92%+ | **87%** | Apache 2.0 | Agentic repo-level editing, TS/React, autonomous bug fixing |
| **Qwen 3.5 32B dense** | 32B | 32B | 256K | 92% | ~65% | Apache 2.0 | Daily coding workhorse, fits single GPU at INT4 |
| **DeepSeek V3.2 (distilled)** | 32B-70B | 32B-37B | 160K | 90%+ | 89.6% | MIT | Python/C++ proficiency, polyglot, cost-efficient |
| **Kimi-Dev-72B** | 72B | 72B | ? | ? | ? | Modified MIT | RL-trained on test suites in Docker — 60.4% SWE-bench Verified |
| **GLM-5 (40B active variant)** | 744B MoE | 40B | 200K | ? | 55% (agentic) | MIT | Go concurrency, backend microservices, #1 LiveBench Agentic |
| **gpt-oss 120B** | 120B | 120B | 128K | strong | ? | Apache 2.0 | Fits A100 80GB, instruction-following parity with proprietary OpenAI |

### Tier 3 — Small/fast models (8B-22B, Vulcan candidates)

| Model | Total params | Active params | Context | LiveCodeBench v6 | Speed (RTX 4090 Q4) | License | Best at |
|---|---|---|---|---|---|---|---|
| **Qwen 3 14B Coder** | 14B | 14B | 256K | 52% (reasoning mode) | ~60-75 t/s | Apache 2.0 | Best overall <15B for code, "thinking" self-correction |
| **Gemma 4 E4B** | 8B | 4.5B | 128K | 52% | **110+ t/s** | Apache 2.0 | Fastest inference, multimodal (images+audio), agentic |
| **Codestral 25.01 / Devstral 2** | 22B | 22B | 256K | ? | fast (sub-100ms FIM) | Apache 2.0 | Fill-in-the-Middle champion, IDE autocomplete |
| **gpt-oss 20B** | 20B | 20B | 128K | ? | ? | Apache 2.0 | Swiss army knife, strong structured output |
| **Phi-4** | 14.7B | 14.7B | **16K** | 23% | ~35-45 t/s | MIT | Algorithmic math. **DISQUALIFIED: 16K context too small.** |
| **StarCoder2 15B** | 15B | 15B | ? | 72.6% HumanEval | ? | BigCode OpenRAIL | Transparent training data, 600+ languages, safest IP position |

### Per-language specialists

| Language | Best model | Why | Source |
|---|---|---|---|
| **Python** | Kimi K2.5 (99% HumanEval) or Qwen3-Coder | Agent-swarm with real-time Docker verification | Gemini search 5 |
| **Rust** | DeepSeek V3.2 Speciale + **Strand-Rust-Coder-v1** (Qwen fine-tune) | Handles borrow checker/lifetimes; Strand = +14% on Rust benchmarks | Gemini search 5 |
| **Go** | GLM-5 (40B active) | #1 LiveBench Agentic Coding (55.0), strong goroutine/channel reasoning | Gemini search 5 |
| **TypeScript** | Qwen3-Coder | Best type-narrowing + async patterns, optimized for React/Next.js | Gemini search 5 |

---

## Recommendations per PANTHEON role

### Zeus — The Architect (Code review, APPROVE/REJECT, PRD generation)

| | Primary | Alternative | GCP test stand-in |
|---|---|---|---|
| **Model** | **Qwen 3.5 397B MoE** (17B active) | DeepSeek V3.2 Speciale (37B active) | Llama 3.1 70B BF16 or gpt-oss 120B |
| **Why** | Highest instruction-following (IFBench). Won't deviate from structured APPROVE/REJECT JSON schema. 76% SWE-bench Verified. Apache 2.0. MoE = fits DGX Spark 128GB easily. | Better at catching subtle logical bugs. "Thinking" tokens simulate execution. IOI Gold Medal logic. But occasionally ignores formatting constraints. | 70B fits 1× A100 80GB. Good enough to validate review-loop protocol. gpt-oss 120B is an alternative if A100 has room. |
| **Quantization** | FP8 on DGX Spark | FP8 | BF16 on A100 80GB |
| **VRAM** | ~50GB active (MoE routes 17B) | ~80GB (37B active, larger KV) | ~140GB |
| **Context** | 256K (expandable to 1M) | 160K | 128K |
| **License** | Apache 2.0 | MIT | Llama 4 License / Apache 2.0 |

**Models to investigate further for Zeus:**
- **MiniMax M2.5** — 80.2% SWE-bench Verified (best open-weight), but parameter count and VRAM requirements are unreported. Need to find sizing data before committing.
- **Kimi K2.5** — 77% Verified, 10M context, but Modified MIT license needs legal review for commercial PANTHEON deployments.
- **GLM-5.1** — 58% SWE-bench **Pro** (multi-file) is extraordinary, but 754B parameters likely needs 8× H100 (640GB). Won't fit DGX Spark's 128GB. Possible via heavy quantization?

### Athena — The Swarm Workers (Parallel code generation across worktrees)

| | Primary | Per-language specialist | GCP test model |
|---|---|---|---|
| **Model** | **Qwen3-Coder 480B MoE** (35B active) | See routing table below | Qwen 2.5 Coder 32B INT4 (proven, available now) |
| **Why** | 87% LiveCodeBench v6 — best open-weight coding score. MoE means it fits like a 35B model. Apache 2.0. Designed for autonomous repo-level editing with execution-driven RL self-correction. | Different languages have different best-in-class models (see below) | Battle-tested, widely available AWQ on HuggingFace, runs on 1× L4 at INT4 |
| **Quantization** | INT4 (AWQ + Marlin kernels) | Varies | INT4 (AWQ) |
| **VRAM per instance** | ~20GB (35B active at INT4) | Varies | ~18GB |
| **Workers per DGX Spark** | ~5-6 concurrent instances (128GB / ~22GB per instance with KV cache) | Mixed models reduces density | N/A (GCP uses 4×L4 per node) |
| **License** | Apache 2.0 | Varies | Apache 2.0 |

**Language-aware routing table (Triumvirate feature):**

```
Triumvirate receives task → inspects primary file extension
  *.rs  → DeepSeek V3.2 Speciale (or Strand-Rust-Coder-v1 if available)
  *.go  → GLM-5 (40B active)
  *.ts  → Qwen3-Coder 480B MoE (default)
  *.tsx → Qwen3-Coder 480B MoE (default)
  *.py  → Qwen3-Coder 480B MoE (default) or Kimi-Dev-72B for test-heavy tasks
  *.sql → Qwen3-Coder 480B MoE (default, best instruction-following for SQL)
  *     → Qwen3-Coder 480B MoE (default fallback)
```

This is a Triumvirate routing-table config, not a hardware change. Each DGX Spark could run 2-3 different models simultaneously at different quantization levels. Triumvirate picks the best one per task based on the language signal in the worktree.

**Decision required: language routing complexity vs v1 simplicity.** For GCP validation (Tiers 1-5), use a single model (Qwen 32B INT4) everywhere. Language routing is a v2 optimization after the swarm pattern itself is proven.

**32B × many workers vs 72B × fewer workers:**

| | 32B × 6 workers per DGX | 72B × 2 workers per DGX |
|---|---|---|
| Aggregate throughput | ~1,860 tok/s (6 × 310) | ~360 tok/s (2 × 180) |
| Code quality per task | Good (92% HumanEval) | Better (94%+ HumanEval) |
| Estimated rejection rate by Zeus | ~20-30% | ~10-15% |
| Net effective throughput | ~1,300 good tok/s | ~310 good tok/s |
| **Verdict** | **4× higher net throughput** | Higher per-task quality but parallelism-starved |

**Recommendation: 32B × many workers for v1.** Zeus catches quality failures. The whole point of Athena is throughput. If a specific task requires higher reasoning depth (e.g., database migration, security-sensitive code), Triumvirate can escalate it to Zeus directly — don't slow down the entire swarm for edge cases.

### Vulcan — The Forger (Syntax fixer, UI assets, instant unblocking)

| | Primary | Alternative | GCP test model |
|---|---|---|---|
| **Model (code fixing)** | **Gemma 4 E4B** (8B, 4.5B active) | Codestral 25.01 / Devstral 2 (22B) | Qwen 2.5 Coder 14B INT4 |
| **Why** | **110+ t/s** on RTX 4090. Sub-second response for a typical 5-line fix. Multimodal (can look at UI screenshots for visual debugging). Apache 2.0. | FIM champion — best at "fill-in-the-middle" patch generation. Sub-100ms for autocomplete-style fixes. 256K context. | Fits on 1× L4 (24GB). Known quantity. |
| **Quantization** | FP8 (native on Ada/3090) | FP8 or INT4 | INT4 |
| **VRAM** | ~5GB (4.5B active at FP8) | ~12GB (22B at INT4) | ~8GB |
| **License** | Apache 2.0 | Apache 2.0 | Apache 2.0 |
| **Model (diffusion)** | **Flux.1 Dev** | SDXL Turbo | (skip for GCP validation) |
| **Why** | Best open diffusion for UI asset generation | Faster but lower quality | Not needed for architecture validation |
| **VRAM** | ~12GB (BF16) | ~6GB | — |
| **GPU** | RTX 3090 GPU 1 (separate from code-fixing GPU 0) | Same | — |

**Vulcan's two GPUs serve different roles:**
- GPU 0: Gemma 4 E4B or Codestral — code fixing at maximum speed
- GPU 1: Flux.1 Dev — UI/UX asset generation on demand

**Why Gemma 4 E4B over Qwen 3 14B for Vulcan:** Vulcan's job is SPEED, not depth. It sees a 10-line error message + 20 lines of surrounding code and returns a 5-line patch. At 110+ t/s vs 60-75 t/s, Gemma produces the patch in half the time. Vulcan doesn't need to "understand the codebase" — it needs to fix the specific syntax error and hand control back to the Athena worker. If Gemma's fix is wrong, the task escalates to Zeus (the slow-but-smart path). The failure cost is low; the speed gain is high.

---

## VRAM budget on production hardware

### DGX Spark (Athena) — 128GB unified memory per unit, 2 units = 256GB total

**Option A: Single-model swarm (simplest)**
```
Model: Qwen3-Coder 480B MoE (35B active) at INT4
VRAM per instance: ~20GB (weights) + ~2GB (KV cache at 8K context) = ~22GB
Instances per DGX: floor(128 / 22) = 5 instances
Total across 2 DGX: 10 concurrent workers
```

**Option B: Mixed-model swarm (language-routed)**
```
DGX-1 (128GB):
  2× Qwen3-Coder 480B MoE INT4 = ~44GB  (TypeScript, Python, SQL)
  1× DeepSeek V3.2 32B INT4 = ~20GB     (Rust)
  1× GLM-5 40B INT4 = ~22GB             (Go)
  Total: ~86GB used, ~42GB headroom

DGX-2 (128GB):
  3× Qwen3-Coder 480B MoE INT4 = ~66GB  (default workers)
  1× Kimi-Dev-72B INT4 = ~40GB          (test-heavy Python)
  Total: ~106GB used, ~22GB headroom
```

### RTX 3090 × 2 (Vulcan) — 24GB VRAM per GPU

```
GPU 0: Gemma 4 E4B FP8 = ~5GB → 19GB free for KV cache (massive context possible)
GPU 1: Flux.1 Dev BF16 = ~12GB → 12GB free
```

### Mac Studio M5 Ultra (Zeus) — 512GB unified memory

```
Model: Qwen 3.5 397B MoE FP8
Full model weight: ~200GB (397B at FP8, but MoE routing means only ~50GB active at inference)
KV cache at 256K context: ~30-60GB
Total active: ~260GB → 252GB headroom
Alternative: load TWO models (Qwen 3.5 for review + DeepSeek V3.2 for deep analysis)
```

---

## Licensing summary — all clean for commercial use

| Model | License | Commercial OK? | Restriction |
|---|---|---|---|
| Qwen 3.5 / Qwen3-Coder | Apache 2.0 | ✅ Unrestricted | None |
| DeepSeek V3.2 / V4 | MIT / Apache 2.0 | ✅ Unrestricted | None |
| Gemma 4 | Apache 2.0 | ✅ Unrestricted | None |
| Codestral 25.01 / Devstral 2 | Apache 2.0 | ✅ Unrestricted | None |
| gpt-oss 120B / 20B | Apache 2.0 | ✅ Unrestricted | None |
| GLM-5 / GLM-5.1 | MIT | ✅ Unrestricted | None |
| StarCoder2 | BigCode OpenRAIL | ✅ With attribution | Must credit BigCode |
| Llama 4 Maverick | Llama 4 Community License | ⚠️ 700M MAU cap | Must request license above 700M monthly users |
| Kimi K2.5 / Kimi-Dev-72B | Modified MIT | ⚠️ Review needed | Unclear revenue restrictions — verify before production |
| MiniMax M2.5 | ? (unreported) | ❓ Unknown | Cannot commit without license verification |

**For PANTHEON production:** stick to Apache 2.0 / MIT models (Qwen, DeepSeek, Gemma, GLM, gpt-oss). Use Llama and Kimi only for GCP testing where the license restrictions don't apply.

---

## GCP validation model progression (per graduated-gcp-validation-plan.md)

| Tier | Zeus model | Athena model | Vulcan model | Notes |
|---|---|---|---|---|
| **1** (CPU smoke) | — | TinyLlama 1.1B | — | Prove HTTP contract only |
| **2** (first GPU) | — | Qwen 2.5 Coder 14B INT4 | — | Proves GPU + real inference |
| **3** (swarm) | — | Qwen 2.5 Coder 32B INT4 (TP=4 on 4×L4) | — | Core swarm thesis test |
| **4** (full trinity) | Llama 3.1 70B BF16 on A100 | Same as Tier 3 | Qwen 2.5 Coder 14B INT4 on L4 | Review + fix loops |
| **5** (full blast) | Same as Tier 4 | 2× g2-standard-48, 4 workers | Same as Tier 4 | 1-hour scale test |

**After GCP validation passes:** swap to production models (Qwen 3.5 397B for Zeus, Qwen3-Coder 480B for Athena, Gemma 4 E4B for Vulcan) on the physical hardware.

---

## Key architectural insight: language-aware worker routing

The search results confirm that **different models are best at different programming languages.** This creates a natural Triumvirate routing feature:

```rust
// Pseudocode for Triumvirate worker dispatch
fn select_model(task: &Task) -> ModelEndpoint {
    match task.primary_language() {
        Language::Rust => endpoints.deepseek_v3_2,     // Best borrow-checker handling
        Language::Go   => endpoints.glm_5,             // Best goroutine/channel reasoning
        Language::TypeScript | Language::TSX => endpoints.qwen3_coder, // Best type-narrowing
        Language::Python => endpoints.qwen3_coder,     // Default; swap to kimi-dev for test-heavy
        Language::SQL   => endpoints.qwen3_coder,      // Best instruction-following for DDL
        _ => endpoints.qwen3_coder,                    // Fallback
    }
}
```

**v1 simplification:** use a single model for all languages (Qwen3-Coder 480B MoE). Language routing is a v2 optimization after the swarm pattern is proven on GCP. Don't add routing complexity before the basic architecture works.

**v2 optimization:** run 2-3 different models per DGX Spark, route by language. Potentially 10-20% quality improvement on Rust and Go tasks at the cost of reduced worker density (fewer instances of each model fit in 128GB).

---

## Open questions (resolve during GCP testing)

1. **Qwen3-Coder 480B MoE on vLLM** — does vLLM handle the MoE routing efficiently on 4× L4? The 35B active parameter count suggests yes, but MoE KV cache sizing may differ from dense models. Test at Tier 2 by deploying alongside the 32B dense and comparing throughput.

2. **MiniMax M2.5 sizing** — parameter count and VRAM requirements are unreported as of April 2026. If it's a dense 70B model, it won't fit single-GPU at INT4. If it's MoE with <40B active, it could replace Qwen3-Coder as the Athena workhorse. Monitor HuggingFace for the model card.

3. **DeepSeek V4 readiness** — released April 2026 but benchmarks are incomplete. If it significantly outperforms V3.2, it becomes the Zeus primary or Athena specialist for reasoning-heavy tasks. Monitor.

4. **Strand-Rust-Coder-v1 availability** — confirmed as a Qwen fine-tune with +14% on Rust benchmarks, but unclear if weights are publicly available on HuggingFace. Search returned a mention but no direct link.

5. **"Thinking" mode latency trade-off** — Gemma 4 E4B drops from 110+ t/s to 10-15 t/s in "thinking" mode. For Vulcan, thinking mode should be OFF (speed > depth). For Athena, thinking mode may be worth the slowdown for complex tasks. Make this a per-task Triumvirate config flag.

6. **Speculative decoding** — Gemma 4 claims +50% speedup with its 4B draft model. If vLLM supports speculative decoding with Gemma 4 as the target and its own E4B as the draft, Vulcan's throughput could approach 150+ t/s. Test at Tier 2.

---

## Sources

All data in this document was gathered via 10 Gemini quicksearches on April 16, 2026. Key sources cited by Gemini:
- HuggingFace model cards and leaderboards
- morphllm.com (SWE-bench analysis)
- awesomeagents.ai (agent benchmark aggregation)
- benchlm.ai (LLM benchmark database)
- llmbase.ai (small model comparisons)
- hamel.dev (vLLM performance analysis)
- galaxy.ai (model comparison)
- deepseek.com (official specs)
- cloudvyn.com, particula.tech (model landscape reviews)

**Caveat:** Gemini search results can conflate model versions, hallucinate benchmark numbers, or present preliminary data as final. Before committing to any model for production PANTHEON deployment, verify benchmark claims against the official model card on HuggingFace and run our own evaluation on the GCP test infrastructure.

---

*To use: read this alongside `graduated-gcp-validation-plan.md`. Start with the GCP test models (proven, conservative), graduate to production models after validation passes. Model selection is a tuning knob, not a one-time decision — revisit quarterly as new models release.*
