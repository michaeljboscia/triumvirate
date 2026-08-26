# Gate 1 — Single L4 Baseline (Single 3090 Proxy)

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

**Purpose:** Establish single-24GB-GPU baseline for the daily-dev workload. Paired with Gate 2, this answers the single-vs-pair 3090 purchase question.

**GCP config:** `g2-standard-4` (1× NVIDIA L4, 24GB)
**Cost:** ~$0.28/hr Spot × 2 hours = **~$0.56 per session**
**Duration:** 2 hours hard cap
**Pre-committed decision rule:** see `30-DECISION-RULES.md` → Decision 1

---

## What this gate proves

The "single 3090" purchase argument rests on: can a single 24GB card handle the daily dev workload comfortably (embeddings, small-model inference, 7-14B LoRA training)? If yes, the 2× pair is overkill for dev-baseline purposes. If no, the pair's 48GB is the right tier.

This gate measures:
- BGE-Large embedding throughput (Pythia ingestion workload)
- Small-model inference (Phi-4 14B, DeepSeek Coder 16B, Qwen Coder 32B at Q4) tok/s
- 7-14B LoRA training feasibility
- Concurrent small-model hosting (2 small models on one 24GB card)

---

## Hypotheses being tested

### H-1.1 — BGE-Large handles Pythia ingestion at interactive speed

**Prediction:** BGE-Large embeds a 50KLOC codebase (Triumvirate repo from fixtures) in ≤ 3 minutes with ≥ 3000 tok/s batch throughput.

**Decision rule:**
- ≥ 3000 tok/s sustained → PASS. Single 24GB is comfortable for ingestion.
- 1500-3000 tok/s → workable but bias toward pair for larger corpora.
- < 1500 tok/s → single card inadequate for moat-building throughput.

### H-1.2 — Small-model inference is usable

**Prediction:** Qwen Coder 32B Q4 runs at ≥ 25 tok/s single-stream. DeepSeek Coder 16B Q8 at ≥ 40 tok/s. Phi-4 14B Q8 at ≥ 50 tok/s.

**Decision rule:**
- All three hit targets → PASS. Specialist fleet viable on single card.
- Any one ≤ 50% of target → weak confirmation; prefer pair for concurrent specialist usage.

### H-1.3 — 7-14B LoRA training completes in reasonable wall-clock

**Prediction:** Qwen 7B LoRA on 10K-sample corpus completes in ≤ 90 min with no OOM.

**Decision rule:**
- ≤ 90 min, no OOM → PASS. Iterative LoRA experiments on small bases are fast.
- 90-180 min → slower than ideal; acceptable for overnight runs.
- OOM or > 3 hrs → fails. Move small LoRAs to cloud burst.

### H-1.4 — Two small models coexist on 24GB

**Prediction:** Phi-4 14B Q8 (~14GB) + DeepSeek Coder 16B Q4 (~10GB) both served via vLLM processes simultaneously without OOM.

**Decision rule:**
- Both serve concurrently, no OOM → PASS. "Concurrent micro-specialist" pattern works at 24GB.
- One crowds the other out → constrained but acceptable; sequential model swap is fine.

---

## Pre-run checklist

- [ ] Preflight complete
- [ ] Gate 0 PASSED (orchestration layer validated)
- [ ] `pantheon-gpu-v1` custom VM image ready
- [ ] Models cached in GCS + PD snapshot `pantheon-models-v1` available
- [ ] Test corpus at `gs://pantheon-fixtures/test-corpus-triumvirate/`
- [ ] LoRA training corpus at `gs://pantheon-fixtures/lora-training-corpus-7b-v1/`

---

## Runbook

### Step 1 — Provision 1× L4 VM

```bash
export PROJECT_ID="pantheon-validation-v1"
export ZONE="us-central1-a"
export RUN_ID="gate1-single-l4-$(date +%Y%m%d-%H%M%S)"
export REGISTRY="us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

RUNNING=$(gcloud compute instances list --filter="status=RUNNING" --format="value(name)")
[ -n "$RUNNING" ] && { echo "ABORT: $RUNNING running"; exit 1; }

gcloud compute instances create pantheon-$RUN_ID \
  --zone=$ZONE \
  --machine-type=g2-standard-4 \
  --accelerator=type=nvidia-l4,count=1 \
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
  --create-disk=name=models-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  --metadata=RUN_ID=$RUN_ID,GATE=1,install-nvidia-driver=True \
  --no-address
```

### Step 2 — SSH + environment verify

```bash
until gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="nvidia-smi" 2>/dev/null; do
  sleep 15
done

gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE

# On VM:
nvidia-smi   # Expected: 1× L4, ~23GB free
sudo mount /dev/disk/by-id/google-*-models /mnt/models
ls /mnt/models/
gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
mkdir -p /tmp/evidence/$RUN_ID
```

