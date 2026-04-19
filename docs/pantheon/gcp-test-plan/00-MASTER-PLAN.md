# Pantheon GCP Test Plan — Master Document

**Status:** canonical · actively maintained
**Owner:** Mike Boscia
**Created:** 2026-04-18
**Purpose:** The single source of truth for how Pantheon gets tested, validated, and sized on GCP before any local hardware is purchased.

---

## What this document is

An **executable test plan** — not a design sketch, not a research summary. Every gate in this document has:

- A hypothesis with pre-committed decision rules
- A concrete GCP configuration (exact machine type, region, image)
- A runbook with literal `gcloud` commands
- A measurement harness (scripted, deterministic)
- Hard-capped duration + budget with kill-switches
- An evidence bundle spec (what lands in GCS)
- A verdict protocol (PASS / FAIL / INCONCLUSIVE)

If you can execute every gate here and produce evidence bundles for all of them, Pantheon is validated end-to-end on real hardware, and the hardware-purchase decisions are de-risked with data instead of vibes.

---

## Philosophy

### 1. OPEX-first

We rent GCP compute until usage crosses $1000/mo sustained for 2 consecutive months. We do NOT buy local hardware speculatively. Every test is metered, time-boxed, and self-destructs.

### 2. Evidence-based decisions

Every hardware purchase (3090 single vs pair, RTX Pro 6000, Mac Studio, etc.) has a pre-committed decision rule defined BEFORE the test runs. No post-hoc rationalization.

### 3. Self-destructing VMs

Every gate's runner ends with `gcloud compute instances delete --quiet`. Every VM is created with `--max-run-duration=Nm --instance-termination-action=DELETE` as a hard backstop. No VM runs longer than its budgeted duration, ever.

### 4. Evidence bundles are mandatory output

Every gate produces exactly one bundle at `gs://pantheon-evidence/{gate_id}/{run_id}/` containing manifest, logs, metrics, artifacts, cost report, summary. Nothing is ever lost.

### 5. Knowledge compounds via capture

Every run auto-populates an Obsidian note from template via Supabase sync. Promoted lessons become durable wisdom. Every subsequent run has access to every prior run via Pythia semantic search.

---

## Gate progression at a glance

```
┌──────────────────────────────────────────────────────────────────────┐
│ PHASE 0 — Preflight (no GPU burn)                                    │
│   GCP project setup, quota verification, pre-baked images,           │
│   model weights cached, evidence bundle infrastructure.              │
│   Cost: ~$6-12 one-time + ~$15/mo ongoing storage                    │
├──────────────────────────────────────────────────────────────────────┤
│ GATE 0 — Plumbing (CPU-only, $0.50)                                  │
│   Docker Compose + NATS + Triumvirate daemon + mock vLLM.            │
│   Proves orchestration layer independent of inference.                │
├──────────────────────────────────────────────────────────────────────┤
│ GATE 1 — Single L4 baseline (1× L4 24GB, $3-5)  [3090-single proxy]  │
│   Pythia embeddings + small-model inference + 14B LoRA.              │
│   Answers: "Is 24GB sufficient for daily dev baseline?"              │
├──────────────────────────────────────────────────────────────────────┤
│ GATE 2 — Dual L4 (2× L4 48GB, $4-6)  [3090-pair proxy]  ★ DECISIVE   │
│   70B-Q4 local inference + concurrent multi-model hosting +          │
│   32B LoRA training + sovereign demo dry-run.                        │
│   Answers: "Single 3090 or 2× 3090 NVLink?" — THE purchase decision. │
├──────────────────────────────────────────────────────────────────────┤
│ GATE 3 — RTX Pro 6000 Hardware Twin (1× G4, $3-6)                    │
│   Target hardware for Phase 3 purchase. Measures actual              │
│   target-card behavior on our workload.                              │
├──────────────────────────────────────────────────────────────────────┤
│ GATE 4 — Athena-scale worker pool (4× A100 80GB, $15-30)             │
│   Parallel worker swarm at production capacity.                      │
│   Validates "original intent" at real speeds.                        │
├──────────────────────────────────────────────────────────────────────┤
│ GATE 5 — Full trinity (8× A100 + 4× A100 + 2× L4, $20-40)            │
│   Zeus + Athena + Vulcan as spec'd, end-to-end load.                 │
│   Empirical data for Pantheon Closet / Rack tier purchase decision.  │
├──────────────────────────────────────────────────────────────────────┤
│ GATE 6 — Air-gap sanity (any tier + firewall lockdown, $2-5)         │
│   Prove 100% sovereign — no outbound traffic, no leaks.              │
│   Required before any customer sovereign demo.                       │
├──────────────────────────────────────────────────────────────────────┤
│ GATE 7 — Soak + stress (repeated runs, ~$50-100)                     │
│   Long-session KV drift, schema-validity decay, retry storms.        │
│   Validates operational stability under sustained load.              │
└──────────────────────────────────────────────────────────────────────┘
```

