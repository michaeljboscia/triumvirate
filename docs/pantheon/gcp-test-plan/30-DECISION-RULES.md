# Pre-Committed Decision Rules

**Status:** rewritten 2026-08-26 against a three-unit, three-peer review of the 2026-04-18 original
**Review record:** `REVIEW-PROGRESS.md` and `review-raw/30-DECISION-RULES-unit-*.md`
**Original:** git at `8ebb902:docs/pantheon/gcp-test-plan/30-DECISION-RULES.md`

> **The machinery survived; the subject matter did not.** All three peers reached that split independently. The
> original's ten rules governed one question: whether to buy hardware the author wanted to buy. That purchase is
> permanently cancelled. But the device itself (commit a threshold before you see the evidence, apply it mechanically,
> amend openly rather than reinterpret) is the most epistemically sound thing in this corpus, and its absence
> elsewhere produced most of the other findings. Gemini: *"You built this framework to stop yourself lying to yourself
> about evidence."*

---

## 1. Why pre-committed rules

A threshold chosen after you have seen the result is not a threshold, it is a rationalization with a number attached.
So the threshold gets fixed first, in writing, and applied mechanically when the evidence arrives.

Four properties make the device work. Drop any one and it stops being a constraint:

1. **The threshold is committed before the evidence is gathered.**
2. **Application is mechanical.** No judgement call at decision time.
3. **There is an explicit inconclusive branch.** Ambiguous evidence has a written path, rather than being squeezed
   into pass or fail.
4. **Amendment is open and logged.** When a rule turns out wrong you change the *rule*, dated, in public. You never
   change the *interpretation* of evidence already in hand.

DeepSeek's warning, worth stating up front because it predicts how this document will decay: **property 4 is the one
people drop.** They keep the threshold and the trigger, then quietly reinterpret when an answer is uncomfortable. The
accountability mechanism is always the first thing to go.

## 2. What these rules now guard against

**This section is new, and it is the point of the rewrite.**

The original rules existed to prevent one specific failure: rationalizing a $15K purchase. That temptation is gone,
and removing the rules without replacing them would leave the real risks unguarded. **A large irreversible cheque is
its own brake. Cheap reversible decisions have none**, so the bleed is continuous and nothing ever forces a stop.

Three failure modes, replacing the one that died:

| Failure mode | What it looks like | Guarded by |
|---|---|---|
| **Budget bleed** | leaving an instance up overnight to avoid waiting five minutes tomorrow; a year of promising runs adding to real money | Rule C |
| **Failure to advance** | staying in the cheap validation track forever because the next step is harder | Rule A |
| **Zombie infrastructure** | a pilot that never converts, a resource nobody owns, a workload nobody stops | Rule D |

## 3. How each rule is structured

```
Trigger:         what causes the rule to be evaluated
Evidence source: the specific artifact consulted, by name
Rule:            the threshold, stated numerically where possible
Fallback:        what happens when evidence is inconclusive
Amendment log:   dated changes, never silent edits
```

---

# The rules

## Rule A: Advance from local validation

**Guards against:** failure to advance.

- **Trigger:** the local plumbing and isolation runs have both completed on the Lenovo.
- **Evidence source:** the evidence bundle from each run.
- **Rule:** ADVANCE to the priced sizing sweep when the canonical task suite reaches its defined pass rate offline
  **and** the isolation run satisfies Rule B. Once both hold, **further local iteration requires a written reason
  logged in the rule application log and sent to a peer for review**, on the same terms as an amendment. An unlogged
  or unreviewed reason does not count, and the default is to advance.
- **Fallback (inconclusive):** one rerun. If still ambiguous, advance anyway and record the ambiguity, because
  lingering is the failure mode this rule exists to prevent.

> **Why the "written reason" clause exists.** The original's Gate 0 rule said *"No further Gate 0 runs required
> unless..."*, which Gemini correctly read as permission to declare the cheap test done and delay the expensive one
> indefinitely. A rule that lets you stop is not the same as a rule that makes you move.