### Step 3 — H-1.1 test: BGE-Large embedding throughput (10 min)

```bash
# Launch vLLM or TEI serving BGE-Large
docker run -d --name tei-bge \
  --gpus all \
  -v /mnt/models/bge-large-en-v1.5:/data:ro \
  -p 8080:80 \
  ghcr.io/huggingface/text-embeddings-inference:cuda-1.5 \
  --model-id /data \
  --max-batch-tokens 16384

until curl -sf http://localhost:8080/health; do sleep 5; done

# Download test corpus (50KLOC)
gsutil -m cp -r gs://pantheon-fixtures/test-corpus-triumvirate/ /tmp/corpus

# Run throughput test
docker run --rm \
  --network host \
  -v /tmp/corpus:/corpus:ro \
  -e RUN_ID=$RUN_ID -e TEST=h-1.1-embed \
  $REGISTRY/pantheon-test-harness:main \
  --mode=embedding-throughput \
  --endpoint=http://localhost:8080 \
  --corpus=/corpus \
  --batch-size=32 \
  --output-dir=/tmp/evidence/$RUN_ID/h-1.1

cat /tmp/evidence/$RUN_ID/h-1.1/metrics.json

docker stop tei-bge && docker rm tei-bge
```

**Expected output:**

```json
{
  "test_id": "h-1.1-embed",
  "total_tokens_embedded": 480000,
  "wall_clock_sec": 142,
  "tokens_per_second": 3380,
  "peak_vram_gb": 3.8,
  "verdict": "PASS"
}
```

### Step 4 — H-1.2 test: small-model inference tok/s (30 min)

Test three specialists sequentially (one at a time, single card).

```bash
# Model A: Phi-4 14B Q8
docker run -d --name vllm \
  --gpus all \
  -v /mnt/models/phi-4-14b:/model:ro \
  -p 8000:8000 \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name phi-4-14b \
  --gpu-memory-utilization 0.90 \
  --max-model-len 4096 --max-num-seqs 4 --dtype float16

until curl -sf http://localhost:8000/v1/models; do sleep 10; done

docker run --rm --network host -e TEST=h-1.2-phi4 -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=throughput-sustained --endpoint=http://localhost:8000/v1 \
  --model=phi-4-14b --concurrency=1 --duration=180 \
  --output-dir=/tmp/evidence/$RUN_ID/h-1.2-phi4

docker stop vllm && docker rm vllm

# Model B: DeepSeek Coder 16B Q8
docker run -d --name vllm \
  --gpus all \
  -v /mnt/models/deepseek-coder-v2-lite-16b:/model:ro \
  -p 8000:8000 \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name deepseek-coder-16b \
  --gpu-memory-utilization 0.90 --max-model-len 4096 --max-num-seqs 4 --dtype float16

until curl -sf http://localhost:8000/v1/models; do sleep 10; done

docker run --rm --network host -e TEST=h-1.2-deepseek -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=throughput-sustained --endpoint=http://localhost:8000/v1 \
  --model=deepseek-coder-16b --concurrency=1 --duration=180 \
  --output-dir=/tmp/evidence/$RUN_ID/h-1.2-deepseek

docker stop vllm && docker rm vllm

# Model C: Qwen Coder 32B Q4
docker run -d --name vllm \
  --gpus all \
  -v /mnt/models/qwen2.5-coder-32b-awq:/model:ro \
  -p 8000:8000 \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name qwen-coder-32b \
  --gpu-memory-utilization 0.92 --max-model-len 4096 --max-num-seqs 2 \
  --quantization awq_marlin --dtype float16

until curl -sf http://localhost:8000/v1/models; do sleep 15; done

docker run --rm --network host -e TEST=h-1.2-qwen32 -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=throughput-sustained --endpoint=http://localhost:8000/v1 \
  --model=qwen-coder-32b --concurrency=1 --duration=180 \
  --output-dir=/tmp/evidence/$RUN_ID/h-1.2-qwen32

docker stop vllm && docker rm vllm
```

### Step 5 — H-1.3 test: 7-14B LoRA training (60-90 min)

```bash
# Download training corpus
gsutil -m cp -r gs://pantheon-fixtures/lora-training-corpus-7b-v1/ /tmp/training

# Launch Axolotl LoRA training
docker run --rm \
  --gpus all \
  --shm-size=8g \
  -v /mnt/models/qwen2.5-7b:/base:ro \
  -v /tmp/training:/data:ro \
  -v /tmp/lora-output:/output \
  -v /tmp/evidence/$RUN_ID/h-1.3:/evidence \
  -e RUN_ID=$RUN_ID -e TEST=h-1.3-lora-7b \
  -e WANDB_DISABLED=true \
  $REGISTRY/pantheon-axolotl:main \
  --config=/data/lora-7b-single-card.yml \
  --base-model=/base --output=/output --log-dir=/evidence

cat /tmp/evidence/$RUN_ID/h-1.3/training-summary.json
```

