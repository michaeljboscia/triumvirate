# Evidence Bundle Specification

**Status:** rewritten 2026-08-26 against a four-unit, three-peer review of the 2026-04-18 original
**Review record:** `REVIEW-PROGRESS.md` and `review-raw/20-EVIDENCE-SPEC-unit-*.md`
**Original:** git at `129c3b5:docs/pantheon/gcp-test-plan/20-EVIDENCE-BUNDLE-SPEC.md`

> **Tense rule, applied throughout and adopted corpus-wide.** Present tense is reserved for behavior that has been
> executed and verified. Everything else says `will`, `should`, or carries an explicit **NOT BUILT** marker. The
> original's defining failure was describing an unbuilt pipeline in the present tense with confident latencies
> ("within 60 sec"), which reads as evidence of implementation. Six automations were described that way. Zero were
> deployed.

---

## 1. What changed and why

The original was designed as an internal artifact justifying a GPU purchase. That purchase is cancelled. These bundles
are now shown to a prospect's security team as evidence for an isolation claim, which is a different job with a
different reader.

Three findings drove the rewrite:

1. **The design goals omitted the only property that matters to a hostile reader.** They listed immutable,
   self-describing, tool-agnostic, structurally queryable, semantically queryable, human-readable, and cheap. All
   three peers independently named the same absence: **verifiability**. Gemini's diagnosis was that this is "a data
   engineering brief, not a security brief."
2. **"Immutable" was declared, never enforced,** and the lifecycle contradicted it by updating the manifest mid-run.
3. **"Retain forever" and "provably destroy client data" cannot both be true.** This is structural, not editorial.

---

## 2. The split: two artifacts, divided by data ownership

**This is the load-bearing change.** The original produced one bundle for every purpose.

| | **Evidence bundle** | **Client data store** |
|---|---|---|
| Contains | metadata, hashes, compute logs, anonymized metrics, isolation captures | client pilot inputs, outputs, and derived artifacts |
| Mutability | write-once, content-addressed | normal, deletion-capable |
| Retention | retained | TTL, then destroyed |
| Deletion | not expected | **must be provable, with a certificate** |
| Signing | signed with a key the tested system never holds | not applicable |

**Why the split is required rather than merely tidy.** The sovereignty product promises a client their data can be
destroyed on request. The original also promised bundles are never deleted. Those are mutually exclusive while client
data sits inside the evidence archive. No wording resolves it. Separating by **data ownership** does: erasure of
client data never touches the evidence archive, and the evidence archive never contains anything a client can demand
be erased.

A secondary benefit falls out of it. Internal material (operator notes, subjective significance ratings, raw `strace`
output) stays out of anything a client reads, where it would either look unprofessional or leak infrastructure detail.

---

## 3. Design goals (revised)

1. **Verifiable.** Tamper-evident by construction. This is first because it was missing entirely and it is the only
   goal a skeptical reader cares about.
2. **Write-once and content-addressed.** Objects are never rewritten. Superseding state is a new object.
3. **Self-describing.** Schema version and generator identity travel with the artifact.
4. **Tool-agnostic.** Plain JSON, Markdown, CSV, and standard capture formats.
5. **Complete or explicitly incomplete.** A bundle states what it could not produce rather than omitting it silently.
6. **Human-readable.**
7. **Bounded in size,** with a stated policy for what happens when raw captures exceed the bound.

Goals 4 and 6 survive unchanged from the original. Goals 1, 2, and 5 are new. **Structural queryability and semantic
queryability were dropped as goals, not as capabilities:** they described downstream consumers that do not exist, so
they belong in a consumer roadmap rather than in the artifact's design brief.

---

## 4. Verifiability (new section, the one that was missing)

A bundle assembled by the system under test, stored where that system can rewrite it, is a system grading its own
homework. Declaring immutability is a promise; enforcing it is a mechanism.

**Minimum a skeptic can check:**

- Every object is **content-addressed**, and the manifest lists each object with its hash.
- The manifest is **signed with a key the tested environment never holds.**
- The signature is **anchored in an append-only log outside the writer's control** (a transparency log or a separate
  ledger). **This is the checkable part.** Bucket-level WORM is a useful second layer, not the first.
- The bundle records the **capture topology**: where the tap point sat relative to every egress path, and the exact
  versions of capture and validation tooling.

