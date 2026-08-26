# Raw peer output: 30-DECISION-RULES.md unit 1 (lines 1-127)

Framing sections, plus Decisions 1, 2, 3 (the hardware purchase triggers).

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**Framing: sound machinery.** Lines 10-12 separate pre-commitment, mechanical application, fallback for ambiguity, and amendment instead of reinterpretation. Lines 18-24 cleanly separate `Trigger`, `Evidence source`, `Rule`, `Fallback`, and `Amendment log`. The amendment protocol is minimal but valid: dated changes, no silent edits, never deletions.

**Decision 1.** For: choosing no local GPU vs 1x 3090 vs 2x 3090 NVLink. Problem still exists as **capacity planning, not purchase planning.** Replacement: rental tier selection and rental continuation/escalation rules.
- Trigger: funds available plus Gate 1 + 2 evidence (line 31). Thresholds: `>= 10 tok/s` single-stream, `>= 5 tok/s per stream` 4-way, contention `< 2.0`, LoRA `<= 3 hrs`, friction `>= 5 events/week` for 48GB (lines 37-42).
- Measurements still worth taking for a rented config: **yes.** Throughput, contention, OOM-free LoRA completion, friction events. Purchase price and "funds available" do not transfer.
- **CONFLICT: line 39's `>= 5 tok/s per stream` under 4-way batched load directly conflicts with the standing production floor of 15 tok/s/stream.**

**Decision 2.** For: whether to buy RTX Pro 6000. Problem persists as **whether a higher rental class is justified.**
- Triggers (any fires): GCP G4 `>$1000/mo for 2 consecutive months`, customer line-item, training bottleneck `3+ consecutive weeks`, on-prem production-speed requirement (68-72). Rule thresholds: `>= 50 tok/s` 72B single-stream, contention `< 1.5x`, LoRA `<= 2 hrs`, swarm `>= 6/8 pass` (78-83).
- No direct 4-way threshold appears, so no conflict with the floor, but the coverage is incomplete.

**Decision 3.** For: scaling from one owned card to two. Problem persists as **rental scale-out / reservation sizing.**
- Triggers: first card `>= 60 days` plus training blocks serving `3+ consecutive weeks`, customer contention, funded deal, or utilization `> 80% sustained for 4+ weeks` (100-104). Rule: sustained utilization `> 70% for 30+ days` plus measurable conflicts (110-113).
- Utilization time series and scheduling-conflict measurement remain load-bearing. Chassis choice is dead.

**Salvage before removing (quoted thresholds):**
- `4-way batched >= 5 tok/s per stream` (39), keep **only** to document its supersession by 15 tok/s/stream.
- `concurrent hosting contention factors both < 2.0` (40)
- `32B LoRA completes in <= 3 hrs without OOM` (41)
- `>= 5 events/week where specifically the 48GB tier mattered` (42)
- GCP spend `>$1000/mo for 2 consecutive months on consistent workload` (69)
- `72B single-stream >= 50 tok/s` (79)
- `three-model concurrent hosting works`, contention `< 1.5x` (80)
- `32B LoRA single-card completes in <= 2 hrs` (81)
- `canonical 8-task agent swarm hits >= 6/8 pass` (82)
- utilization `> 80% sustained for 4+ weeks` (104)
- sustained utilization `> 70% for 30+ days` (112)
- measurable training + serving scheduling conflicts (113)

**Fallbacks.** Lines 55-59 and 89-91 handle inconclusive evidence by forcing one rerun, then choosing the more reversible / non-purchase path. **Good design property. Keep it.** Line 119 is mislabelled: it is a utilization failure branch, not an inconclusive branch.

**Amendment logs.** Populated, each with only `2026-04-18 initial commitment` (61-62, 93-94, 123-124). Purpose: preserve dated rule history and prevent post-evidence edits being disguised as original intent.