### Step 6 — H-1.4 test: concurrent small-model hosting (15 min)

```bash
# Phi-4 14B on port 8000
docker run -d --name vllm-phi4 \
  --gpus all \
  -v /mnt/models/phi-4-14b:/model:ro \
  -p 8000:8000 \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name phi-4-14b \
  --gpu-memory-utilization 0.48 --max-model-len 2048 --max-num-seqs 2

# DeepSeek Coder 16B Q4 on port 8001 (concurrent)
docker run -d --name vllm-deepseek \
  --gpus all \
  -v /mnt/models/deepseek-coder-v2-lite-16b:/model:ro \
  -p 8001:8000 \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name deepseek-coder-16b \
  --gpu-memory-utilization 0.40 --max-model-len 2048 --max-num-seqs 2 \
  --quantization awq_marlin --dtype float16

# Wait for both
until curl -sf http://localhost:8000/v1/models && curl -sf http://localhost:8001/v1/models; do
  sleep 10
done

nvidia-smi   # Should show ~22GB used across two processes

# Concurrent load against both
docker run --rm --network host -e TEST=h-1.4-concurrent -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=multi-endpoint-concurrent \
  --endpoint-a=http://localhost:8000/v1 --model-a=phi-4-14b \
  --endpoint-b=http://localhost:8001/v1 --model-b=deepseek-coder-16b \
  --concurrency-a=2 --concurrency-b=2 --duration=180 \
  --output-dir=/tmp/evidence/$RUN_ID/h-1.4

docker stop vllm-phi4 vllm-deepseek && docker rm vllm-phi4 vllm-deepseek
```

### Step 7 — Evidence + self-destruct

```bash
nvidia-smi --query-gpu=timestamp,index,utilization.gpu,memory.used --format=csv > /tmp/evidence/$RUN_ID/nvidia-smi.csv

cat > /tmp/evidence/$RUN_ID/manifest.json <<EOF
{
  "run_id": "$RUN_ID",
  "gate": 1,
  "gcp_machine_type": "g2-standard-4",
  "gcp_accelerators": ["1x nvidia-l4"],
  "hypotheses_tested": ["H-1.1", "H-1.2", "H-1.3", "H-1.4"]
}
EOF

python3 /opt/pantheon-harness/generate-summary.py --run-id=$RUN_ID --gate=1 \
  --evidence-dir=/tmp/evidence/$RUN_ID --output=/tmp/evidence/$RUN_ID/summary.md

gsutil -m cp -r /tmp/evidence/$RUN_ID gs://pantheon-evidence/gate-1/

python3 /opt/pantheon-harness/cost-tracker.py --run-id=$RUN_ID \
  --output=/tmp/evidence/$RUN_ID/cost-report.json
gsutil cp /tmp/evidence/$RUN_ID/cost-report.json gs://pantheon-evidence/gate-1/$RUN_ID/

python3 /opt/pantheon-harness/generate-obsidian-note.py --run-id=$RUN_ID --gate=1 \
  --summary=/tmp/evidence/$RUN_ID/summary.md --output=/tmp/evidence/$RUN_ID/obsidian-note.md
gsutil cp /tmp/evidence/$RUN_ID/obsidian-note.md gs://pantheon-evidence/gate-1/$RUN_ID/

exit
gcloud compute instances delete pantheon-$RUN_ID --zone=$ZONE --quiet
```

### Step 8 — Drop note into vault

```bash
gsutil cp gs://pantheon-evidence/gate-1/$RUN_ID/obsidian-note.md ~/Documents/pantheon-vault/runs/$RUN_ID.md
cd ~/Documents/pantheon-vault && git add runs/$RUN_ID.md && git commit -m "gate-1 run $RUN_ID"
```

---

## Cost accounting

| Line item | Cost |
|---|---|
| g2-standard-4 Spot, 2 hr max | ~$0.56 |
| **Total per session** | **~$0.60** |

---

## What comes after

Gate 1 PASS + Gate 2 PASS → combined evidence powers the **single vs pair 3090 decision**. Apply Decision 1 from `30-DECISION-RULES.md`.

Gate 1 FAIL → adjust model choices or revisit whether 24GB is viable at all for local dev baseline.

Gate 1 PASS → proceed to Gate 2 (`runbooks/gate-2-dual-l4.md`) the same day or week.
