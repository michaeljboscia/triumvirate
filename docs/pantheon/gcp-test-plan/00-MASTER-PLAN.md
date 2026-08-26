# Pantheon Rented-Compute Validation Plan

**Status:** canonical · rebuilt 2026-08-25 (supersedes the 2026-04-18 plan)
**Owner:** Mike Boscia
**Purpose:** Prove the sovereign product claim on rented hardware, and produce a priced catalog of pilot configurations we can stand up for a client.

> **This is a rewrite, not a revision.** The 2026-04-18 plan existed to de-risk a personal GPU purchase (1x vs 2x RTX 3090). That purchase is cancelled permanently. On 2026-08-23 a banner was bolted to the top of the old document telling the reader to mentally reinterpret every purchase rule as something else. That did not work: the body still ranked gates by a decision that no longer exists. The old version is in git at `6d48e6f:docs/pantheon/gcp-test-plan/00-MASTER-PLAN.md` if provenance is needed.

---

## 1. What changed, and why the old plan could not be patched

Standing policy is **rent first, always** (`docs/pantheon/local-inference-buy-vs-rent.md` section 6). Owned metal happens only as a customer-funded terminal step: sell the outcome, pilot on rented GPUs, sign a term, and only then metal on the client's balance sheet. Never our capex.

**Sharpened 2026-08-25: renting is the destination, not a waypoint.** The earlier framing still read as a sequence ending in a purchase. It does not. Renting is the primary consumption model for the foreseeable future, and the owned-metal step may never arrive. That is an acceptable outcome, not a failure of the plan.

The buyer who justifies local iron needs privacy and compliance requirements so specific that the population is very small, and it shrinks further on contact: most compliance-driven prospects are satisfied by Bedrock or Vertex inside their own VPC, which is a door we can walk them through without owning anything. Nothing in this plan should be read as building toward a purchase, and no work here is justified by "it gets us closer to buying."

The old plan's spine was incompatible with that:

| Old plan assumed | Actually true now |
|---|---|
| Gates 1+2 are DECISIVE because they settle the 3090 buy | Nothing is bought. Those gates settle nothing. |
| Gate 3 simulates an RTX Pro 6000 we intend to purchase | We do not intend to purchase one. |
| Gate 6 (air-gap) is a late nice-to-have | Air-gap **is** the product claim. It runs early or the rest is moot. |
| Evidence bundles justify an internal purchase | Evidence bundles are shown to a **prospect**. Different artifact entirely. |
| VMs should self-destruct aggressively | A client pilot cannot self-destruct mid-demo. |

**Cut outright** (do not annotate, delete): the "3090 decision" section, the L4-as-3090-proxy table, decision rules 1 through 3 in `30-DECISION-RULES.md`, and the DAY 1 / DAY 5-7 execution order that sequenced work around collecting 3090 data.

---

## 2. Ground truth as of 2026-08-25

Verified, not assumed:

- **Nothing has ever been executed.** No `pantheon-validation-v1` project exists. `gcloud projects list` shows nine unrelated projects. Preflight step 1.1 was never run.
- **`fixtures/` does not exist, and never did.** The old plan's directory listing claimed four canonical test corpora. Every gate claiming "a measurement harness (scripted, deterministic)" has no inputs. Checked three ways on 2026-08-25, because "we surely built this and lost it" is the natural assumption: (1) the 2026-05-14 MacBook backup holds an *identical* 22-file tree here, with no `fixtures/`, no `configs/`, no kill function, and no `runner.py`; (2) a whole-backup index search for the corpus names (`test-tasks`, `eval-scorers`, `test-corpus-triumvirate`, `lora-training-corpus`) returns nothing anywhere on the machine; (3) `git log --all --diff-filter=D` over this directory shows nothing was ever deleted, and the 22 files present are the complete set that has ever existed on any branch. It was not lost. It was never written.
- **What *was* real:** all six harness scripts are genuine files, the harness carries a `.ruff_cache` so the Python was actively linted rather than pasted once, and `pantheon-vault` (the Obsidian target) exists. The work was real. The directory tree in the old document described an intended end state in the present tense, and the gap was never reconciled because nothing ever ran.
- **Spend-control layer 6 is fiction.** The old plan claimed a Pub/Sub billing alert firing a Cloud Function that deletes instances across all regions. The function exists only as a pasted snippet inside `10-PREFLIGHT.md`. There is no deployable source anywhere in the repo.
- **Spend-control layer 4 is fiction.** It referenced `timeout Nm python3 runner.py`. No `runner.py` exists, and `runner-wrapper.sh` does not wrap the remote gate script in a timeout.
- **`runner-wrapper.sh` fails on the normal first run.** Line 192 uses `local now=$(date +%s)` outside any function. Bash errors the first time the SSH readiness probe misses, which is the expected path.
- **`request-quotas.sh` targets the wrong project** (`aerial-jigsaw-467620-m8`, line 15).
- **Cost reporting is fake-precise.** `finalize-evidence.py` hardcodes duration to 1.0 hour (line 247). `cost-tracker.py` falls back to `e2-standard-4` at 1.0 hour when metadata is absent (line 219), which understates a GPU run by an order of magnitude.
- **Fixture names disagree across three documents.** Master plan, preflight, and the runbooks each name a different set.

