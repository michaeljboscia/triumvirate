# Raw peer output: local-inference-buy-vs-rent.md unit 3 (lines 285-455)

Section 6 "Rent First. Always.", section 7 recommendations, document history, bibliography.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

**Section 6 is the standing policy for the entire corpus.**

---

## CODEX (engineering angle)

**1. No crossover threshold, cost model, or falsification condition in section 6 either.** Closest lines: *"There is
no economic model for buying GPUs for our own use"* (290) and *"not revisited on a price dip"* (291). **That is a
conclusion, not a model.** The 15 tok/s floor (362-369) is a **performance disqualifier, not an economic crossover.**

**2. The policy statement** (381): *"Rent first, always. Owned metal only as a customer-funded terminal step after a
rented pilot and a signed term."* Plus, in section 6: *"There is no economic model... That is settled"* (290-291).

**It is framed as both a standing decision and an economic conclusion. That raises the burden: the document needs an
economic model if it wants the conclusion, or clearer policy language if it only wants a decision rule.**

**3. The order of operations is under-specified** (301-307). *"Sell the outcome"* has no qualification trigger.
*"Pilot on rented GPUs"* has no pass/fail criteria. *"Sign a term long enough to amortize whatever comes next"* (305)
**lacks the amortization math.** The only termination condition is rhetorical: *"If step 4 never arrives, nothing was
lost"* (309).

**4. Section 7 overstates several rows relative to the body:**
- *"Throughput does not justify it at any price near the ask"* (382) is stronger than the evidence supports.
- *"No utilization model justifies it"* (383) is unsupported: section 6 says **no model exists** (290), **not that a
  model was run and failed.**
- *"Every path is priced at peak"* (384) reads broader than the bibliography supports.
- *"Compliance buyers are lost to VPC-hosted frontier models"* (386) is categorical, **while section 6 says "mostly
  lost" (324), which is softer.** The summary is stronger than the body.
- *"Air-gap buyers need a bigger shop"* (386) is asserted from operational burden, not demonstrated.

**5. The TPS floor integrates only partially.** It is **the only falsification-like criterion in the section**
(367-369), but it **arrives late as "rescued" archival material (359-360), after the policy has already been declared
settled (290-294).** Nothing else in 285-455 references it except that subsection and the L4 calibration (371-373).

**6. The bibliography is mostly secondary.** Hardware pricing cites forums, MacRumors, Macworld, Eastern Herald,
TechRepublic, eBay, Hacker News, Thunder Compute, Tom's Hardware, Tech Insider, and one NVIDIA product page (407-431).
Model benchmarks are all secondary or tertiary (435-443). **Forums and aggregators are cited where primary sources
exist**, particularly for Apple pricing and specs and NVIDIA MSRP.

**7. Document History does not show the conclusion changing in response to evidence.** v2 *"records the standing
policy"* and repurposes the gates (398); v1 consolidated prior analysis (399). **It reads as evidence gathered and
consolidated around a fixed policy, not as a tested revision.**

---

## GEMINI (strategic angle)

**1. Section 6 is written as a conclusion forced by the analysis** (287-288, *"policy that comes out of them"*;
290-291). **It should be written as a structural decision the owner made.**

> "Because the analysis lacks a crossover point and relies on inflated data, presenting the policy as a mathematical
> outcome **makes it fragile. A strategic business decision holds regardless of hardware market fluctuations; a
> mathematically forced one collapses when prices drop.**"

**2. On the word "always":** **as a finding it overclaims**, because economic models inherently have crossover points.
**As a policy it is the correct form.** It functions as an operational invariant, a bright line preventing continuous
relitigation of hardware purchases.

**3. Section 7, row by row:**
- Standing policy (381): **owner's judgment.** The analysis lacks the math to support "always."
- $34K Mac Studio (382): **neither.** Based on an inflated second-hand listing.
- RTX PRO 6000 (383): **neither.** Built as a strawman at peak shortage premium.
- Local inference generally (384): owner's judgment; passing through premiums is a business-model choice.
- Sovereign engagements (385): owner's judgment.
- Target buyer (386): owner's judgment; segment behaviors asserted, not proven.
- GCP gates (387): owner's judgment.
- Retainer architecture, positioning, economics (388-390): owner's judgment.

**4. Falsifiable conditions needed.** It asserts *"not at our utilization"* (290) **without defining the metric.** It
needs exact utilization rates, capital cost thresholds, or SLA/latency requirements that would trigger a move.

**5. Action: SPLIT the document.** It binds a permanent, sound business policy (rent first, own the context layer) to
a flawed point-in-time hardware analysis built on bad data. **Extract the policy into a standalone operational
directive. Demote the hardware analysis**, whose math fails to structurally justify the policy it claims to support.

---

## DEEPSEEK (adversarial logic angle)

Asked whether *"not revisited on a price dip"* is legitimate pre-commitment or insulation against disconfirmation.

> **The test:** a pre-commitment is legitimate **iff it names, in advance, what evidence WOULD reopen the decision and
> at what threshold.** If the policy can specify a falsification condition, even a demanding one, it is a commitment.
> **If the set of reopening conditions is empty, or if it specifically carves out the single most probable
> disconfirming observation, it is evidence insulation dressed as discipline.**
>
> **The verdict: insulation.** The sentence *"targets exactly the datum that would test the claim (price dip) and
> declares it inadmissible, while naming no circumstance that would reopen the question. That is not a refusal to
> relitigate; it is a pre-emptive veto on the most likely counterexample."*
>
> **Minimal rewording that keeps the policy's force:**
>
> > "There is no economic model for buying this hardware at this pricing or this utilization, and this is not
> > revisited on a transient price dip, **only on a sustained structural change (price down at least X% for Y
> > consecutive quarters, or utilization at least Z).**"
>
> This preserves the anti-relitigation force against market noise **while converting "never" into a pre-registered,
> testable condition.**

**This is the cleanest resolution available, and it reconciles Gemini's two halves.** "Always" is correct *as a
policy*; what makes it insulation rather than commitment is the absence of any named reopening condition. **Adding
one threshold sentence converts the document from a stance into a pre-commitment without weakening the bright line at
all**, and a pre-commitment with a stated trigger is exactly the discipline `30-DECISION-RULES.md` demands
everywhere else in this corpus.