---

## The 3090 decision — where this plan pays off immediately

The near-term reason to execute this plan is the **1× 3090 vs 2× 3090 NVLink** local hardware decision (~3-6 weeks out).

**GCP L4 is a faithful 3090 proxy:**

| Local option | GCP equivalent | VRAM | Spot $/hr |
|---|---|---|---|
| 1× RTX 3090 24GB | g2-standard-4 (1× L4 24GB) | 24GB | ~$0.28 |
| 2× RTX 3090 NVLink 48GB | g2-standard-24 (2× L4 48GB, PCIe) | 48GB | ~$0.42 |

**Differences that transfer:** tok/s per card, VRAM footprint, concurrent-model behavior, training time for LoRAs, Pythia embedding throughput, inference quality.

**Differences that don't transfer:** NVLink intra-card bandwidth (L4 pairs go over PCIe, 3090 NVLink is direct). Only matters for tensor-parallel inference on a single model split across cards. ~10-20% performance delta, doesn't affect most workloads.

**After Gates 1 + 2, you know which config to buy** with ~$10 in total GCP spend.

---

## Budget envelope

### One-time investment
| Item | Cost |
|---|---|
| Preflight setup (storage, PD snapshots, image baking) | $6-12 |
| Full gate sequence (Gates 0-5) | $50-100 |
| **Total** | **$56-112** |

### Ongoing (monthly, during active testing)
| Item | Cost |
|---|---|
| GCS storage (model weights, evidence bundles) | $5-8 |
| Artifact Registry (Docker images) | $1-2 |
| PD snapshots (model caches) | $10-15 |
| Repeated test runs as Pantheon develops | $30-100 |
| **Total** | **~$50-125/mo** |

**Entirely absorbed by Gemini Ultra GCP credit ($100/mo).** Effective cost to Mike: $0-25/mo.

---

## Pre-committed decision rules

Lock these BEFORE running tests. Evidence-based decisions require pre-committed rules.

### Decision 1 — Single 3090 vs 2× 3090 NVLink
*(Triggered by completion of Gates 1 + 2)*

**Buy 2× 3090 NVLink if ALL:**
- Gate 2 shows 70B-Q4 local inference at ≥10 tok/s per stream
- Gate 2 shows concurrent multi-model hosting works without contention
- Gate 2 shows 32B LoRA completes training in ≤4 hours
- Friction log shows ≥5 events/week where 48GB matters

**Buy single 3090 only if:**
- Gate 1 shows 24GB is comfortable for Pythia + small model inference
- Gate 2 shows 70B-local is too slow to be useful (<5 tok/s per stream)
- 7-14B LoRA base is sufficient for your moat-building needs

**Skip local hardware entirely if:**
- Friction log shows <3 events/week where ANY local GPU helps
- Pre-bake tooling makes GCP spin-up feel frictionless (<90s perception)

### Decision 2 — RTX Pro 6000 Blackwell purchase
*(Triggered by usage data over 2+ months of operation)*

**Buy RTX Pro 6000 if ANY:**
- GCP spend >$1000/mo on consistent workload for 2 consecutive months (OPEX crossover)
- Signed customer engagement with line-item hardware in contract
- Training workload demonstrably bottlenecked by 3090 pair for 3+ consecutive weeks
- Enterprise/sovereign demo requires 70B-at-production-speed on premise

### Decision 3 — 2nd RTX Pro 6000 / scale-out
*(Triggered by operational evidence post-RTX-Pro-6000-purchase)*

**Buy 2nd RTX Pro 6000 if ANY:**
- Training blocks production serving for 3+ consecutive weeks
- Multiple paying customers cause scheduling contention
- Enterprise deal funds the purchase as line-item

---

## Directory structure