**Contradictions / errors:**
- Line 39's `5 tok/s per stream` vs the standing floor of `15`.
- Line 119 labelled `Fallback` but handles negative rather than inconclusive evidence.
- **Decision 3 utilization inconsistency: the trigger requires `> 80% sustained for 4+ weeks` (104) but the buy rule requires only `> 70% for 30+ days` (112). The rule threshold is weaker than the trigger threshold.**
- Dead-track assumptions embedded at 37, 44, 78, 110, 115-117.

---

## GEMINI (strategic angle)

**1. The framing (8-28): sound and mandatory. Not ceremony.** It encodes intellectual honesty (10-12). *"You built this framework to stop yourself from lying to yourself about evidence. Keep it intact."*

**2. Mapping Decisions 1-3 onto rent-first FAILS.** It forces a shape that no longer fits. Lines 29-127 were designed to gate irreversible $15K capital expenditure. **Applying 4-week friction logs (42) and multi-week bottleneck tracking (71) to a $3/hour highly-reversible rental decision is bureaucratic theater.**

**3. Replacements** (with the owner rule applied: what it was for, whether the problem persists, what replaces it).
(a) They existed to prevent catastrophic, irreversible hardware purchases driven by emotion. (b) The CapEx problem is dead, but **the risk of unjustified resource burn (now time and OpEx) remains.** (c) Three new pre-committed decisions:
- **Decision 1: Track A exit (local isolation).** Trigger: initial local loop configured. Threshold: 8-task canonical swarm achieves >= 6/8 pass offline. Rule: stop tinkering with local infrastructure and advance to Track B.
- **Decision 2: Track B baseline lock (sizing sweep).** Trigger: sweep data collected. Threshold: identify the cheapest rental tier sustaining >= 50 tok/s for the 72B model. Rule: that tier becomes the operational default; renting larger is prohibited without a new explicit rule.
- **Decision 3: Track C pilot conversion.** Trigger: client pilot reaches 30 days or $100 spend. Threshold: client executes a paid contract. Rule: transition to production tier or terminate the pilot instance completely. **No perpetual free trials.**

**4. The amendment protocol is the correct mechanism.** It does not physically prevent goalpost-moving, **but it forces a visible paper trail of your rationalizations. The psychological friction of having to formally write down a bad excuse is what breaks the motivated-reasoning cycle.**

**5. The rent-first temptation (the question that matters).** The new danger is **convenience-driven budget bleed** and **endless benchmarking without shipping.** Without a $15K price tag to give pause, *"it is dangerously easy to rationalize leaving a heavy GCP instance running overnight so I don't have to wait 5 minutes tomorrow, or to stay in Track B forever tweaking rented setups because the hourly cost feels negligible."* **The pre-committed rules must now guard against zombie infrastructure and failure to advance.**

---

## DEEPSEEK (adversarial logic angle)

Asked: separate the machinery from the subject matter. What survives, and what gets dropped in a rebuild?

> **Worth carrying:**
> - **Pre-committed threshold** set before evidence is gathered. This is the core of the device.
> - **Mechanical application.** At decision time, no judgment call is allowed.
> - **Explicit inconclusive-evidence branch.** A written fallback for when evidence is ambiguous or absent, rather than
>   pretending the rule covers every case.
> - **Open amendment log.** When a rule is wrong you revise the *rule* with a dated entry; you never revise the
>   *interpretation* of the evidence.
>
> **Most often dropped when people rebuild such a system: the amendment log.** People keep the threshold and the
> mechanical trigger, but when the rule produces an uncomfortable outcome they silently reinterpret the threshold
> instead of openly amending it. **The accountability mechanism is the first thing to go.**

**Three-peer convergence: the machinery is the asset, the subject matter is the liability.** All three independently
said keep the structure and replace the content. DeepSeek and Gemini also converge specifically on the amendment log
being the load-bearing accountability piece, which is exactly the part a rewrite is most tempted to drop as
bureaucratic overhead.
