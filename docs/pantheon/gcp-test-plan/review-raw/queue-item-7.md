# Raw peer output: queue item 7 (model-selection.md, graduated-gcp-validation-plan.md)

**Date:** 2026-08-26 · **Peers:** Codex (model-selection), Gemini (graduated plan), DeepSeek (adversarial logic)

**Both peers independently recommended ARCHIVE WITH EXTRACTION.**

---

## CODEX on model-selection.md (284 lines)

**Recommendation: archive-with-extraction.** Not worth rewriting as a living document. Its model table is a dated snapshot and its operating premise is dead.

**Extract exactly three things:**
1. The caveat that search-derived model data can conflate versions and must be verified against primary model cards before production (267-280).
2. The substrate-independent routing idea: language-aware dispatch as a later optimization (227-247).
3. The "model selection is a tuning knob" sentence (284).

**Model landscape (21-68): stale but not fiction.** Spot-check sample:
- Qwen3.5-397B-A17B exists, Apache 2.0, though HF lists ~403B not 397B.
- DeepSeek V3.2/Speciale exists, but line 28 says 160K context where current references show 131,072.
- Qwen3-Coder-480B-A35B exists.
- GLM-5.1 exists but is superseded by 5.2, which the doc itself flags.
- **MiniMax M2.5 line 32 "parameter count/license unknown" is obsolete**: 229B total / 10B active, Modified MIT.
- Gemma 4 E4B line 52's 128K context is stale; now up to 256K.
- **gpt-oss 120B described as "dense" at line 34 is wrong**; it is MoE/MXFP4.

*"Enough entries still exist, but many details are stale, superseded, or wrong in deployment-relevant ways. Treat lines 21-68 as a historical watchlist, not an inventory."*

**VRAM budget (149-193) has real technical errors, not just stale premises.**
Assumes 2x DGX Spark (128GB each), 2x RTX 3090, and a 512GB Mac Studio. Beyond being obsolete:
- **The premise is wrong: MoE active params do not mean only active weights need VRAM.** Full weights must generally be resident or offloaded, so line 156's "480B MoE at INT4 = ~20GB" is not a valid full-model budget.
- **Line 186 is arithmetically wrong:** ~397B params at FP8 is roughly ~397GB before overhead, **not ~200GB**, so the ~260GB total at line 188 does not follow.
- Internal sums are otherwise correct.

**Licensing (194-212): the header "all clean for commercial use" overstates.** Line 206 itself says review needed for one model, contradicting the heading, and line 207's "unknown" is now stale. **Asserted via search summary, not auditable verification**; no primary-source citation per model.

**GCP progression (213-226) assumes the demoted gates** and says to swap to owned hardware after validation passes.

**Language-aware routing (227-250): keep the insight, not the mapping.** The idea is substrate-independent, but line 247 ties it to "2-3 models per DGX Spark." Extract as **capability-aware endpoint routing**, models resolved by current benchmark and availability.

**Open questions (251-266):** MiniMax sizing is resolved; the L4 tier question is moot under the sizing sweep; thinking latency and speculative decoding remain valid as generic concepts.

---

## GEMINI on graduated-gcp-validation-plan.md (442 lines)

**1. "Production hardware topology" (36-89)** mapped AI roles to physical machines for a cancelled $20K purchase. *Problem dead.* Replaced by the priced sizing sweep and pilot operations, **though the logical software roles remain.**

**2. THE FINDING: the validation tiers (90-340) carry value the sizing sweep does not.**

> "A sweep measures cost/performance; **these tiers are a software integration test plan.** They validate whether
> Triumvirate can actually orchestrate vLLM via HTTP (130-168), manage parallel Git worktrees without collision
> (205-258), and successfully execute the Zeus/Vulcan review/fix loops (260-302)."

**3. Gemini Ultra credit (390-395)** claims validation is "effectively free" because the subscription "returns ~$100/mo in GCP credit." **This commits exactly the defect the corpus review identified: inferring infrastructure budget from a consumer subscription state rather than verifying a live billing balance.**

**4. PASS/FAIL (396-417) overclaim badly.** A PASS asserts the $20K purchase is "de-risked" and assumes local metal will be "faster/cheaper" (405). **A FAIL binds software integration bugs to hardware viability: if parallel git worktrees fail to merge, the conclusion is "don't buy hardware" (412).**

**5. Archive with extraction.** Extract the **software integration success criteria** from Tiers 1-4 (vLLM dispatch, worktree isolation, review loops) into a dedicated integration test plan. Archive the hardware topology, pricing math, and purchasing conclusions.

---

## DEEPSEEK on the category error

Asked to name the error in binding a software integration result to a hardware procurement decision.

> The precise error is **affirming the consequent**, compounded by a **category/construct-validity error.**
>
> Implicit reasoning: *"If the hardware were inadequate, the software-integration test would fail; the test failed;
> therefore the hardware is inadequate."* That is invalid: **a git worktree merge failure can be caused by
> orchestration bugs, configuration, race conditions, or a bad test, not just by hardware.**
>
> **Harm to the hardware decision:** the failure signal is contaminated by software causes, so you may wrongly veto a
> purchase that is adequate, or wrongly approve one because software tests happened to pass.
>
> **Harm to the software decision:** the test now carries a procurement consequence, **so engineers will distort it,
> tuning thresholds, hiding failures, or optimizing for clean merges instead of real integration quality.** Both
> decisions get a noisier, corrupted signal.

**The second harm is the one that matters here and neither other peer named it.** Attaching a consequential decision
to a test changes how the test gets run. That is a general hazard worth carrying into the rewritten corpus: **a gate
that decides something expensive will be gamed, however unconsciously, unless the thing it decides is genuinely what
it measures.**
