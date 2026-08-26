# Extracted from archived documents

Things worth keeping from documents that were archived during the 2026-08-26 corpus remediation. **Each entry records
what it came from and why it survived**, per the standing rule that nothing gets deleted and hand-waved.

Archived sources live in `archive/`. Review record: `gcp-test-plan/REVIEW-PROGRESS.md`.

---

## 1. The production floor

**From:** `archive/HARDWARE_DECISION.md`
**Now lives in:** `POLICY-rent-first.md` (this is a pointer, not a second copy)

At least **15 tokens per second per stream under 4-way batched concurrent load.** Written as a purchase criterion,
but the criterion is substrate-agnostic: below it, verification gates cannot consume agent output as fast as agents
produce it, and the substrate stops being real-time. That failure mode does not care who owns the silicon.

---

## 2. Software integration criteria (the tiers were not only about hardware)

**From:** `archive/graduated-gcp-validation-plan.md`, tiers 1-4
**Why it survived:** Gemini's finding, and it is the reason that document was not simply deleted. A priced sizing
sweep measures cost and performance. **These tiers measure whether the software works at all**, which is a different
question that nothing else in the corpus currently asks.

The integration questions worth preserving as a test plan in their own right:

| Question | Why it matters |
|---|---|
| Can the orchestrator dispatch to an inference server over HTTP and get a well-formed response back? | the basic contract |
| Can it manage **parallel git worktrees without collision**? | the concurrency model of the whole system |
| Does the **review-and-fix loop** complete end to end? | the core workflow |
| Do generated changes **merge cleanly**? | whether parallel work composes |

**These belong in an integration test plan, not in a gate that decides a purchase.**

### The reason they must be separated, stated plainly

The archived plan's FAIL branch read, in effect: *"if parallel git worktrees fail to merge, do not buy the
hardware."*

DeepSeek named the error: **affirming the consequent, compounded by a category error.** The implicit reasoning is
"if the hardware were inadequate the test would fail; the test failed; therefore the hardware is inadequate," which
ignores that a merge failure is far more likely to be an orchestration bug, a configuration problem, or a race
condition.

**It damages both decisions, and the second harm is the subtle one:**

- **The hardware decision** gets a signal contaminated by software causes, so it can veto adequate hardware or
  approve inadequate hardware for unrelated reasons.
- **The software decision** now carries a procurement consequence, **so the test gets distorted: thresholds tuned,
  failures downplayed, effort spent optimizing for clean merges rather than for real integration quality.**

**Carry this forward as a general rule: a gate that decides something expensive will be gamed, however
unconsciously, unless the thing it decides is genuinely what it measures.**

---

## 3. Capability-aware endpoint routing

**From:** `archive/model-selection.md` lines 227-250
**Why it survived:** the architectural idea is substrate-independent. Route tasks to model endpoints by capability
rather than treating all models as interchangeable.

**What did NOT survive:** the specific model-to-hardware mapping, which tied the optimization to "2-3 models per DGX
Spark," a machine nobody is buying. Resolve models by current benchmark and availability instead, which also avoids
baking a dated landscape into an architecture.

---

## 4. Two verification habits

**From:** `archive/model-selection.md`

**Search-derived model data conflates versions.** Model names, parameter counts, context windows, and licences must
be checked against primary model cards before anything depends on them. The archived document demonstrated the
hazard itself: it described a mixture-of-experts model as dense, carried a context window that had since doubled,
and listed a licence as unknown that had been published.

**"Model selection is a tuning knob."** It is not an architectural commitment, and should not be treated as one.
Architecture that survives a model swap is architecture; architecture that does not is a dependency.

---

## 5. Benchmarks do not predict task outcomes

**From:** `local-inference-buy-vs-rent.md` (demoted rather than archived, but this paragraph is the best thing in it)

Models fail at **integration and specification-reading, not at code generation.** A benchmark score measuring
generation quality therefore predicts very little about whether an agent completes real work.

Worth stating here because it undercuts the benchmark tables elsewhere in the corpus, including in the document it
comes from.

---

## What was archived and why

| Document | Reason |
|---|---|
| `HARDWARE_DECISION.md` + provenance | Marked `Status: ACCEPTED` while charting a path to $160K of hardware the policy forbids. |
| `model-selection.md` | April 2026 snapshot; landscape stale, VRAM budgets built on a cancelled purchase and containing real arithmetic errors, licence claims asserted rather than verified. |
| `graduated-gcp-validation-plan.md` | 442 lines gating a cancelled purchase, with a PASS branch claiming the purchase was "de-risked" and a FAIL branch binding software bugs to hardware viability. |