**NOT BUILT.** None of the above exists today. Until it does, a bundle supports the claim "these are the artifacts
this run produced" and not "these artifacts have not been altered."

---

## 5. Lifecycle (corrected)

The original created `manifest.json` at T+0 with `status="running"` and updated it at the end. That is a mutation, and
it contradicted the immutability goal directly.

**Corrected sequence:**

1. All working state is written to a **staging path outside the bundle.** Mutate freely here.
2. At finalization, final artifacts are written **once**.
3. Non-sentinel objects upload first.
4. **A completion sentinel uploads LAST.**
5. Consumers trigger **only on the sentinel**, and validate the manifest-declared object set before acting.

**Why the sentinel matters.** Object storage makes uploads visible one object at a time. The original triggered on
`manifest.json`, which was written first, so any consumer could fire against an incomplete bundle and insert partial
rows or fail nondeterministically.

**Preemption hazard.** The original uploaded the bundle and then deleted the VM. A hard kill between those steps
leaves the bundle partial or absent, and `trap` does not reliably survive preemption. **Upload must be resumable, or
finalization must run off the machine being torn down.**

The run-state record and the final verdict record are **separate write-once objects**. Neither is edited.

---

## 6. Required files

```
{storage_root}/{gate_id}/{run_id}/
├── manifest.json           REQUIRED, final, signed
├── run-state.json          REQUIRED, written at start, never edited
├── summary.md              REQUIRED
├── metrics/
│   ├── h-{id}.json         one per hypothesis
│   └── gpu-telemetry.csv
├── logs/
├── artifacts/
├── raw/                    isolation captures, see section 8
└── COMPLETE                the sentinel, written last
```

`obsidian-note.md` moves out of the bundle to an internal sidecar. **What it was for:** knowledge capture, so runs
compound into a searchable vault instead of evaporating. That purpose is real and survives. It simply must not ship to
a client, since it carries operator notes and subjective ratings.

### `manifest.json`

**Required fields.** Every one of these is either machine-derivable or the bundle is explicitly incomplete. The
original required none of the integrity fields and required a cost figure that cannot exist yet.

| Field | Contents | Why required |
|---|---|---|
| `schema_version` | this spec's version | consumers must know the shape |
| `run_id`, `gate` | identity | |
| `started_at`, `ended_at` | timestamps | |
| `generator` | tool name, version, git commit, **clean or dirty working tree** | a result from a dirty tree is not attributable to a commit |
| `objects` | every file path with its **content hash** | the basis of tamper evidence |
| `capture_policy` | duration, snap length, payloads, rotation, and whether truncation occurred | a capture is uninterpretable without it |
| `schema_valid` | did this manifest validate against `schema_version` | a bundle that failed validation must say so |
| `completeness` | which required artifacts are present, absent, or truncated | goal 5: explicitly incomplete beats silently partial |
| `cost_status` | `pending_billing_export`, `estimated`, or `final` | never a bare number of unstated provenance |
| `signature` | signature over the object list, key identity, external anchor reference | the checkable part |

`total_cost_usd` may appear **only** alongside a `cost_status` that is not `pending_billing_export`.

**Notably NOT required: `total_cost_usd`.** The original required it and populated the example with `0.86`. Cost
attribution depends on the GCP billing export, which is written **throughout the day rather than in real time**, so
recent usage typically appears within hours rather than at once. (A separate, larger delay applies only to the
**initial retroactive backfill** when an export is first enabled; that is a one-time condition, not the steady state.)

Either way, **authoritative cost is not available at the moment the bundle seals.** Emit
`cost_status: pending_billing_export` and fill it in later as a separate write-once record, or state a figure as an
estimate and label it as one. Never invent it.

### Decision records

**What they were for:** tracing a verdict back to the evidence that produced it. That is worth keeping; a client
asking "why does this say PASS" needs an answer.

**What was wrong:** the original carried `"confidence": 0.85`, an invented float with no scoring model, no inputs, and
no calibration, alongside a verdict of `"buy 2x 3090 NVLink"`. One baseless number contaminates every real number
beside it.

**Corrected shape:** `rule_id`, `rule_version`, `threshold_value`, `measured_value`, `passed`. Traceable, not invented.
No confidence field until something computes one.

---

## 7. Hypotheses and thresholds

