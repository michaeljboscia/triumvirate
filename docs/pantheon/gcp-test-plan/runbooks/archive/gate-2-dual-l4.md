# Gate 2 — Dual L4 (3090 NVLink Pair Proxy) ★ DECISIVE GATE

> **ARCHIVED 2026-08-26.** This runbook served a hardware purchase that is permanently cancelled. Standing policy is
> `../../POLICY-rent-first.md`.
>
> **Its measurement method may have survived. Its purchase verdict did not.** See `../../SIZING-SWEEP-METHOD.md`,
> which records per runbook what it was for, whether that problem persists, and what replaced it.
>
> Known defects across this set: `g4-standard-32` is not a valid machine type; harness scripts are called at
> `/opt/pantheon-harness/`, a path from a GCP image that was never built; and **only two of the six measured at 4-way
> concurrency**, which is the condition the production floor is stated in, so the floor could never have been
> evaluated from most of them as written.
>
> Consult for method and for the fault-injection scenarios. Do not execute.

---

**Purpose:** Empirically answer the **single 3090 vs 2× 3090 NVLink** purchase decision using 2× L4 GPUs on GCP as a faithful proxy.

**GCP config:** `g2-standard-24` (2× NVIDIA L4, 48GB total, PCIe interconnect)
**Cost:** ~$0.42/hr Spot × 2 hours max = **~$0.84 per session**
**Duration:** 2 hours hard cap (enforced by `--max-run-duration=120m`)
**Pre-committed decision rule:** see `30-DECISION-RULES.md` → Decision 1

---

## Why this gate is decisive

This is the gate that converts "I want 2× 3090 NVLink" from vibes into evidence. Three specific unlocks happen at 48GB that don't happen at 24GB:

1. **70B-Q4 local inference runs at all.** Qwen 72B at Q4 needs ~36GB of model weights + KV cache. On 24GB: impossible. On 48GB: runs, and the question becomes "at what tok/s and quality?"
2. **Concurrent multi-model hosting.** Zeus (Qwen 72B-Q4) + one specialist can coexist on 48GB. On 24GB: one at a time only.
3. **32B LoRA training.** Training a LoRA on Qwen 32B base via FSDP across 2 cards is meaningfully different from trying to squeeze it on one 24GB card.

If none of these unlocks matter for your actual workflow, single 3090 is correct. If one or more matters, 2× 3090 NVLink is correct.

**This gate measures all three.**

---

## What the gate does NOT test (important)

- **True NVLink bandwidth.** GCP L4 pairs use PCIe, not NVLink. Intra-GPU communication is ~10-20% slower than a real 3090 NVLink bridge provides. This mostly affects tensor-parallel inference on a single model split across cards. If tensor-parallelism on 70B is the critical workload, expect your local 3090 NVLink pair to be slightly FASTER than what this gate measures.
- **Long-session stability.** Gate 7 (soak/stress) handles 4+ hour runs. This gate is a 2-hour burst.
- **Tool-use quality delta vs Sonnet.** Gate 3 (RTX Pro 6000) and dedicated quality-comparison runs cover that. This gate focuses on capability + throughput.

---

## Pre-run checklist

Before starting Gate 2, verify:

- [ ] Preflight complete (all items in `10-PREFLIGHT.md` checked)
- [ ] Gate 1 passed (single L4 baseline established; Gate 2 needs that comparison)
- [ ] No other Pantheon VMs running (`gcloud compute instances list` is empty)
- [ ] Quota for L4 ≥ 2 in us-central1 confirmed
- [ ] PD snapshot `pantheon-models-v1` exists and is readable
- [ ] Evidence bucket writable
- [ ] Cost tracker configured with a $2 soft budget for this gate

---

## Hypotheses being tested

Each hypothesis has a pre-committed decision rule. Write the hypothesis state BEFORE running the test.

### H-2.1 — 70B-Q4 local inference is usable

**Prior belief:** 70% confident this works at usable speeds.

**Prediction:** Qwen 2.5 72B Q4 (AWQ) runs on 2× L4 via vLLM tensor-parallel=2. Single-stream tok/s ≥ 8. Multi-stream (4-way batching) sustains ≥ 5 tok/s per stream.

