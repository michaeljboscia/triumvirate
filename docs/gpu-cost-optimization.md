# GPU Cost Optimization — Pythia & General Workloads

**Date:** 2026-04-11
**Source:** Billing data analysis session. All prices from BigQuery billing export, not training data.

## Decision: Move Pythia Encoding from A100 to L4 Spot

**Status:** DECIDED — not yet implemented

Pythia's embedding/encoding workload uses ~2-4GB VRAM. Running it on an A100 40GB ($2.93/hr on-demand, $1.43/hr Spot) wastes 36GB of VRAM. An L4 Spot ($0.22/hr) does the same job at 85-92% less cost.

### Cost Comparison (from billing data)

| GPU | On-Demand | Spot | VRAM | Notes |
|-----|-----------|------|------|-------|
| A100 40GB | $2.93/hr | $1.43/hr | 40GB | Current. Overkill for encoding. |
| L4 | $0.56/hr | $0.22/hr | 24GB | Recommended for encoding. |
| T4 | $0.35/hr | $0.17/hr | 16GB | Cheapest. Slower compute. |

**For a 2-hour Pythia encode job:**
- A100 Spot: $2.86
- L4 Spot: $0.44 (6.5x cheaper)
- T4 Spot: $0.34 (8.4x cheaper)

### GPU Quotas (us-east4, verified 2026-04-11)

| GPU | On-Demand | Spot |
|-----|-----------|------|
| A100 40GB | 16 | 64 |
| A100 80GB | 0 | 0 |
| L4 | 16 | 3 |
| T4 | 8 | 3 |
| V100 | 8 | 16 |

**Action needed:** L4 Spot quota is only 3. Request increase to 8-16 if L4 Spot becomes primary GPU. Self-service in GCP Console > IAM & Admin > Quotas.

## Multi-GPU VRAM Stacking

VRAM does NOT pool across GPUs like RAM. Each GPU has isolated memory. To use multiple GPUs as one larger memory space, the inference framework (vLLM, TGI) must shard the model via tensor parallelism.

### Available Multi-L4 Configs

| Machine Type | GPUs | Total VRAM | On-Demand | Spot |
|-------------|------|------------|-----------|------|
| g2-standard-4 | 1x L4 | 24GB | $0.56/hr | $0.22/hr |
| g2-standard-24 | 2x L4 | 48GB | $1.12/hr | $0.44/hr |
| g2-standard-48 | 4x L4 | 96GB | $2.24/hr | $0.88/hr |
| g2-standard-96 | 8x L4 | 192GB | $4.48/hr | $1.76/hr |

**Caveats:**
- L4s connect over PCIe (64 GB/s). A100s use NVLink (600 GB/s). PCIe is fine for batch encoding, bad for distributed training with frequent gradient sync.
- ~10-15% VRAM lost to communication buffers. 2x L4 = ~41-43GB usable.
- Model attention heads must divide evenly across GPU count.

## Training / Fine-Tuning GPU Selection

Training is compute + memory bandwidth intensive. Not just VRAM.

### VRAM Requirements by Fine-Tuning Method

| Method | 7B model | 13B model | 70B model |
|--------|----------|-----------|-----------|
| Full fine-tune | ~56GB | ~104GB | ~560GB |
| LoRA | ~18-20GB | ~30-35GB | ~160GB |
| QLoRA (4-bit) | ~8-10GB | ~14-18GB | ~40-48GB |

### GPU Compute Throughput

| GPU | FP16 TFLOPS | Mem BW | BF16 | NVLink |
|-----|-------------|--------|------|--------|
| T4 | 65 | 300 GB/s | NO | NO |
| L4 | 121 | 300 GB/s | YES | NO |
| A100 | 312 | 2 TB/s | YES | YES |

### Decision Tree

- **QLoRA on model that fits one GPU?** → L4 Spot. Cheaper even though slower.
- **Model needs >24GB and must shard across GPUs?** → A100 Spot. NVLink makes multi-GPU training viable. Multi-L4 over PCIe bottlenecks on gradient sync.
- **Training from scratch?** → Different budget conversation.

## Billing Dashboard Fixes (2026-04-11)

The `~/gcp-billing-metrics.sh` script was reporting **gross** costs, not net. Fixed to show:
- Gross, credits, and net in the header
- Daily spend shows both gross and net columns
- New sections: credits breakdown, actual GPU hourly rates from billing data
- Footer clarifies all numbers are NET

### Active Credits on This Account

| Credit | Monthly Value |
|--------|--------------|
| GKE free tier | ~$24/mo (cluster mgmt fee fully refunded) |
| Promotional credit | ~$44/mo |
| Network Intelligence Center | ~$0.85/mo |

**Example impact:** April gross was $102.40, net was $30.41.

## Next Actions

1. [ ] Update `/deploy-encoder` skill to default to L4 Spot instead of A100
2. [ ] Test Pythia encoder model on L4 (verify it fits in 24GB VRAM)
3. [ ] Request L4 Spot quota increase (3 → 16) in us-east4
4. [ ] Update `~/.claude/gcp-actual-pricing.md` quarterly from billing data

## Reference

- Pricing source: `aerial-jigsaw-467620-m8.GCP_BILLING.gcp_billing_export_v1_01F713_7EFFD2_83E164`
- Pricing reference file: `~/.claude/gcp-actual-pricing.md`
- Hookify rules: `~/.claude/hookify.require-gcp-pricing-evidence.local.md`, `~/.claude/hookify.gcp-costs-net-only.local.md`