**NOT BUILT:** the canonical task suite and its pass rate. See the fixtures section of `10-PREFLIGHT.md`. **This rule
cannot be applied until they exist**, which is itself an instance of the corpus-wide sequencing rule: author the
inputs before the gate that consumes them.

## Rule B: Isolation claim is safe to show a client

**Guards against:** telling a client something untrue. This is the product claim, so it is the most important rule
here.

- **Trigger:** an isolation run has completed and its evidence bundle exists.
- **Evidence source:** the bundle's `raw/` captures, the firewall configuration record, and the connected baseline.
- **Rule:** the claim may be made when **all** of the following hold:
  1. **Packet-capture evidence**, not merely firewall configuration, shows no disallowed egress during the window.
     Every permitted packet is individually accounted for by destination and protocol.
  2. The capture's coverage is stated: which interfaces, which address families (**including IPv6**), what window.
  3. **The disconnected run's output is functionally equivalent to a connected baseline**, where that baseline was
     captured and hashed beforehand **and its own correctness was independently established** (its outputs checked
     against expected results or semantic invariants, not merely observed to complete). **A baseline whose
     correctness is unverified fails this condition**, because comparing two degraded runs proves nothing.
  4. The claim's wording matches what was actually measured. See the wording rule below.
- **Fallback (inconclusive):** the claim is not made. There is no partial credit on this one.

> **Condition 3 is the one the original missed, and it is the reason this rule was rewritten rather than edited.**
> The original passed on the tasks having *completed* while disconnected. DeepSeek names the failure mode: **silent
> functional degradation**, where agent tooling falls back to cached defaults, skips a retrieval step, or returns an
> empty-but-valid-shaped result the surrounding code accepts. **A disconnected run can complete cleanly, produce
> worthless output, and pass a gate that tells a paying client the system works in isolation.**
>
> DeepSeek's catch on the fix, which matters for implementation: **the baseline can be contaminated by the same
> degradation.** It must be captured and hashed *before* the isolation test, with its own correctness established
> separately, or the parity check compares two degraded runs and proves nothing.

**Wording rule.** On a cloud VM with Private Google Access, the strongest supportable claim is:

> "The workload has no public IP and no general internet egress. Outbound traffic is blocked except to Google API
> ranges via Private Google Access, and during the test window after applying deny-all-egress, N outbound packets were
> observed, each accounted for."

Call that **cloud restricted-egress validation**. Reserve "air-gap proof" for a physically disconnected machine.

## Rule C: Sustained spend forces a stop

**Guards against:** budget bleed. **This rule is the original Decision 10, inverted.**

- **Trigger:** monthly spend review.
- **Evidence source:** the billing account, not an estimate.
- **Rule:** when spend on a workload class exceeds **$1000/month for two consecutive months** and the workload is
  sustained rather than a one-time burst, **work on that class STOPS until one of the following is chosen in
  writing:**
  1. **Optimize.** A named change, with the expected magnitude stated **and a date at which actual spend is compared
     against that expectation.** If the actual does not move as predicted, option 1 is spent: choose 2 or 3.
     **Option 1 may be chosen at most twice in a row for the same workload class.**
  2. **Commit.** Move to a reserved or committed-use instance, **only if** the on-demand cost at observed hours
     exceeds the reserved cost plus a stated allowance for lock-in risk.
  3. **Stop.** Shut the workload down.
- **Fallback (inconclusive):** if the spend cannot be attributed to a workload class, that is itself a finding. Fix
  the attribution before continuing to spend.

> **What the original said, and why the inversion matters.** It read: spend crossed $1000/month for two months,
> **therefore buy hardware.** That uses high rent as an argument for capital expenditure, which is precisely backwards
> under a policy where renting is the destination. Gemini's verdict on the inversion: *"It works perfectly and is not
> too clever. Renting is the destination."*
>
> The original also carried break-even utilization hours by card class (RTX Pro 6000 > 150 hrs/mo, A100 80GB > 230,
> H100 > 360). **Those numbers are preserved here as a record, not adopted as thresholds.** Codex: they assume
> purchase price, usable life, utilization, hourly rate, and power/support/ops overhead, none of which are stated, so
> they are not auditable. **Recompute them around the reserved-versus-on-demand delta before option 2 above is ever
> exercised.**