**Decision rule:**
- If single-stream ≥ 10 tok/s AND multi-stream ≥ 5 tok/s per stream → H-2.1 strongly confirmed (95%+). Enables sovereign demo capability.
- If single-stream 5-10 tok/s AND multi-stream ≥ 3 tok/s → usable but slow. Weak confirmation (70%).
- If single-stream < 5 tok/s → fails. 70B-local not worth the hardware premium.

### H-2.2 — Concurrent multi-model hosting works

**Prior belief:** 80% confident.

**Prediction:** Zeus (Qwen 72B-Q4, ~36GB on 1 card) + specialist (Qwen Coder 32B-Q8, ~18GB on 2nd card via PCIe) both serve requests simultaneously without contention. Latency impact when both busy < 2× isolated latency.

**Decision rule:**
- Both serve concurrently, latency impact < 2× → H-2.2 confirmed. Enables Triumvirate's multi-model dispatch pattern on 48GB locally.
- Latency impact 2-3× → usable but constrained. Prefer sequential dispatch.
- Models interfere badly or OOM → fails. 48GB is too cramped for dual-model live serving.

### H-2.3 — 32B LoRA training is feasible at this tier

**Prior belief:** 60% confident.

**Prediction:** LoRA fine-tune of Qwen 2.5 Coder 32B Q8 on a 10K-sample corpus completes in ≤ 3 hours wall-clock using FSDP across 2× L4. No OOM, no gradient accumulation pathologies.

**Decision rule:**
- Training completes in ≤ 3 hrs, eval score positive → H-2.3 confirmed. 32B LoRA accessible at this tier.
- Training requires 3-6 hrs OR needs aggressive gradient accumulation → partial. Prefer 7-14B LoRAs for iteration speed.
- Training fails (OOM, divergence) → falsified. Stay at 7-14B LoRAs, escalate 32B training to rented A100.

### H-2.4 — The 48GB tier unlocks ≥ 5 friction-reducing workflows/week

**Prior belief:** 70% confident, but calibration depends on honest friction logging.

**Prediction:** After Gate 1 + Gate 2 runs, the friction log (kept during normal Pantheon dev) identifies ≥ 5 events per week where specifically the 48GB tier (not 24GB) was the enabling factor.

**Decision rule:** Evaluated after 4 weeks of post-gate operation, NOT during this gate's 2-hour session. Pre-commit the rule now; measure later.

---

## Runbook

### Step 1 — Provision VM (2-3 min, ~$0.02)

```bash
# Set run metadata
export PROJECT_ID="pantheon-validation-v1"
export ZONE="us-central1-a"
export RUN_ID="gate2-dual-l4-$(date +%Y%m%d-%H%M%S)"
export REGISTRY="us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

# Pre-flight inventory check — abort if VMs exist
RUNNING=$(gcloud compute instances list --filter="status=RUNNING" --format="value(name)")
if [ -n "$RUNNING" ]; then
  echo "ABORT: VMs already running: $RUNNING"
  echo "Delete them first with: gcloud compute instances delete $RUNNING --zone=$ZONE"
  exit 1
fi

# Create the VM
gcloud compute instances create pantheon-$RUN_ID \
  --zone=$ZONE \
  --project=$PROJECT_ID \
  --machine-type=g2-standard-24 \
  --accelerator=type=nvidia-l4,count=2 \
  --provisioning-model=SPOT \
  --instance-termination-action=DELETE \
  --max-run-duration=120m \
  --network=pantheon-net \
  --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-gpu \
  --image-project=$PROJECT_ID \
  --boot-disk-size=100GB \
  --boot-disk-type=pd-ssd \
  --create-disk=name=models-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  --metadata=RUN_ID=$RUN_ID,GATE=2,install-nvidia-driver=True \
  --metadata-from-file=startup-script=./harness/gate-2-startup.sh \
  --no-address
```

### Step 2 — Wait for VM ready (3-5 min)

```bash
# Poll until SSH is available
until gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="nvidia-smi" 2>/dev/null; do
  echo "Waiting for VM + GPU driver..."
  sleep 15
done

echo "VM ready at $(date)"
```

