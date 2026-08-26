# Fast AI VM startup: strategies and precomputed cost tradeoffs

**Written:** 2026-08-26 (revised same day after further research corrected two conclusions)
**Problem:** a cold GPU node that downloads model weights from HuggingFace takes 20-30 minutes. Target is 2-3 minutes, with 5 minutes acceptable.
**Why it matters:** for a client pilot or a live demo, a 30-minute cold start is disqualifying, not a cost-optimization question.

> **Provenance.** A peer review recommended deleting the PD-snapshot phase on the grounds it "saves 3-4 minutes worth
> $0.05." That priced it against `gsutil cp` from a same-region bucket, not against a cold HuggingFace download, and
> the verdict was accepted without asking what replaces the capability. This document is the correction.

---

## 1. The headline finding: storage is not the main lever

The instinct is to fix the weights download. **The weights are not the biggest slice.** A measured breakdown of a cold
vLLM start for a 70B model:

| Phase | Naive | Optimized | What fixes it |
|---|---|---|---|
| Container image pull | 4-8 min | 10-20 sec | lazy/streaming image pull |
| Weight load into VRAM | 45-60 sec | 8-12 sec | streaming loader, GPUDirect Storage |
| **CUDA graph capture / JIT** | **30-45 sec** | 2-5 sec | persisted CUDA graphs, JIT cache |
| KV cache init | 10 sec | 2 sec | engine config |
| **Total cold TTFT** | **6-10 min** | **25-40 sec** | |

**The container image is the single largest phase, and CUDA graph capture costs 30-45 seconds with no storage
involvement at all.** Solving only the weights problem leaves most of the delay in place.

**Most of the win is free.** Streaming loaders, graph persistence, and image streaming are configuration, not spend.
Do these before paying for anything.

---

## 2. Strategy A: fix the software first (free, biggest single win)

| Lever | Flag or mechanism | Effect |
|---|---|---|
| Stream weights from object storage | `vllm serve --load-format runai_streamer` | begins moving tensors to VRAM on first byte; saturates 100Gbps links |
| Stream weights from local NVMe | `--load-format fastsafetensors` | GPUDirect Storage, direct NVMe-to-HBM DMA, **26+ GB/s vs 5-7 GB/s for naive mmap** |
| Persist CUDA graphs | write graphs to a persistent NVMe volume | 30-60 sec becomes under 2 sec |
| Snapshot post-init state | `vllm snapshot create` (CRIU-based) | vLLM ready time **180s to under 10s** |
| Pre-shard for tensor parallel | shard to N = TP size before deploy | load time becomes invariant to GPU count |

**Verify each of these against current vLLM docs before relying on the flag names.** They are recent additions and the
surface moves.

## 3. Strategy B: stop re-pulling the container image

The largest phase. Lazy pulling starts the container once a small fraction of metadata is available.

- **GKE Image Streaming:** a 15GB vLLM image goes from 3-5 minutes to **under 15 seconds**.
- **AWS SOCI:** the EKS equivalent, reportedly slightly better than eStargz for AI workloads because it does not
  require modifying image layers.
- **eStargz:** weaker for AI images; suffers I/O hangs during the Python import phase.

**On the local box and on RunPod, the simpler answer is that the image is already cached after the first pull.** This
matters most for ephemeral autoscaled nodes.

## 4. Strategy C: persistent weight storage, by provider

| Provider | Mechanism | Cost | Load time |
|---|---|---|---|
| **RunPod** | Network Volume | **$0.07/GB/mo** (running or stopped), **$0.05/GB/mo above 1TB** | 40-60 sec for 70B |
| RunPod | high-IOPS network tier | $0.14/GB/mo | faster, unquantified |
| RunPod | **Model Store** (pin a HF model to the datacenter cache) | see vendor | eliminates the download phase; "under 20 sec" with Quick Deploy |
| **Lambda Labs** | persistent block storage | **$0.20/GB/mo**, and **zero egress fees** | 2-5 min to provision an instance |
| **Modal** | persistent cache + network volumes | **$0.30/GB/mo**, per-second compute billing | **sub-1s to 5s** small, 10-60 sec large, via transparent snapshotting |
| **GCP** | Hyperdisk ML | capacity ~$0.084/GiB-mo **plus throughput**, see below | 2-5 sec attach, under 15 sec load |
| GCP | GCS FUSE with caching | ~4x cheaper than block | 60-90 sec |

**Lambda's zero egress is worth noting** given that cross-provider egress was the reason the GCS weight cache was cut.

## 5. The Hyperdisk ML trap (correcting my earlier draft)

My first draft speculated that Hyperdisk ML throughput could be provisioned high at attach and dropped after, making
it cheap for occasional runs. **I checked. It does not work that way for our usage pattern.**

Confirmed behavior:

- Throughput **can** be changed dynamically without detaching, and billing **is** hourly, prorated per second. So far
  so good.
- **But there is a 6-hour cooldown between throughput changes**, so you can adjust at most four times a day.
- **And a change takes roughly 20 minutes to take effect.**
- **And you are billed for provisioned performance even when the disk is detached or the VM is stopped.**