**Also note:** the original's trigger was purely financial, so it could authorize buying hardware with no requirement
that the hardware clear the production floor. Any commitment under option 2 must additionally satisfy Rule E.

## Rule D: Pilot converts or terminates

**Guards against:** zombie infrastructure.

- **Trigger:** a client pilot reaches an agreed elapsed time or an agreed spend cap, whichever comes first.
- **Evidence source:** the pilot's spend record and the engagement status.
- **Rule:** the pilot either converts to a paid contract or **terminates completely**, including deleting its
  resources and issuing the client a destruction certificate. **No perpetual free trials.**
- **Fallback (inconclusive):** if conversion is genuinely pending, one extension of a stated length, logged. One only.

## Rule E: Production readiness

**Guards against:** shipping something too slow to work.

- **Trigger:** a release-candidate readiness bundle exists.
- **Evidence source:** that bundle's soak, concurrency, and fault-injection results.
- **Rule:** ready when the soak shows no resource leak over the stated duration, fault injection recovers within the
  stated window, **and throughput meets the production floor of 15 tokens per second per stream under 4-way batched
  load.**
- **Fallback (inconclusive):** not ready.

> **The throughput condition is new.** The original's production-readiness rule gated on soak, concurrency, and fault
> behavior but never on tokens per second, so a system could pass every condition and still be too slow for the
> verification gates to consume its output. Below the floor, backpressure inverts and the substrate stops being
> real-time.

## Rule F: Core thesis validation

**Guards against:** believing the approach works when it does not.

- **Trigger:** a core thesis validation run completes.
- **Evidence source:** that run's bundle.
- **Rule:** VALIDATED when generated-code validity is at least 80%; FALSIFIED below 50%; **50-79% is inconclusive**
  and triggers the fallback.
- **Fallback (inconclusive):** one rerun with a stated change. If still in the middle band, the thesis is recorded as
  partially supported with the band stated. **It is never rounded up.**

> The 80/50 split with an explicit middle band is preserved from the original because it is one of the few places the
> corpus handled ambiguity honestly rather than forcing a binary.
>
> Under the rebuilt plan this rule also carries a second output: its wall-clock figures feed **unit economics** for
> pricing a pilot, rather than only producing a pass or fail.

---

## 4. Amendment protocol

**The original's protocol was documentary, not binding, and this is the correction.**

Rules change when they turn out wrong. That is legitimate and expected. What is not legitimate is changing a rule
because you dislike the answer it just produced.

**Requirements for any amendment:**

1. **The amended rule applies only to evidence collected after the amendment date.** Never retroactively. This is the
   minimum structural constraint and it is not optional.
2. **A 72-hour cooldown between proposing an amendment and it taking effect.** Stated as a number because this
   document's own second property requires mechanical application, and "a cooldown" is not mechanically checkable.
   The point is to separate the amendment from the disappointment that prompted it.
3. **Peer review before it takes effect.** Send the proposed change to Codex or Gemini and record the response
   verbatim, including a response that disagrees.
4. **The log entry states:** the date, **the amendment's author**, the exact before and after text, **the evidence
   IDs that prompted the change**, **what the old rule was for**, whether that problem still exists, and what
   replaces the capability if it does.