### Step 3 — SSH in and verify environment

```bash
gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE

# On VM:
nvidia-smi   # Expected: 2× L4, each ~23GB free
sudo mount /dev/disk/by-id/google-*-models /mnt/models
ls /mnt/models/   # Verify model weights present
docker ps         # Verify docker daemon up
gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
```

### Step 4 — Start vLLM serving Qwen 72B-Q4 with TP=2 (5-8 min)

```bash
# Launch vLLM with tensor-parallel=2 across both L4 cards
export REGISTRY="us-central1-docker.pkg.dev/pantheon-validation-v1/pantheon-images"

docker run -d --name vllm-zeus \
  --gpus all \
  -v /mnt/models/qwen2.5-72b-awq:/models/qwen72b:ro \
  -p 8000:8000 \
  --shm-size=16g \
  ${REGISTRY}/pantheon-vllm-gpu:v0.6.5 \
  --model /models/qwen72b \
  --served-model-name qwen2.5-72b-instruct-awq \
  --tensor-parallel-size 2 \
  --gpu-memory-utilization 0.90 \
  --max-model-len 8192 \
  --max-num-seqs 8 \
  --quantization awq_marlin \
  --dtype float16

# Wait for model load (~5-8 min for 72B)
until curl -sf http://localhost:8000/v1/models 2>/dev/null | grep -q qwen2.5-72b; do
  echo "Waiting for vLLM model load..."
  sleep 20
done

echo "vLLM ready at $(date)"
nvidia-smi   # Both cards should show ~20GB used
```

### Step 5 — H-2.1 test: 70B single-stream + multi-stream throughput (20 min)

```bash
# Single-stream test (1 concurrent request, sustained)
docker run --rm \
  --network host \
  -e RUN_ID=$RUN_ID \
  -e GATE=2 \
  -e TEST=h-2.1-single-stream \
  ${REGISTRY}/pantheon-test-harness:main \
  --endpoint=http://localhost:8000/v1 \
  --model=qwen2.5-72b-instruct-awq \
  --mode=throughput-sustained \
  --concurrency=1 \
  --duration=300 \
  --output-dir=/tmp/evidence/$RUN_ID/h-2.1-single

# Multi-stream test (4 concurrent requests, continuous batching)
docker run --rm \
  --network host \
  -e RUN_ID=$RUN_ID \
  -e GATE=2 \
  -e TEST=h-2.1-multi-stream \
  ${REGISTRY}/pantheon-test-harness:main \
  --endpoint=http://localhost:8000/v1 \
  --model=qwen2.5-72b-instruct-awq \
  --mode=throughput-sustained \
  --concurrency=4 \
  --duration=300 \
  --output-dir=/tmp/evidence/$RUN_ID/h-2.1-multi

# Collect metrics
cat /tmp/evidence/$RUN_ID/h-2.1-single/metrics.json
cat /tmp/evidence/$RUN_ID/h-2.1-multi/metrics.json
```

**Expected harness output schema:**

```json
{
  "test_id": "h-2.1-single-stream",
  "run_id": "gate2-dual-l4-...",
  "model": "qwen2.5-72b-instruct-awq",
  "concurrency": 1,
  "duration_sec": 300,
  "requests_completed": 42,
  "total_input_tokens": 21000,
  "total_output_tokens": 12600,
  "tokens_per_second_per_stream_median": 12.4,
  "tokens_per_second_per_stream_p95": 10.1,
  "time_to_first_token_ms_median": 320,
  "first_token_errors": 0,
  "completion_errors": 0,
  "json_schema_validity_rate": 0.95
}
```

### Step 6 — H-2.2 test: concurrent multi-model hosting (20 min)

