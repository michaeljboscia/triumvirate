# Graduated GCP Validation Plan — PANTHEON + YellingToad

**Purpose:** Validate the PANTHEON AI Software Factory architecture on GCP before committing $20K to local hardware. Six escalating tiers ($0.13/hr → $4.60/hr) that prove increasingly ambitious claims using real code (Go, Rust, Python) and real project deliverables (YellingToad + Tellus LandOS). No Kubernetes on GCP — plain VMs + Docker Compose. Each tier's infrastructure carries forward; nothing gets torn down.

**Created:** 2026-04-16
**Architecture reference:** `/Users/you/PANTHEON_ARCHITECTURE.md`
**Hardware specs:** `/Users/you/projects/triumvirate/docs/vulcan-1-build-spec.md`
**YellingToad Go rewrite:** `/Users/you/gtm-machine-infrastructure/yelling-toad/go/`
**Triumvirate Rust daemon:** `/Users/you/projects/triumvirate/daemon/`

> **Hardware premise retired (2026-08-23).** The "$20K to local hardware" framing below reflects April 2026 pricing and a
> purchase that is no longer planned. The DRAM/GDDR7 shortage repriced every path (512GB M3 Ultra Studio: a $23-26K
> used-market item Apple no longer sells; 2x RTX PRO 6000: $36-43K, not $30K), and measured throughput did not justify
> either box regardless of price. Standing policy is rent first, always, with owned metal only as a customer-funded
> terminal step. See `docs/pantheon/local-inference-buy-vs-rent.md`, section 6 for the policy.
>
> The tiers and methodology below still stand. Read them as a catalog of rentable configurations for client pilots and as
> evidence generation for sovereign-build quotes, not as a runway to a purchase.

---

## Design principles

1. **No Kubernetes on GCP.** Docker Compose on GCE VMs. SSH + `docker compose up`. K8s only earns its keep on physical Athena (2× DGX Spark, multi-node GPU scheduling). For validation, compose is simpler and cheaper.

2. **Nothing gets torn down.** Each tier adds a VM or a container to the existing setup. The GCE VMs, Docker images, model caches, and service discovery from Tier 0 are still running at Tier 5. No "rebuild from scratch" between tiers.

3. **Real code, real deliverables.** Every tier produces shippable code in Go (YellingToad), Rust (Triumvirate), or Python (test harness, Pythia, Prefect). The test payloads are real project tasks, not synthetic benchmarks.

4. **Graduated GPU spend.** Debug all non-GPU problems (Docker, networking, service discovery, NATS, Git worktrees) on $0.13/hr CPU VMs before touching $1.12/hr GPU VMs. Every dollar of GPU spend goes to testing GPU-specific questions.

5. **Full-scale test runs ≤ 1 hour.** By Tier 5, the infrastructure is proven and the only variable is "does the full trinity perform at scale." One hour of runtime at ~$4.60/hr = ~$5. Total cumulative spend across all tiers: ~$40-60.

---

## Production hardware topology (what we're validating toward)

```
┌─────────────────────────────┐
│  Zeus (Mac Studio M5 Ultra) │
│  ollama / llama.cpp native  │
│  Llama 405B on Metal        │
│  512GB unified memory       │
│  No containers              │
└──────────┬──────────────────┘
           │ HTTP :11434
           │
┌──────────▼──────────────────┐
│  Server / Orchestrator     │
│  Triumvirate daemon (Rust)  │
│  Pythia SQLite + embeddings │
│  Docker (no k3s)            │
└──────────┬──────────────────┘
           │ HTTP :8000 (vLLM OpenAI-compat)
     ┌─────┴──────┐
     │            │
┌────▼────────┐  ┌▼────────────┐
│ Athena DGX-1│  │ Athena DGX-2│
│ k3s server  │  │ k3s agent   │
│ 4-25 vLLM   │  │ 4-25 vLLM   │
│ worker pods │  │ worker pods │
│ NVIDIA GPU  │  │ NVIDIA GPU  │
│ plugin      │  │ plugin      │
└─────────────┘  └─────────────┘
           │
     ┌─────┴──────┐
┌────▼────────────┐
│ Vulcan (PC)     │
│ docker compose  │
│ GPU 0: 32B coder│
│ GPU 1: diffusion│
└─────────────────┘
```