**For a 45-minute gate run, burst-provisioning is impossible.** You cannot raise throughput and have it take effect
inside the run, and you cannot lower it again for six hours. The economics therefore revert to continuous
provisioning: 1,700 MiB/s (roughly a 60-second load for 100 GiB) is about **$212/month, whether or not you use it.**

**Hyperdisk ML is priced for a production fleet that amortizes it across constant load. It is the wrong tool for
occasional gate runs.** Minimum throughput is 400 MiB/s, so even a parked volume carries a floor charge.

**If a client requires GCP, use GCS FUSE with caching (60-90 sec) rather than Hyperdisk ML.**

## 6. Strategy D: warm pool, when a human is watching

- **RunPod Active Workers:** run continuously, zero cold start, at a **20-32% discount** to the flex rate.
- **RunPod Flashboot:** sub-200ms on supported templates.
- **Modal:** scale-to-zero native with sub-second restores, which is warm-pool behavior without paying for idle.
- **GKE Pod Snapshots (2026):** resume from frozen memory in under 5 seconds.

**Use only around a scheduled demo, with a budget alarm.** This is the one strategy that bills for doing nothing.

## 7. Strategy E: the local box already solved this

The Lenovo (RTX 4000 Ada, 12GB) has persistent local disk. Download once, bind-mount, cold start is zero forever
after. The image also stays in the local Docker cache. **Track A has no startup problem to solve.**

---

## 8. Precomputed cost tradeoffs

**Assumptions:** 100 GB working set; RunPod A100 80GB at $1.19/hr; GCP L4 at ~$0.71/hr. Prices are vendor list figures
from August 2026 documentation and should be re-checked against a live quote.

### Monthly cost to keep 100 GB resident and fast

| Option | Monthly | Load time | Notes |
|---|---|---|---|
| Local disk (Lenovo) | **$0** | zero after first | already owned |
| RunPod Network Volume | **$7.00** | 40-60 sec | cheapest real option |
| RunPod high-IOPS | $14.00 | faster | |
| Lambda Labs persistent | $20.00 | plus 2-5 min provision | zero egress |
| Modal persistent cache | $30.00 | sub-1s to 60 sec | per-second compute |
| GCS bucket (FUSE source) | ~$2.00 | 60-90 sec | cheapest on GCP |
| **GCP Hyperdisk ML @ 1,700 MiB/s** | **~$212** | under 60 sec | **billed continuously, cooldown blocks bursting** |

### What a cold start actually costs in wasted GPU time

| Cold start | Wasted per run, A100 @ $1.19/hr | Wasted per run, L4 @ $0.71/hr |
|---|---|---|
| 30 min | **$0.60** | $0.36 |
| 5 min | $0.10 | $0.06 |
| 40 sec | $0.013 | $0.008 |

**On GPU time alone, storage never pays for itself at our volume.** Thirty minutes to forty seconds saves about $0.59
per run. A $7/month RunPod volume needs roughly **12 runs a month** to break even. Hyperdisk ML at $212/month would
need over **350 runs a month**.

**So the justification for fast startup is not the GPU bill:**

1. **Client-facing latency.** A prospect watching a 30-minute boot is a lost engagement. Decisive and unpriceable.
2. **Iteration speed.** A 30-minute penalty per attempt changes what work you are willing to attempt at all.
3. **Spot preemption recovery.** On preemptible capacity, a 30-minute restore can exceed the mean time between
   preemptions, at which point the node never completes work. **This is a correctness problem, not a comfort problem,
   and it is the one that makes fast startup a requirement rather than a nicety.** Any plan using Spot must state it.

---

## 9. Recommendation

**Do these in order. The first two are free.**

1. **Fix the software path first.** Streaming loader (`runai_streamer` for object storage, `fastsafetensors` for local
   NVMe), persist CUDA graphs, and evaluate `vllm snapshot create`. This addresses the largest phases and costs
   nothing. Anything else is optimizing the smaller slice first.
2. **Stop re-pulling the image.** Cached locally and on RunPod; GKE Image Streaming or SOCI if running on an
   autoscaled hyperscaler fleet.
3. **Default persistent storage: RunPod Network Volume at $0.07/GB/month**, with Model Store caching where available.
   Cheapest path to target, and consistent with rent-first since RunPod is 2-4x cheaper than GCP for A100 class.
4. **If the client requires GCP: GCS FUSE with caching, not Hyperdisk ML.** The cooldown makes Hyperdisk ML's burst
   model unusable for short runs, and continuous provisioning is roughly $212/month for our working set.
5. **Warm pool only around a scheduled demo**, with a budget alarm.
6. **Track A locally needs none of this.**

**Do not rebuild the PD-snapshot pattern.** It is superseded on every axis, and the original implementation carried an
unverified `mkfs.ext4` target and a snapshot-then-delete-source sequence with no validation.

---

## 10. What still needs measuring

- **Our actual working set is far smaller than 100 GB.** The three local models (small chat model, BGE embeddings,
  Whisper) may already start fast enough that none of this is needed for Track A. **Measure before buying anything.**
- **The vLLM flag names above are recent.** Verify against current docs before scripting them.
- **RunPod Model Store** availability for the specific models and datacenters in play, and whether pinning is charged.
- **Spot preemption frequency** on the target GPU class, since that determines whether reason 3 above is binding.

Every one of these is answerable by measurement, and measurement is exactly what this corpus has never done.