Treat the harness as a skeleton with a few useful safety primitives, not as working software.

---

## 3. Corrected hardware landscape

The old plan's pricing is four months stale and some of it was never right. Current figures, with the caveat that GPU and DRAM pricing is moving fast in the 2026 shortage.

### 3.1 Rented GPU, per hour

| Config | VRAM | GCP on-demand | GCP spot | RunPod |
|---|---|---|---|---|
| 1x L4 (`g2-standard-4`) | 24GB | ~$0.71 | ~$0.42-0.62 | $0.45-0.60 |
| 1x A100 80GB (`a2-ultragpu-1g`) | 80GB | ~$5.03 | ~$2.51 | **$1.19-1.60** |
| 1x RTX PRO 6000 Blackwell (`g4-standard-6`) | 96GB | ~$0.65 | ~$0.26 | n/a |
| 1x H100 PCIe | 80GB | n/a in this table | n/a | $1.99-2.99 |
| 8x B200 (`a4-highgpu-8g`) | 1.4TB | ~$90.22 | ~$39.63 | n/a |

**RunPod is 2-4x cheaper than GCP on-demand for A100 class.** For pilot economics that is decisive, and it is the single biggest cost finding in this rebuild. GCP earns its premium only where the client requires it (their own VPC, their compliance boundary, IAP, VPC Service Controls). Pick the provider from the client's constraint, not from habit.

New since April: the G4 series (RTX PRO 6000 Blackwell Server Edition, 96GB GDDR7) is GA and is **cheap on spot**. Fractional G4 (1/2, 1/4, 1/8 of a card) now exists and lowers the floor for light inference. Valid G4 machine types are `g4-standard-6/12/24/48/96/192/384`. The old `cost-tracker.py` references `g4-standard-32`, which is not a real type.

### 3.2 The V100 idea does not survive contact

An 8x V100 32GB box gives 256GB of VRAM for very little money, which is why it keeps looking attractive. It is a trap in 2026:

- **vLLM dropped Volta (sm_70) in 0.20+.** Prebuilt wheels ship no sm_70 kernels. You get "no kernel image is available" and must run a community fork or build from source against CUDA 12.6. CUDA 12.8+ is phasing Volta out entirely.
- **FlashAttention 2 requires Ampere (sm_80) or newer.** This is a hardware constraint, not a software gap. V100 falls back to xFormers or Triton attention, costing roughly 30-50% throughput in long-context work.
- **No bf16.** Modern models are trained in bf16. On V100 you downcast to fp16, which can overflow in specific layers and destabilize the model. No fp8 either.
- **It is not even cheap on GCP.** ~$2.67-3.20/hr on-demand, against ~$0.71 for an L4 that is faster and more stable. P100 hit end of support 2026-09-15 and V100 is listed as approaching it. NVIDIA removed V100 from AI Enterprise Infra 8.0+.

**Verdict: do not build a track around V100.** If the goal is "lots of VRAM, cheaply," the current answer is spot G4 (96GB per card, ~$0.26/hr spot) or RunPod A100 80GB at $1.19/hr. Both have a working software stack.

### 3.3 Apple Silicon, corrected

There is **no M4 Ultra**. Apple skipped it: the M4 architecture lacked the interconnect to bridge two Max dies. The current high-end desktop part is the **M5 Ultra**, announced 2026-08-25.

- Mac Studio M5 Ultra base (96GB): **$5,499**, ships 2026-09-22.
- 256GB configuration: **~$9,499** (the +$4,000 memory upgrade), not $12K.
- **512GB configuration: announced 2026-08-25, ships late October 2026, price not published.** The ~$13K figure circulating is an analyst extrapolation. Do not put it in a client quote as a price.
- High-memory configs carry roughly a 30% premium over 2024 levels because of the DRAM shortage.