```bash
# Start second vLLM for Qwen Coder 32B — pin to GPU 1, leave GPU 0 with Zeus
docker stop vllm-zeus
# Reconfigure vLLM to use only GPU 0, with less memory budget
docker run -d --name vllm-zeus \
  --gpus '"device=0"' \
  -v /mnt/models/qwen2.5-72b-awq:/models/qwen72b:ro \
  -p 8000:8000 \
  --shm-size=8g \
  ${REGISTRY}/pantheon-vllm-gpu:v0.6.5 \
  --model /models/qwen72b \
  --served-model-name qwen2.5-72b-instruct-awq \
  --tensor-parallel-size 1 \
  --gpu-memory-utilization 0.88 \
  --max-model-len 4096 \
  --max-num-seqs 2 \
  --quantization awq_marlin \
  --dtype float16

# Wait for load
until curl -sf http://localhost:8000/v1/models; do sleep 10; done

# Start Qwen Coder 32B on GPU 1
docker run -d --name vllm-coder \
  --gpus '"device=1"' \
  -v /mnt/models/qwen2.5-coder-32b-awq:/models/coder32b:ro \
  -p 8001:8000 \
  --shm-size=8g \
  ${REGISTRY}/pantheon-vllm-gpu:v0.6.5 \
  --model /models/coder32b \
  --served-model-name qwen2.5-coder-32b-instruct-awq \
  --tensor-parallel-size 1 \
  --gpu-memory-utilization 0.80 \
  --max-model-len 4096 \
  --max-num-seqs 4 \
  --quantization awq_marlin \
  --dtype float16

# Wait for load
until curl -sf http://localhost:8001/v1/models; do sleep 10; done

# Verify both serving
nvidia-smi   # Expected: GPU 0 ~20GB used (Zeus), GPU 1 ~18GB used (Coder)
curl -s http://localhost:8000/v1/models
curl -s http://localhost:8001/v1/models

# Concurrent load test — hit BOTH endpoints simultaneously
docker run --rm \
  --network host \
  -e RUN_ID=$RUN_ID \
  -e GATE=2 \
  -e TEST=h-2.2-concurrent-models \
  ${REGISTRY}/pantheon-test-harness:main \
  --mode=multi-endpoint-concurrent \
  --endpoint-a=http://localhost:8000/v1 --model-a=qwen2.5-72b-instruct-awq \
  --endpoint-b=http://localhost:8001/v1 --model-b=qwen2.5-coder-32b-instruct-awq \
  --concurrency-a=2 --concurrency-b=4 \
  --duration=300 \
  --output-dir=/tmp/evidence/$RUN_ID/h-2.2

cat /tmp/evidence/$RUN_ID/h-2.2/metrics.json
```

**Expected output additions:**

```json
{
  "test_id": "h-2.2-concurrent-models",
  "endpoint_a_tok_s_isolated": 11.2,
  "endpoint_a_tok_s_under_concurrent_load": 7.8,
  "endpoint_b_tok_s_isolated": 45.2,
  "endpoint_b_tok_s_under_concurrent_load": 38.1,
  "contention_factor_a": 1.44,
  "contention_factor_b": 1.19,
  "errors": 0
}
```

**Decision check:** contention_factor_a < 2.0 AND contention_factor_b < 2.0 → H-2.2 confirmed.

### Step 7 — H-2.3 test: 32B LoRA training feasibility (45-60 min)

```bash
# Stop inference containers to free GPU memory for training
docker stop vllm-zeus vllm-coder
docker rm vllm-zeus vllm-coder

# Pull training harness image (Axolotl pre-configured)
docker pull ${REGISTRY}/pantheon-axolotl:main

# Download training corpus
gsutil -m cp -r gs://pantheon-fixtures/lora-training-corpus-v1/ /tmp/training-corpus/

# Launch LoRA training with FSDP across both cards
docker run --rm \
  --gpus all \
  --shm-size=16g \
  -v /mnt/models/qwen2.5-coder-32b-awq:/models/base:ro \
  -v /tmp/training-corpus:/data:ro \
  -v /tmp/lora-output:/output \
  -v /tmp/evidence/$RUN_ID/h-2.3:/evidence \
  -e RUN_ID=$RUN_ID \
  -e TEST=h-2.3-lora-32b \
  -e WANDB_DISABLED=true \
  ${REGISTRY}/pantheon-axolotl:main \
  --config=/data/lora-32b-coder-fsdp.yml \
  --base-model=/models/base \
  --output=/output \
  --log-dir=/evidence

cat /tmp/evidence/$RUN_ID/h-2.3/training-summary.json
```