### Why k3s on Athena only

| Node | Runtime | Why |
|---|---|---|
| **Zeus** (Mac Studio) | Native ollama/llama.cpp on Metal | macOS — k3s doesn't run natively. Container layer costs memory bandwidth. 405B needs direct Metal access to 512GB unified memory. |
| **Athena** (2× DGX Spark) | **k3s** (server on DGX-1, agent on DGX-2) | Multi-node GPU scheduling, NVIDIA device plugin enforces allocation, pod health checks, rolling model updates, `kubectl scale` for worker count. k3s overhead: ~512MB RAM, negligible CPU. |
| **Vulcan** (RTX 3090 workstation) | Docker Compose | Single machine, 2 GPUs, 2-3 containers max. Compose is 30 lines. k3s adds control plane overhead for zero benefit on a single node. |
| **Server** (orchestrator) | Docker Compose | Triumvirate + Pythia + support services. Single machine, no GPU. |

### GCP validation uses Docker Compose everywhere

The GCP tiers below use compose on GCE VMs. The vLLM container images and Triumvirate HTTP backend code are identical between GCP-compose and production-Athena-k3s. Only the orchestration layer underneath changes.

---

## GCP validation tiers

### Tier 0 — YellingToad Go on GCE ($0.13/hr)

**Question:** Can we build, deploy, and run the YellingToad Go rewrite on a GCE VM with Docker Compose?

**Why first:** Shakes out all non-GPU deployment problems — Docker multi-stage builds, compose networking, NATS embedded messaging, persistent volumes, log aggregation, crash restart — on the cheapest possible VM. Every problem fixed here is a problem that doesn't burn GPU dollars later.

#### Infrastructure

| Component | GCE machine type | Cost (Spot) |
|---|---|---|
| Orchestrator + YT coordinator + 2 YT render workers | `e2-standard-4` (4 vCPU, 16GB RAM) | $0.13/hr |

#### Code to write (Go)

- Finish YellingToad coordinator binary (`go/cmd/yellingtoad/main.go`)
- NATS JetStream embedded broker for job dispatch (`go/internal/render/broker/`)
- Render worker that consumes from NATS, fetches pages via rod+Chromium, returns crawl results
- Dockerfile: multi-stage build, coordinator + worker from same binary (flag-switched: `--mode=coordinator` vs `--mode=worker`)
- `docker-compose.yml` with coordinator (1 replica) + worker (2 replicas) + NATS

#### Success criteria

| # | Criterion |
|---|---|
| 1 | `docker compose up` starts all 3 containers on GCE VM |
| 2 | Coordinator dispatches 10 URLs via NATS to workers |
| 3 | Workers return CrawlPage results to coordinator |
| 4 | Coordinator writes results to Supabase (port 5432 COPY) |
| 5 | Worker crash → compose restarts it → picks up next job from NATS |

#### What carries forward
- GCE VM (stays running for subsequent tiers)
- Docker registry (Artifact Registry) with YT images
- `docker-compose.yml` pattern (extended in later tiers)
- Proven NATS messaging pattern (Triumvirate may reuse for worker dispatch)

---

### Tier 1 — Add Triumvirate + vLLM smoke test, no GPU ($0.13/hr)

**Question:** Can Triumvirate dispatch inference requests to a vLLM endpoint and receive valid completions?

**Why no GPU:** vLLM can run in CPU-only mode with a tiny model (e.g., TinyLlama 1.1B). The goal isn't speed — it's proving the HTTP contract between Triumvirate and vLLM works end-to-end.

#### Infrastructure

| Component | Where | Cost |
|---|---|---|
| Everything from Tier 0 + Triumvirate container + vLLM (CPU) container | Same `e2-standard-4` VM | $0.13/hr (unchanged) |

