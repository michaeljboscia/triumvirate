# Raw peer output: 30-DECISION-RULES.md unit 3 (lines 247-350)

Decision 9, Decision 10, Rule application log, Amendment protocol, What this document enables.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**Decision 10 is the major failure.** Trigger: "Monthly, reviewed as part of financial snapshot" (281). Thresholds:
- "Monthly GCP spend on that workload class > $1000 for 2 consecutive months" (288)
- "Workload is consistent (not a one-time burst)" (289)
- "RTX Pro 6000 class: > 150 hrs/mo" (291)
- "A100 80GB class: > 230 hrs/mo" (292)
- "H100 class: > 360 hrs/mo" (293)

**The break-even math is only partially salvageable.** It assumes purchase price, usable life, utilization, cloud hourly price, power/support/ops overhead, and workload equivalence between local and GCP hardware. **Those assumptions are not stated, so the thresholds are not auditable as written.** The concept survives translation to on-demand vs reserved only if rewritten around the reserved-instance commitment delta: compare on-demand cost at observed hours against reserved/committed-use cost plus lock-in risk. **It does not justify CAPEX under the standing rent-first policy.**

**Decision 9 mechanically does NOT fire** just because a 512GB M5 Ultra exists. The 256GB purchase requires all four predicates at 256-259, including a paid engagement, friction log, and macOS-native workflow share. The 512GB purchase requires either Sovereign/405B justification or 256GB being unavailable (261). **Since 256GB exists, the fallback at 270-272 is obsolete. Correct outcome: do not buy unless live evidence satisfies the non-Apple predicates.** The "~$12K" 512GB assumption is also stale.

**Rule application log is empty.** Lines 310-325 are a sample object, not an actual application. Purpose was to bind each rule execution to evidence, observed state, verdict, confidence, and next actions (308).

**Amendment protocol is directionally right but incomplete.** It prevents silent edits and retroactive application (332), but **lacks immutable before/after text, amendment author, evidence IDs, rationale owner, an explicit replacement rule, and a required statement of what the old rule was for, whether the problem persists, and what replaces it.**

**"What this document enables" overclaims.** Line 344 is false for this range because the sample log is not populated. Line 346 is aspirational unless the log is maintained. Line 347 is partly true only if rules remain valid. **Line 348 is false/stale because it cites "Rule 2" and an RTX Pro 6000 CAPEX trigger that is cut.** Line 350 is acceptable as intent, not as current fact.

**No threshold in 247-350 directly conflicts with the 15 tok/s/stream floor.** The real conflict is Decision 10's financial trigger: **it can force CAPEX without any production-performance gate at all.**

---

## GEMINI (strategic angle)

**1. Decision 10 (287-293).**
- *What it was for:* gating CapEx purchases by using a high OpEx threshold as a stop-and-think mechanism.
- *Does the problem persist?* Yes. Budget bleed is unguarded, and **the current rule actively subverts rent-first by using high rent as an excuse to buy hardware.**
- *What replaces it:* **the inversion works perfectly and is not too clever.** Rewrite 287-293 so that $1000/mo sustained spend triggers **a hard stop to justify continuing OpEx** (optimize architecture, commit to reserved instances), not an automatic pivot to CapEx. **Renting is the destination.**

**2. Decision 9 (247-272).**
- *What it was for:* trigger for purchasing Apple silicon for Mac-native workflows based on WWDC availability.
- *Does the problem persist?* Yes, Mac-native workflows need hardware, but a blind ~$12K CapEx buy violates rent-first.
- *What replaces it:* since AWS provides Mac M3 Ultra metal, **replace the acquisition triggers (255-264) with a mandate to rent first.** CapEx unlocks only if rental proves structurally unviable due to latency or specific MLX bottlenecks, regardless of the M5 Ultra announcement.

**3. Amendment protocol (330-341).**
- *Does the problem persist?* Yes. **This protocol is far too easy to satisfy. Journaling a rationalization (334-335) does not stop motivated reasoning; it just records it.**
- *What replaces it:* real friction requires a structural barrier. **A mandatory 72-hour cooldown before any amendment takes effect, or a hard cost/peer review.**

**4. Rule application log (306-329).** *What it was for:* a machine-readable auditable trail. **An empty log after four months proves the mechanism was a bureaucratic fantasy. If the tool is too heavy to pick up, it gets bypassed entirely.** Replace the manual JSON requirement with a ledger entry or a git commit message.

**5. "What this document enables" (342-350).** Aspirational fiction. It asserts an "auditable decision trail" (346) that does not exist, and "clean business defensibility" (348) resting on a rule that explicitly violates core policy. **Replace with a single directive: this document enforces rent-first constraints, and any deviation requires structural friction rather than written excuses.**

---

## DEEPSEEK (adversarial logic angle)

Asked: does a written amendment log prevent goalpost-moving, or merely record it?

> **Neither is globally right.** (A) works for people with enough integrity that seeing a bad excuse in writing shames
> them into honesty, **but that is a personality trait, not a mechanism.** (B) is right whenever **the same person
> controls the threshold, the reasoning, and the application, and faces no external cost.**
>
> The binding addition is not a better journal; it is a **structural separation of powers: an independent reviewer who
> must approve any threshold change and who automatically invalidates its application to prior evidence.** Or,
> minimally: **the amended rule applies only to evidence collected after the change, enforced by someone other than
> the decision-maker.**
>
> **A log records drift; it does not stop it. Only a veto from outside the motivated mind can do that.**

**Note Gemini refined her own position between units.** In unit 1 she argued the friction of writing down a bad excuse
is what breaks the motivated-reasoning cycle. Here she reverses: journaling records a rationalization without
preventing it, and real friction needs a structural barrier. **DeepSeek explains why both of her positions were
partly right:** it depends entirely on whether one person controls threshold, reasoning, and application. For a solo
operator, which is the actual situation, condition (B) holds and the log is documentary rather than binding.

**Practical consequence for the rewrite:** a solo operator cannot supply his own external veto. The realistic
substitutes are the cooldown (Gemini) and the after-the-change-only scope rule (DeepSeek), **plus the one genuinely
external reviewer already available: the peer agents.** An amendment that must be reviewed by Codex or Gemini before
taking effect is a real separation of powers, and it costs almost nothing.
