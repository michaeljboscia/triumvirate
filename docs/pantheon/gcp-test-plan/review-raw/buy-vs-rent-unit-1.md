# Raw peer output: local-inference-buy-vs-rent.md unit 1 (lines 1-130)

Section 1 "The Trigger: A $34,000 Facebook Marketplace Listing", section 2 "Published Benchmarks".

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

**This is the one document in the corpus whose conclusions are currently load-bearing.**

---

## CODEX (engineering angle): fact check

- **Line 66 STALE/WRONG.** M5 Ultra is no longer "expected around October 2026"; it was **announced 2026-08-25.** Mac Studio M5 Ultra starts around **$5,499**, 512GB comes **late October**, 256GB pricing roughly **$9,499-$10,799** depending on config.
- **Lines 16-21 UNVERIFIABLE.** The Facebook Marketplace listing is not cited. Config is plausible for M3 Ultra; the specific $34,000 listing cannot be checked.
- **Lines 25-35 MOSTLY VERIFIED, weakly sourced.** Retail math is internally consistent and matches reported maxed-out $14,099 M3 Ultra pricing, but a MacRumors forum is not an ideal source for Apple retail config pricing.
- **Line 39 STALE in present-tense framing.** "Cannot be bought new" was true for the **M3 Ultra 512GB SKU**, but M5 Ultra restored a 512GB path shipping late October. Should name the specific SKU.
- **Lines 45-48 UNVERIFIABLE / LOW CONFIDENCE.** eBay asking prices and forum advice are caveated (good) but do not support valuation precision.
- **Lines 54-58 STALE/OVERPRECISE.** The `$23,000-$26,000` and `$27,000+` offer guidance depends on scarcity that changed materially once M5 Ultra 512GB was announced.
- **Lines 76-87 PARTLY VERIFIED, weak provenance.** GLM-5.2 numbers corroborated, **but the table cites secondary coverage rather than the primary eval, and the methodology is vendor-run.**
- **Line 83 UNVERIFIABLE from cited sources.** "Artificial Analysis Intelligence Index 51, top open-weight, 4th overall" needs a direct citation and a snapshot date.
- **Lines 91-100 PARTLY VERIFIED.** K2.6 SWE-Bench Pro figures corroborated, **but "Modified MIT license" and "594GB INT4 QAT" are not supported by the cited source.**
- **Line 104 UNVERIFIABLE / WEAKLY SOURCED.** K2.7 numbers come from secondary summaries and proprietary suites. The document does correctly caveat the absence of third-party results.
- **Lines 108-110 STALE.** Kimi K3 weights are now on Hugging Face.
- **Lines 114-123 PARTLY VERIFIED, line 118 STALE.** "Open-weight leader, tied with Gemini 3.1 Pro" is out of date; current snapshots show different ordering.
- **Line 127 STALE.** The cited frontier reference points no longer match the current snapshot.

**Net:** the largest defect is section 1's hardware premise. **The $34,000 M3 Ultra comparison is no longer the right anchor after the M5 Ultra announcement.** Section 2 is usable only as a dated snapshot, and only if every benchmark names its provenance: vendor-run, third-party harness, proprietary suite, or unknown. **Several numbers are stated with more confidence and precision than their sources support.**

---

## GEMINI (strategic angle): motivated reasoning

**1. The conclusion selects the analysis.** Rather than evaluating a standard capital expenditure, the document builds its case from **an inflated $34,000 second-hand listing** (14). By fixating on "uncapped scarcity tax" (56) and scammer risk (62), **it conflates buying hardware with making a desperate, foolish transaction.** The analysis is arranged to make a purchase look emotionally and financially reckless **while ignoring normal procurement avenues.**

**2. The benchmark comparison does not compare like with like.** It props up open-weight models using **self-reported vendor evaluations** (GLM-5.2 at 87, Kimi K2.7 at 104) while comparing them against **independent third-party** metrics for frontier models (127-128).

**Most egregiously: it uses Kimi K2.6 to justify a 512GB machine, then admits at line 100 that the model requires 594GB and cannot run on the hardware being analyzed.**

**3. The anchor is obsolete and was always distorting.** Anchoring on a scalper's listing maximizes "timing risk" (66) and lets the document compute depreciation from an inflated $26-34K baseline **rather than from retail.** With M5 Ultra 512GB now real, **the true capital cost is a standard MSRP, which collapses the exaggerated scarcity and depreciation framing.**

**4. No falsifiable conditions. None.** A legitimate buy-versus-rent analysis defines a crossover ("if inference costs exceed $X/month, capex becomes optimal"). This never does. **Without a usage-based break-even, "rent first, always" is an ideological stance rather than an economic finding.**

**5. The strongest unaddressed argument: zero-marginal-cost agentic loops.** A permanent rent policy enforces perpetual per-token metering, **which penalizes continuous high-volume agentic workflows, long-context retrieval, and speculative execution.** Local hardware removes per-token anxiety and network latency. **The document's focus on capital risk entirely ignores the operational chilling effect of metered inference.**

---

## DEEPSEEK (adversarial logic angle)

Asked whether "no crossover threshold" makes this a stance rather than a finding, distinguishing "none exists" from "none was computed."

> **The criticism is correct**, for one reason: **the document makes a universal economic claim while containing no
> economic content that could support it.**
>
> **(a) No crossover because none exists** would be a legitimate but extraordinary finding: that for every plausible
> input (usage, utilization, lifespan, electricity, labor, discount rate, residual value) renting strictly dominates.
> **To be a finding it must be derived, not asserted.**
>
> **(b) No crossover because it was never computed** is the likely reality. **The absence of a crossover is not
> evidence that none exists; it is evidence the question was never asked.** Triggering a universal policy from one
> overpriced second-hand listing is generalization from an anecdote.
>
> **How a reader tells them apart:** Does it contain a cost model comparing rent and buy as functions of the relevant
> variables, or does it narrate why one listing was bad? Does it define what counts as plausible usage? **Can any
> condition under which buying would be correct even be imagined from the text?**
>
> **Minimum to be (a) rather than (b):** an explicit cost comparison formula (capex, opex, depreciation, utilization,
> time horizon, discount rate, opportunity cost); a justified parameter space; a demonstration that the buy curve
> exceeds the rent curve throughout it; and **a falsification condition: "if utilization exceeded X, or rental prices
> exceeded Y, buying would be preferable."**
>
> **Without those, the document is an ideological stance in economic clothing.**

---

## REVIEWER NOTE (important, and it changes how this finding should be used)

**This does not mean rent-first is wrong.** The owner has independently affirmed the policy on his own reasoning:
no purchase until a customer needs the iron, the qualifying buyer is vanishingly rare, and renting is likely the
primary consumption model indefinitely. **That judgment stands on its own and does not depend on this document.**

**What the finding means is narrower and still important:** the document cannot *support* the policy as an economic
result, so it cannot tell you when to revisit it. That gap became concrete tonight when the owner asked whether to
lease a 512GB machine and **the document offered no framework to answer**, because it contains no crossover, no cost
model, and no falsification condition.

**The fix is not to change the conclusion. It is to supply the missing economics so the conclusion becomes checkable**,
and to separate the policy (a decision the owner has made) from the analysis (which currently does not carry it).