**Keep the pre-registered hypothesis structure.** One peer recommended cutting it as an academic formality. I dissent,
and the record shows why: pre-registering a prediction and a threshold *before* a test runs is the only thing in this
corpus that prevents rationalizing a result afterward, and the absence of that discipline produced every other finding
in this review. **Replace the content, keep the structure.** The old hypotheses were about local inference on hardware
nobody is buying; the new ones are about isolation and pilot economics.

### The threshold bug, and why it is worse than it looks

The original's canonical metrics example recorded `tokens_per_second_per_stream_median: 12.4` at `concurrency: 1`
against `targets.tokens_per_second_per_stream_min: 10`, marked `PASS`.

Standing policy requires **15 tok/s/stream under 4-way batched load** (`local-inference-buy-vs-rent.md` section 6).

So the example is wrong twice, and the two are different in kind:

- **The threshold is wrong** (10 against a policy of 15), so 12.4 passes when it should fail.
- **The measurement condition is wrong** (`concurrency: 1` against a policy of 4-way batched), so the number does not
  measure the thing the policy is about.

**The concurrency error is the more dangerous one in a template.** A wrong number is visible and gets caught. A wrong
experimental condition propagates silently into every run that copies the template, and still produces a
plausible-looking PASS. **Fix the threshold. Fix the load condition first.**

Every metrics record must state its concurrency, and any comparison against the production floor must be at 4-way
batched load or explicitly marked as not testing the floor.

---

## 8. Raw captures and the size bound

The original set a bundle target under 100 MB, then listed per-gate sizes up to 500 MB, and separately relied on
`raw/` for tcpdump and strace without requiring any file to exist there.

Two conflicts, both unresolved in the original:

1. **`raw/` is load-bearing for the isolation claim.** If nothing populates it, the claim rests on summarized metrics
   rather than inspectable evidence, which is exactly the distinction a security team will press on.
2. **A tcpdump-bearing bundle can exceed 100 MB easily**, so the size goal and the evidence requirement fight.

**Resolution required before the isolation gate runs.** State the capture duration, the snap length, whether payloads
are captured, the rotation and compression policy, and what happens when the cap is hit. **A capture that was silently
truncated is worse than no capture, because it looks complete.** NOT BUILT.

### Requirements that settle both conflicts

1. **The isolation gate MUST fail if `raw/` is empty or missing.** It is not optional output. A gate that passes with
   no capture is asserting isolation from summarized metrics, which is the distinction the whole claim turns on. This
   is a hard gate condition, not a convention.
2. **The size bound applies per artifact class, not per bundle.** Metadata, metrics, logs, and summaries stay small.
   Captures are bounded by an explicit **capture policy** (duration, snap length, payload on/off, rotation), and the
   resulting size is whatever that policy produces. **The policy is the constraint; the byte count is the
   consequence.** The original had it backwards, setting a byte target and leaving the capture unspecified.
3. **Truncation must be recorded in the manifest**, with the reason and the point at which it occurred. A capture that
   hit its cap is still usable evidence when it says so, and worthless when it does not.

### GPU telemetry sampling

The original sampled `nvidia-smi` at 30 seconds. That is adequate for coarse utilization, memory residency, thermal
drift, and broad power shape. **It is blind to** short stalls, bursty saturation, allocation spikes, power-throttling
transients, PCIe and NVLink transfer bursts, and per-process attribution.

**For a performance claim, 30 seconds is too coarse.** Record per-process GPU usage, driver and CUDA versions, and
either finer sampling or DCGM metrics, plus correlation IDs tying telemetry to specific test windows. **For the
isolation claim it is irrelevant**, so this need not block Track A.

---

## 9. Retention and destruction

**Non-client evidence:** retained. Transitioning to a colder storage class is reasonable for rarely-read data, but
note nearline's 30-day minimum storage duration and its retrieval charges, and check how a lifecycle transition
interacts with any retention policy or bucket lock before enabling one.

**Client data:** TTL, then destroyed, with a **destruction certificate** as a deliverable. This is a product
requirement, not a nicety.

**The original said "All bundles retained forever" and "NEVER delete bundles."** What that was for: proving compute
spend was not wasted, and supporting the moat narrative. **It cannot survive contact with a sovereignty product**, and
it also conflicts with legal hold release, privacy deletion, contractual retention limits, and data minimization
obligations. The ownership split in section 2 is what lets the useful half of that intent survive.

---

## 10. Storage cost

