# Priced Sizing Sweep: method

**Status:** created 2026-08-26, replacing six demoted gate runbooks
**Replaces:** `runbooks/gate-1` through `gate-5` and `gate-7`, archived under `runbooks/archive/`
**Review record:** `REVIEW-PROGRESS.md`, `review-raw/queue-item-8.md`

> **This is Track B of the rebuilt plan.** The six runbooks it replaces existed to justify a hardware purchase that is
> cancelled. Their measurement *methods* survive that cancellation; their purchase verdicts do not.

---

## 1. What this measures, in two layers

Both layers are needed, and **the second depends on the first.**

| | **Layer A: capability** | **Layer B: unit economics** |
|---|---|---|
| Rows | hardware configurations | the same configurations |
| Measures | tokens/sec/stream at stated concurrency, VRAM peak, contention factor, cost per hour | cost and elapsed time per unit of business output |
| Nature | **an objective measurement** | **an estimate under stated assumptions** |
| Audience | us | a client deciding what to buy |

**Layer B is what a client actually asks for**: "Configuration A costs $X and two hours per merged pull request;
Configuration B costs $Y and eight." A tokens-per-second figure does not answer the question they are asking.

**But Layer B cannot be measured directly, and pretending otherwise is how it becomes a marketing number.** It is
modelled on top of Layer A plus explicit workload assumptions. Publish it as an estimate, with the assumptions
visible, or do not publish it.

### What makes Layer B hard to state honestly

Each of these is a way the number quietly becomes fiction:

- **The unit must be defined and repeatable.** How many review passes, test runs, builds, context windows, and
  retries constitute "a merged pull request"?
- **Queuing and contention count**, not just peak throughput.
- **Cherry-picking is easy and invisible.** An easy task or a favorable prompt mix moves the number a lot.
- **State the assumptions inline**, next to the figure, not in an appendix nobody reads.

---

## 2. Configurations to sweep

| Row | Where | Notes |
|---|---|---|
| Local RTX 4000 Ada 12GB | the Lenovo | Track A's machine; the zero-cost baseline |
| L4 24GB | rented | Ada, same die family as the local card |
| Dual L4 / 48GB class | rented | the configuration measured below the floor, see calibration |
| G4, RTX PRO 6000 Blackwell 96GB | GCP | GA, and cheap on spot |
| A100 80GB | **RunPod first** | roughly 2-4x cheaper than GCP for the same silicon |

**Pick the provider from the client's constraint, not from habit.** GCP earns its premium only when the client
requires their own tenancy.

---

## 3. Measurement method

**This is the part worth preserving from the archived runbooks.** The gates are dead; these procedures are not.

### The batch ladder

Measure each configuration at **single-stream, 4-way batched, and 8-way batched (stress)**. From the archived gate-3
method, which was the only one that ran the full ladder.

**4-way is not optional and not one option among three.** The standing production floor is stated as
**15 tok/s/stream under 4-way batched load**, so a configuration measured only single-stream **cannot be compared to
the floor at all.** Most published vendor throughput figures are single-stream, which is precisely why this has to be
measured rather than looked up.

### Contention

From the archived gate-2 method: **run each endpoint in isolation, then hit both simultaneously, and compute a
contention factor** per endpoint. Multi-model hosting is a real deployment pattern and its cost is invisible in
single-endpoint numbers.

The archived gate-3 mixed-load variant is worth keeping as a pressure test: unequal concurrency across three
endpoints, rather than a symmetric split.

### Per row, record

`model fit` · `single-stream tok/s` · `tok/s/stream at 4-way` · `tok/s/stream at 8-way` · `VRAM peak` ·
`contention factor per endpoint` · `cost per hour` · `cost per 1M tokens`

Then, as **explicitly labelled estimates**: `cost per unit of work` and `elapsed time per unit of work`.

---

## 4. Soak and stress belong here, and they matter more now

**The archived gate-7 was demoted along with the rest, and that was too hasty.** Under rent-first with client pilots,
long-duration behavior is more important than it was when the plan served a purchase, not less.

Gemini's reasoning, and it is correct: **a client pilot runs unattended.** If the orchestrator hits a retry cascade at
hour six, the client bleeds budget indefinitely on infrastructure making zero progress. **A short test cannot show
KV-cache drift, memory leaks, or retry storms, and those are exactly what destroy pilot economics.**

So the sweep includes a sustained-load row, and the fault-injection scenarios from the archived gate-7 get ported
into the automated suite rather than lost: killing an inference container mid-request, corrupted tool-call responses,
and the retry behavior that follows each.

**Port those scenarios before the archived runbooks stop being consulted.** They are specific and hard-won, and
"we'll remember them" is how they disappear.

---

## 5. Evidence contract

Each row emits one metrics record conforming to `20-EVIDENCE-BUNDLE-SPEC.md`.

Three prohibitions, each learned from a specific defect in the archived runbooks:

- **No purchase verdicts.** These rows inform pricing and configuration choice. Nothing here decides an acquisition.
- **No invented cost certainty.** Record `cost_status` rather than a bare figure; billing export is not real-time.
- **No dependency on `/opt/pantheon-harness/`**, a path from a GCP image that was never built and the single most
  likely first-run failure across the whole corpus.

---

## 6. What was archived, and what came out of it

Per the standing rule, recorded so nobody restores a runbook without knowing why it went.

| Runbook | What it was for | Does the problem persist? | What replaces it |
|---|---|---|---|
| gate-1, single L4 | 24GB viability as a 3090-single proxy | as rented 24GB sizing, yes | a row in this sweep |
| gate-2, dual L4 | 48GB viability, marked DECISIVE for the purchase | the measurement yes, the verdict no | a row, plus **its contention method, preserved above** |
| gate-3, RTX PRO 6000 | hardware twin for a $15K card | as rented G4 sizing, yes, and G4 is now cheap on spot | a row, plus **its batch ladder, preserved above** |
| gate-4, worker swarm | could parallel workers produce mergeable code | **yes, but it is a software integration question**, not a hardware one | the integration criteria in `../EXTRACTED-from-archive.md` |
| gate-5, full trinity | empirical evidence of what $100-400K of hardware would deliver | no, that question is gone | nothing; it was the most expensive gate and served only the purchase |
| gate-7, soak and stress | long-run stability | **yes, and more than before** | section 4 above |

**Only two of the six measured at 4-way concurrency at all** (gate-2's 72B run and gate-3's ladder). The rest were
single-stream, which is why the production floor could never have been evaluated from them as written.