This is cheaper and more available than the superseded assumption (a 512GB M3 Ultra as a $23-26K scarce used-market item). **It does not reopen the buy decision for us.** The rent-first policy rests on utilization, not just sticker price, and a cheaper box does not create utilization we do not have. What it does change is the **client-side bill of materials**: a sovereign build we quote to a customer just got materially cheaper and comes new and in warranty. That helps the proposal, not our capex.

**The lease case, kept honest.** A leased 512GB M5 Ultra is a different question from a purchased one and the policy above does not forbid it. A lease is monthly opex against a business, not capital tied up in a depreciating box, so the utilization math that kills the purchase does not automatically kill the lease. Two conditions keep it clean:

1. **Name it for what it is.** "I want the box" is a legitimate reason. It stops being legitimate the moment it gets dressed up as a business case that this plan is supposed to have validated. Curiosity and flexing are real motives with real value (they are how capability gets discovered), and they are cheap as long as they are not laundered into a forecast.
2. **Rent one before leasing one.** 256GB M3 Ultra is rentable today via AWS `mac-m3ultra.metal`, and the 512GB M5 Ultra is expected on rental racks late October, the same window as retail. A few days of rental at the 24-hour minimum answers whether 512GB of unified memory actually does what it promises on our workload, for roughly the cost of a nice dinner, before signing anything multi-year.

Do not put a price in a lease conversation yet regardless: Apple has not published the 512GB figure.

### 3.4 Rented Apple metal exists

Confirmed. This was not in the old plan at all.

- **AWS EC2 Mac:** `mac-m3ultra.metal` and `mac-m4max.metal`, roughly $1.08/hr entry to $3.50+/hr for Ultra tier. Largest rentable unified memory today is **256GB (M3 Ultra)**.
- **MacStadium:** enterprise monthly. Mac Studio M2 Ultra 128GB around $449/month.
- **Scaleway:** Mac mini M4 from about €0.11/hr or €75/month.
- **Hard constraint:** Apple licensing imposes a **24-hour minimum allocation**. Per-second billing habits from GPU clouds do not transfer. Budget Mac experiments in whole days.
- 512GB M5 Ultra is expected on rental racks late October 2026.

**Consequence:** the "sovereign appliance on Apple Silicon" story can be piloted on rented metal before anyone buys anything, which is exactly the order of operations the policy demands.

---

## 4. The three tracks

Seven gates and ten pre-committed purchase rules were a control structure for a $15K personal spending decision. With exposure now at a few dollars an hour, that scaffolding is overhead. Both twins independently recommended drastic collapse. Three tracks replace it.

### Track A. Sovereign Proof (the product claim)

**This runs first. Everything else is subordinate to it.** A control-motivated prospect does not care about tok/s if the system phones home. If the orchestration layer cannot run in a vacuum, there is no product.

**Why this still leads even though almost nobody buys an air-gapped appliance.** The narrow-segment objection is correct and it does not demote this track, because the deliverable is not appliance marketing. Egress proof is **portable evidence**. The same packet capture that would satisfy a true air-gap buyer is what a control-motivated firm asks for when deploying into its own Bedrock or Vertex VPC, which is the realistic engagement. Proving the stack has no hidden phone-home is a claim we need in every deployment conversation, sovereign or not. It also happens to be the cheapest track to run, so the evidence with the widest reuse is also the least expensive to produce.

1. **A0. Plumbing, CPU-only, ~$0.50.** Docker Compose, NATS, Triumvirate daemon, mock vLLM. Proves orchestration independent of inference.
2. **A1. Air-gap proof, ~$2-5.** Formerly Gate 6. Full egress lockdown, then run the agent swarm and prove zero outbound traffic.

Deliverable is the artifact a prospect asks to see:
- **Packet capture or VPC firewall drop logs** showing zero egress, timestamped and signed.
- The exact firewall and VPC Service Controls configuration that produced it, reproducible by the client's own security team.
- A named failure list: what the stack *tried* to reach and was denied (telemetry endpoints, model registries, package mirrors). Prospects trust a document that names its own leaks more than one claiming perfection.

### Track B. Sizing Matrix (the scale steps)

Not five bespoke gates. **One parameterized sweep** across the rented catalog, producing a priced performance table. The scale steps survive as rows in that table rather than as ceremonies with their own runbooks.

Rows to sweep, cheapest first: 1x L4 · fractional G4 · 1x G4 (96GB) · 1x A100 80GB (RunPod first, GCP only if the client needs GCP) · multi-GPU worker pool · rented Apple Silicon (M3 Ultra 256GB today, M5 Ultra 512GB after late October).