**Axolotl config** (`lora-32b-coder-fsdp.yml`) should specify:

```yaml
base_model: /models/base
adapter: lora
lora_r: 16
lora_alpha: 32
lora_dropout: 0.05
sequence_len: 2048
micro_batch_size: 1
gradient_accumulation_steps: 8
num_epochs: 1
learning_rate: 2e-4
optimizer: paged_adamw_8bit
fsdp:
  - full_shard
  - auto_wrap
fsdp_config:
  fsdp_transformer_layer_cls_to_wrap: Qwen2DecoderLayer
bf16: true
```

**Expected training output:**

```json
{
  "test_id": "h-2.3-lora-32b",
  "base_model": "qwen2.5-coder-32b-awq",
  "training_samples": 10000,
  "training_duration_sec": 6840,
  "peak_vram_gb_gpu0": 22.1,
  "peak_vram_gb_gpu1": 22.8,
  "loss_initial": 1.84,
  "loss_final": 0.42,
  "eval_score_improvement": 0.18,
  "oom_events": 0
}
```

**Decision check:** training_duration_sec < 10800 (3 hrs) AND oom_events == 0 → H-2.3 confirmed.

### Step 8 — Evidence bundle + self-destruct

```bash
# Collect logs from all containers
docker logs vllm-zeus > /tmp/evidence/$RUN_ID/vllm-zeus.log 2>&1 || true
docker logs vllm-coder > /tmp/evidence/$RUN_ID/vllm-coder.log 2>&1 || true

# Collect GPU metrics throughout
nvidia-smi --query-gpu=timestamp,index,utilization.gpu,memory.used,temperature.gpu,power.draw \
  --format=csv > /tmp/evidence/$RUN_ID/nvidia-smi-final.csv

# Build manifest
cat > /tmp/evidence/$RUN_ID/manifest.json <<EOF
{
  "run_id": "$RUN_ID",
  "gate": 2,
  "started_at": "$(date -u -d @$(cat /tmp/run-start-time.txt) -Iseconds)",
  "ended_at": "$(date -u -Iseconds)",
  "gcp_machine_type": "g2-standard-24",
  "gcp_accelerators": ["2x nvidia-l4"],
  "hypotheses_tested": ["H-2.1", "H-2.2", "H-2.3"],
  "verdicts": {
    "H-2.1": "$(jq -r .verdict /tmp/evidence/$RUN_ID/h-2.1-single/metrics.json)",
    "H-2.2": "$(jq -r .verdict /tmp/evidence/$RUN_ID/h-2.2/metrics.json)",
    "H-2.3": "$(jq -r .verdict /tmp/evidence/$RUN_ID/h-2.3/training-summary.json)"
  }
}
EOF

# Upload entire evidence bundle to GCS
gsutil -m cp -r /tmp/evidence/$RUN_ID gs://pantheon-evidence/gate-2/

# Generate cost report from GCP billing API
python3 /opt/pantheon-harness/cost-tracker.py \
  --run-id=$RUN_ID \
  --output=/tmp/evidence/$RUN_ID/cost-report.json
gsutil cp /tmp/evidence/$RUN_ID/cost-report.json gs://pantheon-evidence/gate-2/$RUN_ID/

# Generate human-readable summary
python3 /opt/pantheon-harness/generate-summary.py \
  --run-id=$RUN_ID --gate=2 \
  --evidence-dir=/tmp/evidence/$RUN_ID \
  --output=/tmp/evidence/$RUN_ID/summary.md
gsutil cp /tmp/evidence/$RUN_ID/summary.md gs://pantheon-evidence/gate-2/$RUN_ID/

# Auto-generate Obsidian note from summary
python3 /opt/pantheon-harness/generate-obsidian-note.py \
  --run-id=$RUN_ID --gate=2 \
  --summary=/tmp/evidence/$RUN_ID/summary.md \
  --template=/opt/pantheon-harness/templates/gate-run-note.md \
  --output=/tmp/evidence/$RUN_ID/obsidian-note.md
gsutil cp /tmp/evidence/$RUN_ID/obsidian-note.md gs://pantheon-evidence/gate-2/$RUN_ID/

# Self-destruct
exit   # log out of SSH

# Back on laptop — wait for auto-delete OR force it
gcloud compute instances delete pantheon-$RUN_ID --zone=$ZONE --quiet
```