```
/Users/mikeboscia/projects/triumvirate/docs/pantheon/gcp-test-plan/
├── 00-MASTER-PLAN.md              ← this file
├── 10-PREFLIGHT.md                ← GCP setup, pre-bake, storage
├── 20-EVIDENCE-BUNDLE-SPEC.md     ← what every run emits
├── 30-DECISION-RULES.md           ← pre-committed gates + rules
├── runbooks/
│   ├── gate-0-plumbing.md         ← CPU-only Docker + Triumvirate
│   ├── gate-1-single-l4.md        ← 3090-single proxy
│   ├── gate-2-dual-l4.md          ← 3090-pair proxy ★ DECISIVE
│   ├── gate-3-rtx-pro-6000.md     ← RTX Pro 6000 hardware twin
│   ├── gate-4-athena-swarm.md     ← 4× A100 parallel workers
│   ├── gate-5-full-trinity.md     ← 8×A100 + 4×A100 + 2×L4
│   ├── gate-6-airgap-sanity.md    ← sovereign validation
│   └── gate-7-soak-stress.md      ← long-session stability
├── fixtures/
│   ├── test-tasks-pythia-embed/   ← canonical embedding test corpus
│   ├── test-tasks-agent-swarm/    ← 8 canonical agent tasks
│   ├── test-tasks-lora-train/     ← LoRA training datasets
│   └── eval-scorers/              ← scoring rubrics + harness
├── harness/
│   ├── runner-wrapper.sh          ← provision → run → capture → destroy
│   ├── cost-tracker.py            ← GCP billing API → cost report
│   ├── evidence-bundler.sh        ← log + metric → GCS bundle
│   ├── kill-switch.sh             ← emergency VM teardown (all regions)
│   └── metric-collectors/         ← per-workload measurement scripts
└── evidence-templates/
    ├── manifest.json.template     ← per-run metadata schema
    ├── summary.md.template        ← human-readable run summary
    ├── cost-report.json.template  ← GCP spend breakdown
    └── obsidian-note.md.template  ← auto-generated Obsidian vault note
```

---

## Execution order for first-time setup

```
DAY 1 (4-6 hours, $0 GPU):
  [ ] Complete 10-PREFLIGHT.md sections 1-3 (GCP project, quota, IAM)
  [ ] GCP quota increase request filed for A100s
  [ ] Artifact Registry + GCS buckets created
  [ ] Gemini Ultra credit applied and verified

DAY 2-3 (6-8 hours, ~$6-12 GCP):
  [ ] Complete 10-PREFLIGHT.md sections 4-7 (pre-baked images, model cache, PD snapshots)
  [ ] Evidence bundle tooling deployed
  [ ] Cost tracker verified
  [ ] harness/runner-wrapper.sh tested on a dummy run

DAY 4 ($0.50 GPU, 1-2 hours):
  [ ] Execute Gate 0 (plumbing)
  [ ] Evidence bundle lands in GCS
  [ ] Obsidian note auto-generated

DAY 5-7 ($3-6 GPU, 4-8 hours):
  [ ] Execute Gate 1 (single L4)
  [ ] Execute Gate 2 (dual L4) ← 3090 DECISION data collected
  [ ] Apply pre-committed decision rule

WEEK 2 ($15-30 GPU, 4-8 hours):
  [ ] Execute Gate 3 (RTX Pro 6000 twin)
  [ ] Execute Gate 4 (Athena-scale)
  [ ] Evidence suite complete for Pantheon Desk purchase decisions

WEEK 3-4 ($20-40 GPU, as needed):
  [ ] Execute Gate 5 (full trinity) — optional, informs Pantheon Closet/Rack tier
  [ ] Execute Gate 6 (air-gap) — required before any sovereign customer demo
  [ ] Execute Gate 7 (soak/stress) — required for production confidence
```

---

## Kill-switches — how this CAN'T run up an unexpected bill

Six layers of spend control, in order of defense:

1. **`--max-run-duration=Nm` on every VM create** — GCP forcibly deletes VM after duration
2. **`--instance-termination-action=DELETE`** — no "stopped but billable" state possible
3. **`trap 'gcloud ... delete' EXIT` in runner** — VM self-deletes on script exit
4. **`timeout Nm python3 runner.py`** — test process killed at hard deadline
5. **Pre-flight inventory check** — can't start a gate if other VMs are live
6. **Billing alerts at $10, $30, $50 with PubSub auto-kill at $50**

The nuclear backstop (layer 6) fires a Cloud Function that runs `gcloud compute instances list | xargs delete` across all regions if total billing hits $50 unexpectedly. Setup documented in `10-PREFLIGHT.md`.

---

## What success looks like at the end of this plan

After executing all 7 gates, you have:

1. **Empirical validation** that Pantheon's architecture works on each target hardware tier from "consumer" to "enterprise"
2. **Evidence bundles** for every gate in `gs://pantheon-evidence/` — immutable, queryable
3. **Pre-committed decision rules applied** — each hardware purchase is supported by concrete measurements
4. **First LoRA adapters** trained and scored (moat construction underway)
5. **Pythia corpus seeded** with initial codebases + evaluation rubrics
6. **Obsidian vault populated** with run notes, lessons, decisions, hypotheses
7. **Operational tooling hardened** — `runner-wrapper.sh`, kill-switches, cost tracker all battle-tested
8. **The $15K RTX Pro 6000 purchase** (if triggered) is de-risked at 1:1 hardware parity
9. **Sovereign demo capability** is provably air-gap-clean
10. **A permanent knowledge archive** of what every tier of GPU spend actually delivers

---

## What comes next

This document is the index. The details live in per-gate runbooks in `runbooks/`. Start with `10-PREFLIGHT.md` before touching any GPU, then proceed gate-by-gate.

**Next file to read: `10-PREFLIGHT.md`.**