> **Why requirement 3 exists.** Gemini argued in one review unit that the friction of writing down a bad excuse
> breaks the motivated-reasoning cycle, then argued in another that journaling a rationalization merely records it.
> DeepSeek resolved the contradiction: the first is true *"for people with enough integrity that seeing a bad excuse
> in writing shames them into honesty, but that is a personality trait, not a mechanism,"* and the second holds
> **whenever the same person controls the threshold, the reasoning, and the application, and faces no external cost.**
> That is exactly this situation.
>
> **A log records drift; it does not stop it. Only a veto from outside the motivated mind can.** A solo operator
> cannot supply his own external veto, but the peer agents are genuinely external and cost almost nothing. That is
> the separation of powers available here, so it is the one used.

Requirement 4's "what was it for" clause is the owner's standing rule, promoted from review practice into the
protocol itself.

---

## 5. Rule application log

Each application records: the date, the rule, the evidence bundle consulted by ID, the observed values, the verdict,
and what happened next.

**Keep it light enough to actually use.** The original specified a hand-written JSON object per application and the
log sat **empty for four months**. Gemini's reading is correct: *"An empty log after four months proves the mechanism
was a bureaucratic fantasy. If the tool is too heavy to pick up, it gets bypassed entirely."* A ledger entry or a git
commit is sufficient. A perfect format nobody fills in is worth less than a rough one that gets used.

**The log is currently empty.** No rule in this document has ever been applied, because no evidence has ever been
gathered.

---

## 6. What was cut, and what it was for

Recorded so nobody restores these without reading why they went.

| Cut | What it was for | Does the problem persist? | Replacement |
|---|---|---|---|
| **Decision 1** (1x vs 2x 3090) | choosing a local GPU configuration | as capacity planning, yes | the measurements move into the sizing sweep; the purchase gate is gone |
| **Decision 2** (RTX Pro 6000) | whether to buy a bigger card | as "is a higher rental class justified", yes | Rule C option 2 |
| **Decision 3** (second card) | scale-out after a purchase | as rental scale-out sizing, yes | utilization evidence in the sizing sweep |
| **Decision 6** (Rack tier) | buying an $80-500K enterprise rack | the purchase, no; the **commitment** decision, yes | Rule C option 2, reserved instances instead of metal |
| **Decision 9** (Mac Studio) | buying Apple silicon for Mac-native work | yes, but rent-first applies | rent `mac-m3ultra.metal` first; CapEx only if rental is structurally unviable for a named reason |
| **Decision 10** (OPEX→CAPEX) | a spend threshold as stop-and-think | **yes, and it was the only such brake** | **inverted into Rule C** |

**A note on Decision 9, because it is the one good-news finding in this review.** Applied mechanically as written, it
said **do not buy**, and that was correct. A 512GB M5 Ultra was announced 2026-08-25, which looks like a trigger, but
the rule required a paid engagement and a friction log alongside it, and those predicates were not met. **It is the
only place in this corpus where a pre-committed rule ran as designed and produced the right answer.** That is the
argument for keeping the machinery.

Two stale facts removed with it: the `~$12K` figure for a 512GB configuration (Apple has not published a price), and
a fallback branch for "Apple does not ship 512GB," which events have overtaken.

**Thresholds preserved as a record, not adopted:** contention factors `< 2.0` and `< 1.5x`; `32B LoRA <= 3 hrs` and
`<= 2 hrs` without OOM; `>= 5 friction events/week where the 48GB tier specifically mattered`; `72B single-stream
>= 50 tok/s`; `canonical 8-task swarm >= 6/8 pass`; utilization `> 80% for 4+ weeks` and `> 70% for 30+ days`.

**One threshold is preserved only as a warning:** the original pre-committed `>= 5 tok/s per stream` under 4-way
batched load. **The standing production floor is 15.** It was three times too low, in a rule meant to be applied
mechanically. It is recorded here so nobody reintroduces it.

---

## 7. What this document does

It fixes thresholds before evidence arrives, so a result cannot be rationalized afterward.

That is the whole claim. The original's closing section asserted an auditable decision trail that did not exist, and
cited a CapEx trigger as evidence of business defensibility while that trigger contradicted standing policy. **No
rule in this document has been applied yet, so it currently enables nothing. It will enable the above once evidence
exists to apply it to.**