#### Code to write (Rust + Python)

**Rust (Triumvirate):**
- New module: `daemon/crates/triumvirate/src/inference/vllm.rs`
- Config field: `inference_backend: "vllm"` with `vllm_endpoints: ["http://vllm:8000/v1"]`
- OpenAI-compatible HTTP client: POST `{endpoint}/v1/chat/completions` with model name, messages, temperature
- Response parser: extract `choices[0].message.content`
- Integration with existing worker-dispatch pipeline: when a task needs inference, route to vLLM instead of ollama

**Python (test harness):**
- `scripts/pantheon/run_smoke_test.py`: send 5 prompts to vLLM via Triumvirate's dispatch API, verify 5 completions return, validate response schema

#### Success criteria

| # | Criterion |
|---|---|
| 6 | vLLM container starts in CPU mode with TinyLlama |
| 7 | Triumvirate sends a prompt, receives a completion via HTTP |
| 8 | Round-trip latency < 30s (CPU inference is slow; just proving the plumbing) |
| 9 | 5/5 smoke-test prompts return valid completions |

#### What carries forward
- vLLM Docker image (swap model weights + add GPU later)
- Triumvirate vLLM backend (same code path for all future tiers)
- `docker-compose.yml` now has 5 containers (YT coordinator, 2 YT workers, Triumvirate, vLLM)

---

### Tier 2 — First real GPU: 1× L4 with Qwen 32B ($0.28/hr)

**Question:** Does the vLLM container successfully attach to a GPU, load a real model, and serve inference?

**Why Qwen 32B:** Fits on a single L4 (24GB) at INT4 quantization. Tests the GPU path without multi-GPU tensor parallelism complexity.

#### Infrastructure

| Component | GCE machine type | Cost (Spot) |
|---|---|---|
| GPU VM | `g2-standard-4` (1× L4, 24GB VRAM) | $0.28/hr |
| Orchestrator VM (from Tier 0) | `e2-standard-4` | $0.13/hr |
| **Tier total** | | **$0.41/hr** |

#### Code to write

- Model cache setup: download Qwen 32B INT4 (AWQ) to GCS bucket, mount as persistent disk on GPU VM
- Update `docker-compose.gpu.yml`: vLLM with `--tensor-parallel-size 1 --gpu-memory-utilization 0.95`
- Update Triumvirate config: `vllm_endpoints` points at GPU VM's internal IP

#### Success criteria

| # | Criterion |
|---|---|
| 10 | vLLM loads Qwen 32B INT4 on L4 without OOM |
| 11 | Inference latency < 5s for a 500-token completion |
| 12 | Triumvirate dispatches a real coding task ("write a Python function that..."), gets valid code back |

#### What carries forward
- GPU VM instance (add more GPUs in Tier 3)
- Model-loading pipeline (GCS → persistent disk → vLLM)
- Proven: CUDA drivers, NVIDIA container toolkit, GPU scheduling all work

---

### Tier 3 — Athena swarm: 2 workers, parallel worktrees ($1.12/hr)

**Question:** Can Triumvirate dispatch multiple coding tasks to multiple vLLM workers simultaneously, each operating in its own Git worktree, producing mergeable code?

**This is the core PANTHEON thesis test.**

#### Infrastructure

| Component | GCE machine type | Cost (Spot) |
|---|---|---|
| GPU VM (upgraded) | `g2-standard-48` (4× L4, 96GB VRAM) | $1.12/hr |
| Orchestrator VM | `e2-standard-4` | $0.13/hr |
| **Tier total** | | **$1.25/hr** |

#### Code to write (Rust + Python)

**Rust (Triumvirate):**
- Parallel worktree dispatch: create N Git worktrees, assign one per task, dispatch prompts to vLLM with per-worktree Pythia context
- Result collection: poll for completions, write output files to worktrees, run validation commands
- Merge orchestration: after all workers complete, attempt sequential merge to test branch

