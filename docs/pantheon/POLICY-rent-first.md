# Standing Policy: Rent First

**Status:** operational directive, in force
**Established:** 2026-08-23 · **Split into a standalone document:** 2026-08-26
**Decided by:** Mike Boscia
**Supporting analysis:** `local-inference-buy-vs-rent.md` (an August 2026 snapshot; see the note on its standing below)

---

## The policy

**Rent compute. Owned metal only ever as a customer-funded terminal step, after a rented pilot and a signed term.
Never our capex.**

**Renting is the destination, not a stage before buying.** The terminal step may never arrive, and that is an
acceptable outcome rather than a failed plan.

## Why this is a decision and not a calculation

**This is stated deliberately, because how it is framed determines whether it survives contact with a changing
market.**

The policy rests on the owner's judgment about what business he is running, not on a derived economic model. A
three-peer review of the supporting analysis found it contains **no crossover threshold, no cost model, and no
falsification condition anywhere.** That does not make the policy wrong. It makes it a decision.

**A strategic business decision holds regardless of hardware price movements. A mathematically forced one collapses
when prices drop.** Presenting this as a calculation would invite exactly the challenge a price fall brings, and
there is no model underneath to meet that challenge.

The reasoning behind the decision, in the owner's own terms:

- Nothing gets bought until a customer needs the iron.
- The buyer who justifies owned hardware needs privacy and compliance requirements so specific that the population is
  very small, and it shrinks further on contact, because most compliance-driven prospects are satisfied by frontier
  models hosted inside their own cloud tenancy.
- Renting will be the primary consumption model for the foreseeable future.

## When this gets reopened

**A pre-commitment is only legitimate if it names, in advance, what would reopen it.** A rule that refuses to
reconsider on the single most likely disconfirming observation is not discipline, it is insulation.

So: **this is not revisited on a transient price dip.** It IS revisited on a **sustained structural change**, defined
as any of:

| Trigger | Threshold | Status |
|---|---|---|
| Sustained price decline | acquisition cost of the target configuration falls at least **X%** and stays there for **Y consecutive quarters** | **X and Y NOT SET** |
| Sustained utilization | measured utilization of rented capacity exceeds **Z%** across **N consecutive months** | **Z and N NOT SET** |
| Customer-funded purchase | a signed engagement puts hardware on the client's balance sheet | this is the terminal step, and is in policy already |
| Metering constrains the work | see below | needs a metric |

**The thresholds are deliberately unset rather than invented.** Putting a plausible-looking number here without
computing it would repeat the defect this split exists to fix. **Set them from measured rental spend and utilization
once Track B has produced real numbers.** Until then, the honest position is that the reopening conditions are named
but not yet quantified, which is strictly better than pretending they do not exist.

### The trigger nobody had thought of

**Metered inference suppresses work you never attempt.** Per-token pricing penalizes high-iteration agentic loops,
long-context retrieval, and speculative execution, because every failed attempt costs money. That cost never appears
in a token bill, because the bill only records what you did run.

**A volume-based crossover cannot see this**, which is why "spend exceeds infrastructure cost" is the wrong threshold
on its own. If metering is measurably changing what work gets attempted, that is a reopening condition in its own
right, and it needs a metric rather than a feeling.

**This is also the honest place to record the owner's own stated instinct**: wanting a large local machine for its
own sake, for experimentation and for the removal of per-token friction. **That is a legitimate reason, and it does
not need to be laundered into a business case.** A lease is opex against a business rather than capital tied up in a
depreciating box, so the utilization math that kills a purchase does not automatically kill a lease. Two conditions
keep that honest: name it as wanting the machine, and rent one first (256GB M3 Ultra metal is rentable today; the
512GB M5 Ultra reaches rental racks around the same time it reaches retail).

## Order of operations

Each step needs a trigger before it counts as a process rather than a narrative. **The triggers marked NOT SET are
genuinely undefined and should be filled in rather than assumed.**

| Step | Trigger to proceed | Status |
|---|---|---|
| 1. Sell the outcome | a qualified prospect with a stated requirement | qualification criteria **NOT SET** |
| 2. Pilot on rented GPUs | signed pilot scope and a spend cap | pass/fail criteria **NOT SET** |
| 3. Sign a term | pilot met its stated criteria | amortization math **NOT SET** |
| 4. Metal on the client's balance sheet | term signed and long enough to amortize the specific hardware | **conditional and optional** |

**The sequence terminates successfully at any step.** Most engagements should end at 1 or 2, and a pilot that does
not convert is a normal outcome rather than a failure. **Step 4 is not the goal.**

## The performance floor that applies regardless of who owns the hardware

**At least 15 tokens per second per stream under 4-way batched concurrent load.**

Below that, verification gates cannot consume agent output as fast as agents produce it, backpressure inverts, and
the substrate stops being real-time. **That failure mode does not care who owns the silicon**, so it applies to
rented configurations exactly as it did to owned ones.

Consequences for renting:
- A rented configuration below the floor is **not cheap, it is useless.** Cost per hour is the wrong axis to select
  on alone.
- Every row in a sizing sweep must report tokens per second per stream **at 4-way concurrency**, next to its price.
- **A single-stream number cannot be compared to this floor.** Most published throughput figures are single-stream.

For calibration: a 2x L4 configuration was measured at a median of 12.4, p95 10.1, p99 8.6 under that load. **That is
below the floor**, and it is a warning against renting that card class and expecting fleet throughput.

## What this policy does not decide

- **Which provider.** Pick from the client's constraint, not habit. Cross-provider pricing varies by a factor of
  several for the same silicon.
- **Whether to lease.** A lease is opex, not capex, and is not what this policy forbids. See above.
- **Whether the retainer product works.** That is a separate open question, and a live one: if maintaining a client's
  context layer is bespoke manual labor per client, the business is a services firm with a technology story and
  should be priced and staffed accordingly.

## Standing of the supporting analysis

`local-inference-buy-vs-rent.md` remains as the August 2026 snapshot that prompted this decision. **It is a dated
research artifact, not the authority for this policy.** A three-peer review found it anchors on a single uncited
second-hand listing, compares vendor-run benchmarks against third-party ones for competing models, draws its
throughput verdict from a capacity-limit test read as a representative benchmark, and states no crossover anywhere.

**Those defects do not touch this policy**, because this policy does not derive from them. That separation is the
entire reason for the split: welded together, one price movement appears to invalidate both.