Per row, measure what a buyer actually asks about:
- **P50/P95/P99 latency under concurrency**, not single-stream hero numbers. Ten simultaneous analysts is the realistic question.
- Tokens/sec per stream and aggregate, at a stated quantization and context length.
- VRAM headroom at target concurrency.
- **Cost per 1M tokens** at that config, which is the number that goes in a proposal.

### Track C. Pilot Operations (entirely missing before)

The old plan was a benchmarking suite with no ability to host a client. Required before any pilot touches client data:

- **Ingestion and destruction.** How client data enters, and how we *prove* disks were wiped and weights purged at termination. A certificate of destruction is a deliverable.
- **Access.** IAP or dedicated VPN. Not SSH and local port forwards.
- **Lifecycle.** Track A and B VMs self-destruct. A pilot must stay up during business hours. These are opposite lifecycle policies and need separate tooling.
- **Tenancy and exit.** Isolation between pilots, and a portability story: what the client keeps if they walk.

---

## 5. Blockers: what must be built before anything runs

Ordered by whether it costs real money or loses real data if executed as written.

1. **Build the layer-6 kill function, or delete the claim.** No source exists. A runaway VM is currently stopped only by `--max-run-duration`. Claiming six layers when four are real is the dangerous kind of wrong.
2. **Implement the remote timeout (layer 4).** A hung gate script currently runs until VM max duration.
3. **Fix `runner-wrapper.sh:192`** (`local` outside a function). It fires on the first run.
4. **Create `configs/`** and the per-track `.env` files the runner requires but which do not exist.
5. **Write the actual workload scripts.** The runner expects `/tmp/gate-test.sh` or `gs://pantheon-runners/gate-N-test.sh`. No local source exists for either.
6. **Fix cost accounting.** Remove the `e2-standard-4` / 1.0-hour fallbacks in `cost-tracker.py:219` and `finalize-evidence.py:247`. A cost report that silently invents a cheap machine type is worse than no cost report, and it is client-facing.
7. **Fix `request-quotas.sh:15`** to target the real project.
8. **Reconcile fixture names** across the three documents, then build the corpora. Or scope Track A to need none, which is the faster path.
9. **Correct machine types.** `g4-standard-32` does not exist. Verify accelerator flags per machine series: G2, G4, and A2 bundle GPUs by machine type, so blanket `--accelerator` handling copied from N1 is wrong.
10. **Scope `kill-switch.sh --nuclear`.** It deletes all VMs in the project. Safe in a dedicated project, destructive in a shared one, which is an argument for the dedicated project.

**Note on spend controls:** `--max-run-duration` and `--instance-termination-action=DELETE` are still valid and still work, but they delete *instances*. They do not touch disks, snapshots, images, Artifact Registry, or buckets. Orphaned disks bill quietly. The preflight inventory check only looks for `status:RUNNING`, so it misses exactly those.

---

## 6. Where to start

The correct first move is not a GPU. It is Track A0 plus the blocker list, because a $0.50 CPU run exercises the entire harness and every claim in section 5 fails cheaply there rather than expensively on an A100.

1. Create the dedicated project and link billing (`gcloud billing accounts list` shows `01F713-7EFFD2-83E164`, open). A dedicated project is what makes the nuclear kill-switch safe.
2. File GPU quota requests immediately, since approval is not instant and gates nothing else. Note that all new projects start at **zero** GPU quota and need both a regional quota and a matching `GPUS_ALL_REGIONS`.
3. Work the blocker list against Track A0 on CPU only.
4. Run Track A1 (air-gap) on the cheapest capable instance. **This is the deliverable that matters.** Everything in Track B is a pricing exercise; Track A1 is the product.

Open question worth settling before Track B: the old plan assumed the whole budget was absorbed by a $100/month Gemini Ultra GCP credit. That assumption is unverified and four months old. Confirm it before planning around it.

---

## 7. Related documents

- `docs/pantheon/local-inference-buy-vs-rent.md` (section 6 is the standing policy)
- `docs/advisory/claude-deployment-options.md` (which door a privacy-sensitive client goes through)
- `docs/advisory/ria-compliance-intake.md` (first worked client example)
- `10-PREFLIGHT.md`, `20-EVIDENCE-BUNDLE-SPEC.md`, `30-DECISION-RULES.md` in this directory. **All three still carry the purchase-era spine and need the same treatment as this file.**
