# Pantheon Hardware Decision Record

> **ARCHIVED 2026-08-25. Do not act on this document.** It was written to plan a self-funded hardware
> purchase. That purchase is permanently cancelled. Standing policy is rent first, always, with owned
> metal only ever as a customer-funded terminal step, and renting is the destination rather than a
> waypoint. See `docs/pantheon/local-inference-buy-vs-rent.md` section 6.
>
> This file previously carried `Status: ACCEPTED` while charting a path to $160K of hardware, which made
> it the most dangerous document in the corpus: a canonical, accepted instruction to do exactly what the
> policy forbids. It is kept for provenance only.
>
> **One thing was extracted before archiving:** the production floor of 15 tokens per second per stream
> under 4-way batched load. That criterion is substrate-agnostic, applies to rented configurations
> exactly as it did to owned ones, and now lives in `local-inference-buy-vs-rent.md` section 6.

**Status**: ARCHIVED (was ACCEPTED)
**Date**: 2026-04-27
**Supersedes**: `/Users/mikeboscia/PANTHEON_ARCHITECTURE.md` and `/Users/mikeboscia/projects/triumvirate/PANTHEON_ARCHITECTURE.md` (both dated 2026-04-19, both describe 3090/Spark/M5 Trinity)
**Conversation provenance**: ~2026-04-12 through 2026-04-19 sessions; primary thread in `~/.claude/projects/-Users-mikeboscia/2741b98f-28e8-4dbc-a3e8-1e3ee4f9ad1c.jsonl` (2026-04-13, 599 TPS-related hits)
**Empirical anchors**: GCP test-plan runbooks at `docs/pantheon/gcp-test-plan/runbooks/`

---

## The decision

The original "high-speed Pantheon Closet" architecture — Zeus (Mac Studio M5 Ultra 512GB) + Athena (2× NVIDIA DGX Spark) + Vulcan-1 (2× RTX 3090 NVLink) at ~$20-30K target — is **rejected as production hardware** for the multi-agent fleet workload. It would still produce working coding agents, but only at roughly 15-20 tokens-per-second-per-stream under the fleet load Pantheon's discipline matrix demands. That sits AT or BELOW the production floor required for the verification gates to keep pace with agent throughput. Under-floor hardware means gates lag, audit chain backlogs, and the substrate's value proposition collapses.

The new path:

1. **Daily-driver tier** — Mac Studio M5 Ultra (256-512GB) + 1× RTX Pro 6000 Blackwell. Roughly $15-30K. Sufficient for solo development; gates pass on isolated workloads.
2. **Working tier** — 2× RTX Pro 6000 (chassis + power + storage built up around the daily driver). Roughly $75K aggregate. First configuration that clears the production floor under multi-agent fleet load.
3. **Higher tier** — A100-80G class hardware OR equivalent enterprise pod. Roughly $160K. Only purchased after Working tier has been operational long enough to identify the specific bottleneck A100s solve (per Decision Rule 6 in `30-DECISION-RULES.md`).

Until capital is allocated for any of those tiers: **all testing and development happens via OPEX in GCP or on federal Hyperscalers** (CoreWeave Federal, Lambda Labs, Together AI). Cost-optimized vendor partners aggregate VRAM pools large enough to keep developing the toolset without committing to inadequate hardware.

## Why a smaller OPEX spend beats a $30K CapEx on inadequate hardware

This is the load-bearing thesis of this decision. The $20-30K original Trinity target *would* produce code-writing agents — but at TPS-per-stream throughput that fails the production floor under fleet load. Spending $30K on hardware that can't clear the floor is worse than spending a fraction of that monthly on cloud GPU that can clear the floor:

- Inadequate hardware is sunk cost. You can't return it. You can't upgrade it incrementally without throwing away the original investment.
- OPEX scales with usage. If a workload doesn't need the GPU, the bill stops.
- OPEX moves with the model frontier. When Llama 5 / Qwen 4 / Mistral 3 ship, OPEX gets the new card class without re-procurement.
- Neoclouds offer cost-per-VRAM at roughly 30-40% of GovCloud at-list, with comparable hardware tiers (subject to data classification — federal CUI requires an authorized hosting boundary that most Neoclouds don't yet hold).

The principle: **first on-prem hardware purchase must clear the production floor under fleet load.** Under-floor hardware is not a stepping stone; it is a liability that delays meeting the floor while consuming capital that could have rented above-floor capacity in the meantime.

## The TPS floor as decision criterion

"Production floor" is empirical, not aspirational. The gate runbooks define it explicitly:

| Source | Hardware | TPS prediction (single-stream) | TPS prediction (4-way batched) |
|---|---|---|---|
| `runbooks/gate-1-single-l4.md` | 1× L4 24GB (3090 single proxy) | n/a | varies |
| `runbooks/gate-2-dual-l4.md` | 2× L4 48GB (3090 NVLink pair proxy) | varies | **median 12.4, p95 10.1, p99 8.6** |
| `runbooks/gate-3-rtx-pro-6000.md` | 1× RTX Pro 6000 Blackwell | **≥ 50** | **≥ 20** |
| `runbooks/gate-5-full-trinity.md` | 8× A100 80GB (Llama 405B) | **≥ 30** | **≥ 15** |

**The 12.4-median / 10.1-p95 from Gate 2 (dual-L4 = 3090-NVLink-pair proxy) is the empirical evidence that the original Vulcan-1 spec falls below the production floor.** Combined with the GB10 Spark architecture's known limitations on memory bandwidth (273 GB/s — far below H100/B200 class) and the M5 Ultra's role as architect-not-worker, the Trinity at $20-30K target cannot deliver fleet-load throughput.

**Production floor requirement**: ≥ 15 tokens-per-second-per-stream under 4-way batched concurrent agent load. Anything below that, the verification gates cannot consume agent output as fast as agents produce it; backpressure inverts; the substrate stops being real-time.

## What this changes from the prior plan

| Before | After |
|---|---|
| Vulcan-1 = 2× RTX 3090 NVLink ($3,500-4,500) | Vulcan = 1× RTX Pro 6000 Blackwell ($13-15K), then 2× ($26-28K total) |
| Athena = 2× DGX Spark linked ($8-12K) | Athena role absorbed into A100-80G class hardware OR cloud-burst (deferred until Working tier operational) |
| Zeus = Mac Studio M5 Ultra 512GB ($12K) | Zeus = Mac Studio M5 Ultra 256-512GB ($8-12K) — role unchanged, daily-driver framing emphasized |
| Total Closet target ≈ $20-30K | Tier 1 ≈ $15-30K · Tier 2 ≈ $75K · Tier 3 ≈ $160K |
| "Buy first, validate later" | OPEX-first; CapEx triggered by Decision Rules 2/3/6 in `30-DECISION-RULES.md` against measured GCP usage |

## What stays the same

- The architectural pattern (Zeus = architect, Athena = swarm, Vulcan = forger). The role decomposition is unchanged; only the silicon under each role changes.
- The networking + RAM-disk + storage infrastructure described in the superseded `PANTHEON_ARCHITECTURE.md` (100GbE intra-node, 10GbE asset, gigabit control, dual 20A circuits, U.2/U.3 NVMe persistent + tmpfs ephemeral). Those decisions are silicon-independent.
- The business model (~$30/day OpEx target, $3-5K/month per-client maintenance retainers).
- The /bugsquasher protocol for sub-1.5M-token repos.

## Acceleration paths (DoD-specific)

Two channels could compress the on-prem timeline materially without violating the OPEX-first principle:

1. **Direct DoD GPU procurement** — RTX Pro 6000 fits the FAR $15K micro-purchase ceiling cleanly. 2× RTX Pro 6000 + Mac Studio 512GB sits inside the $350K simplified-acquisition threshold. Even Tier 2 (~$75K) is well inside JIOP's $50M acquisition authority.
2. **Government Hyperscaler tenancy** — Azure Government's GPU SKUs (H100 class), AWS GovCloud, or Google Distributed Cloud authorized boundaries. Substrate is deployment-portable; running in JIOP's authorized tenant on day one means on-prem becomes nice-to-have rather than blocking.

Either acceleration is preferable to spending capital on under-floor hardware while the procurement channel works.

## Decision rules (from `30-DECISION-RULES.md`, summarized)

- **OPEX → CAPEX threshold**: monthly GCP spend on a workload class > $1000 for 2 consecutive months AND usage hours exceed break-even (RTX Pro 6000 class: > 150 hrs/mo; A100 80GB class: > 230 hrs/mo; H100 class: > 360 hrs/mo).
- **2× RTX Pro 6000**: triggered by Decision Rule 3 — RTX Pro 6000 in operation ≥ 60 days AND specific bottleneck identified.
- **Pantheon Rack tier (A100-80G class or DGX H100)**: triggered by Decision Rule 6 — enterprise customer engagement signed OR self-directed $150-500K investment AND Gate 5 PASS.

## Open questions (do not relitigate without one of these triggers)

- **Will Apple ship a 1TB unified-memory Mac Studio at WWDC 2026?** Trigger: Apple announcement. Could shift Zeus tier downward if 1TB unified replaces some of the GPU role for embeddings + architect.
- **Will any Neocloud achieve FedRAMP High before late 2026?** Trigger: CoreWeave Federal / Lambda Labs / Together AI authorization. Would unlock CUI-class workloads at OPEX-tier costs.
- **Does direct DoD GPU procurement materialize as a real channel?** Trigger: JIOP feedback or AAL/DIU pathway opens. Would compress Tier 1 to immediate.

---

*This decision supersedes prior architecture documents. Future updates append to this file with dated amendments. The OPEX-first principle is non-negotiable: first on-prem CapEx must clear the production floor under measured fleet load. Under-floor hardware is a liability, not a stepping stone.*