**Python (Pythia + test harness):**
- `scripts/pantheon/export_pythia_corpus.py`: export Tellus LandOS Pythia corpus to portable `.db` file, upload to GCS, download on orchestrator VM
- `scripts/pantheon/run_swarm_test.py`: dispatch 4 real Tellus tasks simultaneously, collect results, validate, produce pass/fail report

#### Model configuration
- Qwen 72B INT4 (AWQ), TP=4 across 4× L4 GPUs
- vLLM: `--tensor-parallel-size 4 --max-model-len 8192 --gpu-memory-utilization 0.95`
- 2 concurrent inference slots (vLLM continuous batching handles this)

#### Test tasks (4 parallel, all against Tellus LandOS)

1. Write `screen_datacenter_v1()` SQL function
2. Write `filter_water_sewer_nc_v3.py` with extent verification
3. Write residential tract screening rubric v0.1
4. Write a Prefect flow for NC OneMap parcel refresh

#### Success criteria

| # | Criterion |
|---|---|
| 13 | All 4 worktrees created without conflict |
| 14 | All 4 vLLM requests complete (no OOM, no timeout) |
| 15 | Pythia context injection produces correct import paths in generated code |
| 16 | ≥3 of 4 outputs pass basic validation (`tsc --noEmit` / `python3 -c "import ..."`) |
| 17 | No merge conflicts when merging all 4 worktrees to test branch |
| 18 | Total wall-clock < 30 min for all 4 tasks |

#### Verdict
- 5/6 pass → Athena swarm thesis validated, proceed to Tier 4
- 3-4/6 pass → architecture works but needs tuning (context injection or worktree isolation)
- <3/6 pass → fundamental issue; investigate before spending more

---

### Tier 4 — Full trinity: Zeus + Athena + Vulcan ($3.30/hr)

**Question:** Does the Zeus review loop drive quality convergence? Does Vulcan's "syntax fixer" pattern reduce worker retry cycles?

#### Infrastructure

| Component | GCE machine type | Cost (Spot) |
|---|---|---|
| Athena (from Tier 3) | `g2-standard-48` (4× L4) | $1.12/hr |
| Zeus | `a2-ultragpu-1g` (1× A100 80GB) | $2.18/hr |
| Vulcan | Reuse orchestrator VM (vLLM CPU-mode with 32B, or attach a T4) | ~$0.00 (reuse) or $0.11 (T4 Spot) |
| Orchestrator | `e2-standard-4` | $0.13/hr |
| **Tier total** | | **~$3.43/hr** |

#### Code to write (Rust)

**Triumvirate review loop:**
- Zeus endpoint receives: worker output + test results + Pythia context
- Zeus returns structured decision: `{"verdict": "APPROVE"|"REJECT", "feedback": "...", "confidence": 0.0-1.0}`
- On REJECT: worker receives feedback, regenerates, resubmits (max 3 cycles)

**Vulcan fast-fix path:**
- When a worker's output fails validation (test error, type error, syntax error), Vulcan gets ONLY the error message + 20 lines of surrounding code
- Vulcan returns a targeted patch (diff format, not full rewrite)
- If Vulcan's fix passes validation, worker moves to next task (fast path)
- If Vulcan's fix fails, escalate to Zeus review (slow path)

#### Model configuration
- Zeus: Llama 70B BF16 on A100 80GB (stand-in for production 405B on Zeus hardware)
- Athena: Qwen 72B INT4 on 4× L4 (same as Tier 3)
- Vulcan: Qwen 32B INT4 on CPU (slow but functional) or on a T4 if attached

#### Success criteria

| # | Criterion |
|---|---|
| 19 | Zeus produces structured APPROVE/REJECT decisions (not hallucinated prose) |
| 20 | ≥2 rejected outputs get successfully reworked on second attempt |
| 21 | Vulcan fixes ≥50% of syntax/type errors without escalating to Zeus |
| 22 | Average review cycles < 3 per task |
| 23 | End-to-end (dispatch → all approved → merged) < 60 min |

