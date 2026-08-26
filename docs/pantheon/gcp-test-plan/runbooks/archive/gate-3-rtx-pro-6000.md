# Gate 3 — RTX Pro 6000 Blackwell Hardware Twin

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

**Purpose:** Validate Pantheon on the EXACT hardware you'd buy for Phase 3 local deployment. GCP's G4 instance family uses NVIDIA RTX Pro 6000 Blackwell Server Edition — literally the same silicon as the workstation card you'd own.

**GCP config:** `g4-standard-32` (1× RTX Pro 6000 Blackwell, 96GB)
**Cost:** ~$3-4/hr Spot × 1-2 hrs = **~$3-8 per session**
**Duration:** 2 hours hard cap
**Pre-committed decision rule:** see `30-DECISION-RULES.md` → Decision 2

---

## Why this gate matters

This is the closest you'll get to "running Pantheon on your eventual local box" without buying the box. If Gate 3 passes cleanly:

- The $13-15K RTX Pro 6000 purchase is empirically de-risked
- You know exactly what tok/s to expect from your local Zeus
- You've validated the concurrent-worker pattern at production capacity
- You have benchmark numbers for customer pitches

If Gate 3 reveals unexpected behavior (thermal throttling, driver quirks, Blackwell FP4 surprises), you discover it on a $6 burn, not a $15K commitment.

---

## What this gate validates specifically

- **Zeus at 72B-Q4 on Blackwell single card** — the "local Zeus" pattern
- **4-way concurrent worker batching** via vLLM continuous batching
- **70B model at single-card tensor-parallel=1** (no TP needed on 96GB)
- **FP4 inference** if supported in your vLLM build (Blackwell native)
- **Multi-model concurrent hosting** — Zeus + 2 specialists on one 96GB card
- **32B LoRA training single-card** (no FSDP complexity)

---

## Hypotheses being tested

### H-3.1 — 72B-Q4 single-card inference hits production speed

**Prediction:** Qwen 2.5 72B Q4 on 1× RTX Pro 6000 achieves ≥ 50 tok/s single-stream, ≥ 20 tok/s per stream under 4-way concurrent load.

**Decision rule:**
- Single-stream ≥ 50 tok/s, 4-way ≥ 20 tok/s → PASS. RTX Pro 6000 is production-viable for Zeus role.
- Single-stream 30-50, 4-way 15-20 → acceptable; purchase still justified.
- Single-stream < 30 tok/s → investigate; purchase decision blocked pending diagnosis.

### H-3.2 — Multi-model concurrent hosting at scale

**Prediction:** Zeus (72B-Q4, ~40GB) + Qwen Coder 32B-Q8 (~35GB) + DeepSeek Coder 16B-Q8 (~18GB) all serve simultaneously on 96GB with < 5GB contention on KV cache.

**Decision rule:**
- All three serve with contention factors < 1.5× → PASS. Full specialist fleet viable single-card.
- One or two fit but third crowds out → partial; pick best specialist combo.
- VRAM pressure causes OOM → must pick between full Zeus OR specialist fleet, not both.

### H-3.3 — 32B LoRA training single-card

**Prediction:** Qwen Coder 32B LoRA with 10K-sample corpus completes in ≤ 2 hrs on single card (no FSDP needed with 96GB).

**Decision rule:**
- ≤ 2 hrs training, no OOM → PASS. Training moat accessible at this tier.
- 2-4 hrs → acceptable.
- OOM or > 4 hrs → adjust batch size / gradient accumulation.

### H-3.4 — Pantheon end-to-end agent task performance

**Prediction:** Canonical 8-task agent swarm (4 Tellus + 4 YellingToad tasks) completes end-to-end via Triumvirate dispatch + multi-model routing in ≤ 30 min, ≥ 6/8 pass eval.

**Decision rule:**
- ≥ 6/8 pass, ≤ 30 min → PASS. Production-class agent throughput on single local card.
- 4-5/8 pass → acceptable; identify failure modes.
- < 4/8 pass → architectural issue; investigate before hardware purchase.

---

## Pre-run checklist

- [ ] Preflight complete
- [ ] Gate 0, 1, 2 passed
- [ ] G4 quota confirmed (16× RTX Pro 6000 in us-central1, already verified via user screenshot)
- [ ] `pantheon-gpu-v1` image tested on G4 (may need driver variant)
- [ ] Canonical 8-task fixture set in `gs://pantheon-fixtures/agent-tasks-canonical/`
- [ ] Eval harness scoring rubrics ready

---

## Runbook

### Step 1 — Provision G4 with RTX Pro 6000