The original claimed "100 runs/year = ~5GB = $1.20/year. Effectively free forever," two lines below a table whose own
per-gate figures total roughly **672 MB per full run**. That is **~67 GB and about $16/year**, more than 13x the
stated figure.

"Effectively free" also omitted Class A and B operation charges, retrieval charges outside Standard, egress, lifecycle
and rewrite operations, early-deletion minimums, and versioned-object storage if versioning is used for immutability.

**For Track A the storage root is local disk, so the relevant policy is disk quota and rotation, not cloud pricing.**
Size the local budget, rotate the debug artifacts, and keep the host from filling up.

---

## 11. Downstream consumers

**The original claimed six automations fire when a bundle lands, with stated latencies. An audit found zero
deployed.** Three had no implementation at all; three existed only as templates or unrelated code.

**What the pipeline was for, read charitably:** compute cycles normally produce ephemeral shell output rather than
durable intelligence. Writing results simultaneously to relational storage, vector storage, and readable Markdown was
an attempt to make retained evidence compound. **That problem is real and the thesis is sound.** It was simply built
in the wrong order: an elaborate consumer pipeline for a producer that had never produced anything, because the test
fixtures were never authored.

| Consumer | Status | What it was for | Disposition |
|---|---|---|---|
| Supabase extraction | NOT BUILT | aggregate tracking of cloud runs and costs | **Drop.** The in-bundle manifest serves the need. |
| Pythia embedding | NOT BUILT (index state exists) | historical runs semantically queryable | **Keep, move.** Local post-run script, not a cloud function. |
| Obsidian sync | NOT BUILT (template only) | readable reports into a knowledge base | **Keep, move.** Local copy step. |
| Dashboard refresh | NOT BUILT | cost-per-insight charts for cloud spend | **Drop.** No cloud spend on Track A. |
| Hypothesis tracker | NOT BUILT | forces human synthesis of raw data | **Keep.** See below. |
| Failure alert | NOT BUILT | paging when an unattended remote VM failed | **Drop.** Local execution, non-zero exit code suffices. |

**On the hypothesis tracker:** the original listed a human review step inside a list of automations, which is careless
labelling. But the step itself is correct and should stay. **Automation cannot synthesize a strategic lesson.** The
pipeline should present formatted evidence and stop, requiring a human to decide what it means before any conclusion
is recorded. Fix the label, keep the gate.

**No latency is claimed for anything above, because nothing is deployed.** When a consumer is built, it gets a
latency only once that latency has been measured.

---

## 12. Schema versioning

Increment `schema_version` on any breaking change and record it in the changelog below. **The original also promised
migrations in `harness/migrations/`, which does not exist,** and required that "downstream consumers handle both old
and new schemas," which is unenforceable while zero consumers are deployed. Both are deferred until there is a
consumer to be compatible with.

**Changelog:**
- `1.0` (2026-04-18) initial spec, never executed.
- `2.0` (2026-08-26) this rewrite. Breaking: bundle splits by data ownership; `manifest.json` is write-once and
  signed; `run-state.json` separated; `total_cost_usd` no longer required; `COMPLETE` sentinel added; decision records
  drop the confidence field; `obsidian-note.md` moves out of the bundle.

---

## 13. What this spec actually delivers

The original listed seven benefits. Four were false as written because they depended on consumers that do not exist,
and two were overstated. Corrected:

| Claim | Status |
|---|---|
| A defined, checkable bundle shape | **True**, this is what the spec delivers |
| Auditability: what config produced what result | **True once populated.** Note this is auditability, **not reproducibility.** Reproducing a result needs pinned inputs, which is a different guarantee. |
| Tamper evidence | **NOT BUILT.** Section 4 specifies it. |
| Semantic search across runs | **NOT BUILT.** Requires the Pythia consumer. |
| Structured queries across metrics | **NOT BUILT.** Requires a database consumer. |
| Cost accountability | **NOT BUILT.** Requires real billing data per run. |
| "Knowledge moat" | **Cut.** It was a story, not a property. A pile of test runs is not a moat, and a competitor needs a working implementation rather than your configuration history. The real value (evidence compounds if you keep it) is already covered by auditability, without the marketing. |

The original closed with "Guard them like production data." That was decorative: with no signing, no immutability
enforcement, and no access separation, the sentence carried no engineering constraint. **Section 4 replaces the
adjective with a mechanism.**