---

### Tier 5 — Full-scale 1-hour blast ($4.60/hr)

**Question:** Does throughput scale with worker count? Is the architecture bottlenecked anywhere?

#### Infrastructure

| Component | GCE machine type | Cost (Spot) |
|---|---|---|
| Athena-A | `g2-standard-48` (4× L4) | $1.12/hr |
| Athena-B | `g2-standard-48` (4× L4) | $1.12/hr |
| Zeus | `a2-ultragpu-1g` (1× A100 80GB) | $2.18/hr |
| Vulcan | `g2-standard-4` (1× L4) | $0.28/hr |
| Orchestrator | `e2-standard-4` | $0.13/hr |
| **Tier total** | | **$4.83/hr** |

**Duration: exactly 1 hour. Budget: ~$5.**

#### Test scenario

- 8 real tasks dispatched simultaneously (expand task list to 8 Tellus + YellingToad items)
- 4 workers across 2 GPU VMs (2 per VM, continuous batching)
- Zeus reviews all outputs
- Vulcan intercepts test failures
- Measure: PRs/hour, cost/PR, worker utilization, Zeus bottleneck analysis

#### Success criteria

| # | Criterion |
|---|---|
| 24 | 8 workers produce output without GPU OOM across nodes |
| 25 | Vulcan fixes ≥50% of failing tests without escalating to Zeus |
| 26 | Wall-clock throughput ≥3× vs Tier 3 (8 tasks in similar time as 4 tasks) |
| 27 | Total cost for 8-task run < $10 |
| 28 | No single component (Zeus review, Pythia lookup, Git merge) is >40% of wall-clock |

---

## Code deliverables summary (by language)

### Go (YellingToad) — Tiers 0, 5

| Deliverable | Tier | Location |
|---|---|---|
| Coordinator binary (NATS dispatch, job management) | 0 | `yelling-toad/go/cmd/yellingtoad/` |
| Render worker (rod + Chromium, NATS consumer) | 0 | `yelling-toad/go/internal/render/` |
| Dockerfile (multi-stage, flag-switched coordinator/worker) | 0 | `yelling-toad/Dockerfile.go` |
| docker-compose.yml (coordinator + N workers + NATS) | 0 | `yelling-toad/docker-compose.yml` |

### Rust (Triumvirate) — Tiers 1, 3, 4

| Deliverable | Tier | Location |
|---|---|---|
| vLLM HTTP inference backend | 1 | `triumvirate/daemon/crates/triumvirate/src/inference/vllm.rs` |
| Config: `inference_backend` + `vllm_endpoints` | 1 | `triumvirate/daemon/crates/triumvirate/src/config/` |
| Parallel worktree dispatch | 3 | `triumvirate/daemon/crates/triumvirate/src/dispatch/worktree.rs` |
| Zeus review loop (APPROVE/REJECT protocol) | 4 | `triumvirate/daemon/crates/triumvirate/src/review/` |
| Vulcan fast-fix routing | 4 | `triumvirate/daemon/crates/triumvirate/src/inference/vulcan.rs` |

### Python (glue + Pythia + test harness) — Tiers 1, 3, 5

| Deliverable | Tier | Location |
|---|---|---|
| Smoke test (5 prompts → 5 completions) | 1 | `scripts/pantheon/run_smoke_test.py` |
| Pythia corpus export + upload | 3 | `scripts/pantheon/export_pythia_corpus.py` |
| Swarm test harness (N tasks → N worktrees → validate → report) | 3 | `scripts/pantheon/run_swarm_test.py` |
| GCE setup script (create VMs, configure firewall, mount disks) | 0 | `scripts/pantheon/gce_setup.sh` |
| Model cache script (download weights to GCS) | 2 | `scripts/pantheon/cache_models.sh` |

---

## Cost summary