```bash
export PROJECT_ID="pantheon-validation-v1"
export ZONE="us-central1-a"
export RUN_ID="gate3-rtx-pro-6000-$(date +%Y%m%d-%H%M%S)"
export REGISTRY="us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

RUNNING=$(gcloud compute instances list --filter="status=RUNNING" --format="value(name)")
[ -n "$RUNNING" ] && { echo "ABORT: $RUNNING running"; exit 1; }

gcloud compute instances create pantheon-$RUN_ID \
  --zone=$ZONE \
  --machine-type=g4-standard-32 \
  --accelerator=type=nvidia-rtx-pro-6000,count=1 \
  --provisioning-model=SPOT \
  --instance-termination-action=DELETE \
  --max-run-duration=120m \
  --network=pantheon-net --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-gpu --image-project=$PROJECT_ID \
  --boot-disk-size=100GB \
  --create-disk=name=models-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  --metadata=RUN_ID=$RUN_ID,GATE=3,install-nvidia-driver=True \
  --no-address
```

**Note:** If the `pantheon-gpu-v1` image doesn't have the latest Blackwell driver, either rebuild the custom image from `c0-deeplearning-common-cu128-*` (Blackwell support) or use `--image-family=common-cu128 --image-project=deeplearning-platform-release` directly for this gate.

### Step 2 — Verify Blackwell card and driver

```bash
until gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="nvidia-smi" 2>/dev/null; do sleep 15; done

gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE

# On VM:
nvidia-smi   # Expected: 1× RTX Pro 6000, ~95GB free, CUDA 12.8+
nvcc --version
sudo mount /dev/disk/by-id/google-*-models /mnt/models
gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
mkdir -p /tmp/evidence/$RUN_ID
```

### Step 3 — H-3.1 test: 72B-Q4 single-card throughput (25 min)

```bash
docker run -d --name vllm \
  --gpus all \
  -v /mnt/models/qwen2.5-72b-awq:/model:ro \
  -p 8000:8000 \
  --shm-size=16g \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name qwen-72b \
  --tensor-parallel-size 1 \
  --gpu-memory-utilization 0.90 \
  --max-model-len 8192 --max-num-seqs 8 \
  --quantization awq_marlin --dtype float16

until curl -sf http://localhost:8000/v1/models; do sleep 20; done

# Single-stream (sustained)
docker run --rm --network host -e TEST=h-3.1-single -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=throughput-sustained --endpoint=http://localhost:8000/v1 \
  --model=qwen-72b --concurrency=1 --duration=240 \
  --output-dir=/tmp/evidence/$RUN_ID/h-3.1-single

# 4-way batched
docker run --rm --network host -e TEST=h-3.1-batch4 -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=throughput-sustained --endpoint=http://localhost:8000/v1 \
  --model=qwen-72b --concurrency=4 --duration=240 \
  --output-dir=/tmp/evidence/$RUN_ID/h-3.1-batch4

# 8-way batched (stress)
docker run --rm --network host -e TEST=h-3.1-batch8 -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=throughput-sustained --endpoint=http://localhost:8000/v1 \
  --model=qwen-72b --concurrency=8 --duration=180 \
  --output-dir=/tmp/evidence/$RUN_ID/h-3.1-batch8

docker stop vllm && docker rm vllm
```

### Step 4 — H-3.2 test: three-model concurrent hosting (15 min)

```bash
# Zeus (72B-Q4) on port 8000, VRAM budget ~40%
docker run -d --name vllm-zeus \
  --gpus all -v /mnt/models/qwen2.5-72b-awq:/model:ro -p 8000:8000 --shm-size=8g \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name qwen-72b \
  --gpu-memory-utilization 0.42 --max-model-len 4096 --max-num-seqs 2 \
  --quantization awq_marlin --dtype float16

# Coder 32B on port 8001, VRAM budget ~35%
docker run -d --name vllm-coder \
  --gpus all -v /mnt/models/qwen2.5-coder-32b-awq:/model:ro -p 8001:8000 --shm-size=8g \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name coder-32b \
  --gpu-memory-utilization 0.35 --max-model-len 4096 --max-num-seqs 4 \
  --quantization awq_marlin --dtype float16

# DeepSeek 16B on port 8002, VRAM budget ~18%
docker run -d --name vllm-deepseek \
  --gpus all -v /mnt/models/deepseek-coder-v2-lite-16b:/model:ro -p 8002:8000 --shm-size=4g \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name deepseek-16b \
  --gpu-memory-utilization 0.18 --max-model-len 4096 --max-num-seqs 4 \
  --quantization awq_marlin --dtype float16

until curl -sf http://localhost:8000/v1/models && curl -sf http://localhost:8001/v1/models && curl -sf http://localhost:8002/v1/models; do
  sleep 15
done

nvidia-smi   # Expected: ~85GB used across three processes

# Concurrent load test
docker run --rm --network host -e TEST=h-3.2-triple -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=multi-endpoint-concurrent \
  --endpoint-a=http://localhost:8000/v1 --model-a=qwen-72b \
  --endpoint-b=http://localhost:8001/v1 --model-b=coder-32b \
  --endpoint-c=http://localhost:8002/v1 --model-c=deepseek-16b \
  --concurrency-a=2 --concurrency-b=3 --concurrency-c=3 \
  --duration=240 \
  --output-dir=/tmp/evidence/$RUN_ID/h-3.2

docker stop vllm-zeus vllm-coder vllm-deepseek
docker rm vllm-zeus vllm-coder vllm-deepseek
```