### Step 9 — Verify evidence landed

```bash
# On laptop:
gsutil ls -r gs://pantheon-evidence/gate-2/$RUN_ID/

# Expected output:
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../manifest.json
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../summary.md
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../cost-report.json
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../obsidian-note.md
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../h-2.1-single/metrics.json
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../h-2.1-multi/metrics.json
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../h-2.2/metrics.json
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../h-2.3/training-summary.json
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../vllm-zeus.log
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../vllm-coder.log
# gs://pantheon-evidence/gate-2/gate2-dual-l4-.../nvidia-smi-final.csv

# Download summary to review
gsutil cp gs://pantheon-evidence/gate-2/$RUN_ID/summary.md /tmp/
cat /tmp/summary.md
```

### Step 10 — Drop Obsidian note into vault

```bash
# Copy auto-generated note into your Obsidian vault
mkdir -p ~/Documents/pantheon-vault/runs/
gsutil cp gs://pantheon-evidence/gate-2/$RUN_ID/obsidian-note.md \
  ~/Documents/pantheon-vault/runs/$RUN_ID.md

cd ~/Documents/pantheon-vault
git add runs/$RUN_ID.md
git commit -m "gate-2 run $RUN_ID evidence"
```

Open Obsidian, navigate to the new note, add any qualitative observations from the run that the harness can't capture (e.g., "felt noticeably slower than Sonnet on long-context tasks" or "json fencing issue returned in 2 edge cases").

---

## Apply pre-committed decision rule

Once evidence is in GCS and Obsidian note is written, apply the Decision 1 rule from `30-DECISION-RULES.md`:

### Case A: Buy 2× 3090 NVLink

If ALL:
- H-2.1 single-stream ≥ 10 tok/s
- H-2.1 multi-stream ≥ 5 tok/s per stream
- H-2.2 contention factors both < 2.0
- H-2.3 training completes in ≤ 3 hrs
- (Deferred: friction log ≥ 5 events/week over the next 4 weeks)

Then the hardware purchase is evidence-supported. When funds arrive, buy used 2× 3090 + NVLink bridge + workstation, total budget $3500-4500.

### Case B: Buy single 3090 only

If:
- H-2.1 fails (single-stream < 5 tok/s) — 70B-local isn't useful
- H-2.2 succeeds OR fails (either way, single card avoids the concurrency complexity)
- H-2.3 requires 3-6 hrs or gradient accumulation hacks — stay at 7-14B LoRAs

Then buy single 3090, budget $2500 total.

### Case C: Skip local GPU entirely

If:
- H-2.1, H-2.2, H-2.3 all underwhelm
- Friction log (measured over subsequent weeks) shows < 3 events/week where local GPU would help
- Pre-bake tooling makes GCP feel near-frictionless

Then don't buy. Stay OPEX-first. Budget unchanged.

---

## Cost accounting for this gate

| Line item | Cost |
|---|---|
| VM provision (g2-standard-24 Spot, 2 hrs max) | ~$0.84 |
| PD from snapshot (transient) | ~$0.05 |
| Egress (minimal, GCS same-region) | ~$0.01 |
| **Total per session** | **~$0.90** |

Max two sessions budgeted (first run + one replication): **~$1.80 total for Gate 2.**

---

## What comes after Gate 2

Gate 2 produces THE purchase decision for the 3090 question. After Gate 2 evidence is reviewed:

- If decision is "buy" → wait for funds, buy the measured config, don't second-guess
- If decision is "skip" → stay OPEX-first, proceed to Gate 3 (RTX Pro 6000 twin) when ready to test the next hardware tier

Next gate runbook: `runbooks/gate-3-rtx-pro-6000.md` — tests the Phase 3 purchase candidate on GCP's G4 instances (literal 1:1 hardware parity with what you'd buy).