| Tier | What it proves | GCP cost | Dev time |
|---|---|---|---|
| **0** ($0.13/hr) | Docker + Compose + NATS + Go deploy | ~$1-2 | 4-6 hrs |
| **1** ($0.13/hr) | Triumvirate ↔ vLLM contract | ~$1-2 | 3-4 hrs |
| **2** ($0.41/hr) | GPU attach + real model loads | ~$2-4 | 1-2 hrs |
| **3** ($1.25/hr) | **Parallel swarm (core thesis)** | ~$5-10 | 3-4 hrs |
| **4** ($3.43/hr) | Zeus review + Vulcan fix loops | ~$10-15 | 2-3 hrs |
| **5** ($4.83/hr) | Full-scale 1-hour blast | ~$5 | 1 hr |
| **Total** | **Full PANTHEON validated or falsified** | **~$24-38** | **14-20 hrs dev** |

All GCE VMs use Spot pricing. VMs can be stopped between tiers (no burn when idle). Model weights cached on GCS persistent disk (don't re-download each session).

---

## Gemini Ultra credit note

Per memory `reference_gcp_gemini_ultra_credit.md`: Gemini Ultra subscription returns ~$100/mo in GCP credit. The entire 6-tier validation ($24-38) fits within a single month's credit. **This validation is effectively free.**

---

## What a PASS at Tier 5 means

You have empirical evidence that:
1. Multiple AI workers CAN produce valid, mergeable code in parallel
2. An AI reviewer CAN drive quality convergence without human intervention
3. A dedicated "syntax fixer" CAN accelerate the pipeline by handling trivial failures
4. The architecture scales with worker count (not bottlenecked on shared resources)
5. Real project deliverables (YellingToad Go code, Tellus SQL functions) are produced as a side effect of the test

**The $20K hardware purchase is de-risked.** The software works; you're buying faster/cheaper hardware to run it locally.

## What a FAIL means

- **Tier 0 fail:** Docker/networking fundamentals broken. Fix before anything else.
- **Tier 1 fail:** Triumvirate ↔ vLLM contract doesn't work. Fix the Rust HTTP backend.
- **Tier 2 fail:** GPU drivers/scheduling broken. GCE-specific issue, not architectural.
- **Tier 3 fail:** **Workers produce code that can't merge.** This is the critical finding — means Pythia context injection or worktree isolation needs redesign. Don't buy hardware.
- **Tier 4 fail:** Review loop doesn't converge. `/council` protocol needs work. Hardware still viable but review model may need to be larger.
- **Tier 5 fail:** Scaling is sub-linear. Add workers ≠ proportionally more throughput. The concurrency model needs rethinking before scaling hardware.

---

## Model selection (TBD)

Model choices per role (Zeus architect, Athena worker, Vulcan fixer) are documented separately. This plan is model-agnostic — the vLLM container accepts any HuggingFace model. Model selection is a tuning decision that happens during/after Tier 2, once GPU inference is proven to work.

Factors to evaluate per role:
- VRAM budget (determines quantization level)
- Code-generation quality (benchmark against known-good outputs)
- Context window (8K vs 32K vs 128K — affects Pythia injection strategy)
- Inference speed (tokens/sec at the chosen quantization)
- License (commercial use permitted?)

**Next document:** `docs/plans/pantheon-model-selection.md` — covers model choices, quantization trade-offs, and per-tier model progression.

---

## Relationship to other plans

- **Supersedes:** `docs/plans/pantheon-gcp-validation-plan.md` (earlier version without Tier 0 / YellingToad / k3s-topology / compose-over-k8s decisions)
- **References:** `/Users/you/PANTHEON_ARCHITECTURE.md` (the $20K hardware blueprint this validates)
- **Feeds into:** `docs/plans/pantheon-model-selection.md` (TBD — model choices per role)
- **Parallel workstream:** `docs/plans/nc-dd-data-layer-acquisition.md` (Tellus DD — some of whose tasks become Tier 3/5 test payloads)

---

*To execute: read this plan, start at Tier 0, pass each tier's success criteria before advancing. Each tier gates the next. Total calendar: 3-5 focused sessions across 1-2 weeks.*