### Step 5 — H-3.3 test: 32B LoRA single-card (60-90 min)

```bash
gsutil -m cp -r gs://pantheon-fixtures/lora-training-corpus-32b-v1/ /tmp/training

docker run --rm \
  --gpus all --shm-size=16g \
  -v /mnt/models/qwen2.5-coder-32b-awq:/base:ro \
  -v /tmp/training:/data:ro \
  -v /tmp/lora-output:/output \
  -v /tmp/evidence/$RUN_ID/h-3.3:/evidence \
  -e RUN_ID=$RUN_ID -e TEST=h-3.3-lora-32b-single -e WANDB_DISABLED=true \
  $REGISTRY/pantheon-axolotl:main \
  --config=/data/lora-32b-single-card.yml \
  --base-model=/base --output=/output --log-dir=/evidence
```

Axolotl config (`lora-32b-single-card.yml`):

```yaml
base_model: /base
adapter: lora
lora_r: 32
lora_alpha: 64
sequence_len: 2048
micro_batch_size: 2
gradient_accumulation_steps: 4
num_epochs: 1
learning_rate: 2e-4
optimizer: paged_adamw_8bit
bf16: true
# No FSDP needed — single card with 96GB
```

### Step 6 — H-3.4 test: full agent swarm (20 min)

```bash
# Re-launch all three vLLM endpoints from Step 4

# Dispatch 8 canonical agent tasks via Triumvirate
gsutil -m cp -r gs://pantheon-fixtures/agent-tasks-canonical/ /tmp/tasks

docker run --rm --network host \
  -v /tmp/tasks:/tasks:ro \
  -e TEST=h-3.4-full-swarm -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=full-agent-swarm \
  --triumvirate-url=http://localhost:7788 \
  --tasks=/tasks --parallel=4 \
  --eval-rubric=/tasks/eval-rubric.yaml \
  --output-dir=/tmp/evidence/$RUN_ID/h-3.4

cat /tmp/evidence/$RUN_ID/h-3.4/swarm-summary.json
```

**Expected output:**

```json
{
  "test_id": "h-3.4-full-swarm",
  "tasks_dispatched": 8,
  "tasks_completed": 8,
  "tasks_passed_eval": 7,
  "wall_clock_sec": 1680,
  "avg_task_duration_sec": 210,
  "routing_decisions": {
    "zeus": 4,
    "coder": 8,
    "deepseek": 12,
    "escalations_to_zeus": 2
  }
}
```

### Step 7 — Evidence + self-destruct

```bash
cat > /tmp/evidence/$RUN_ID/manifest.json <<EOF
{
  "run_id": "$RUN_ID",
  "gate": 3,
  "gcp_machine_type": "g4-standard-32",
  "gcp_accelerators": ["1x nvidia-rtx-pro-6000-blackwell"],
  "hypotheses_tested": ["H-3.1", "H-3.2", "H-3.3", "H-3.4"]
}
EOF

python3 /opt/pantheon-harness/generate-summary.py --run-id=$RUN_ID --gate=3 \
  --evidence-dir=/tmp/evidence/$RUN_ID --output=/tmp/evidence/$RUN_ID/summary.md

gsutil -m cp -r /tmp/evidence/$RUN_ID gs://pantheon-evidence/gate-3/
python3 /opt/pantheon-harness/cost-tracker.py --run-id=$RUN_ID --output=/tmp/evidence/$RUN_ID/cost-report.json
python3 /opt/pantheon-harness/generate-obsidian-note.py --run-id=$RUN_ID --gate=3 \
  --summary=/tmp/evidence/$RUN_ID/summary.md --output=/tmp/evidence/$RUN_ID/obsidian-note.md
gsutil cp /tmp/evidence/$RUN_ID/{cost-report.json,obsidian-note.md} gs://pantheon-evidence/gate-3/$RUN_ID/

exit
gcloud compute instances delete pantheon-$RUN_ID --zone=$ZONE --quiet
```

---

## Decision rule application

If H-3.1 + H-3.2 + H-3.3 + H-3.4 all PASS → RTX Pro 6000 Blackwell purchase is fully de-risked when trigger fires (per Decision 2 in `30-DECISION-RULES.md`).

If any hypothesis fails → investigate BEFORE committing $13-15K. Most likely culprits: vLLM version vs Blackwell compatibility, thermal behavior on G4 chassis, driver edge cases.

---

## Cost accounting

| Line item | Cost |
|---|---|
| g4-standard-32 Spot, 2 hr max | ~$6-8 |
| **Total per session** | **~$8** |

Re-run Gate 3 quarterly post-purchase to validate drift and compare GCP vs local hardware performance.

---

## What comes after

Gate 3 PASS → proceed to Gate 4 (Athena-scale worker pool on A100s). Runbook: `runbooks/gate-4-athena-swarm.md`.
