# Gate 4 — Athena-Scale Parallel Worker Swarm

**Purpose:** Validate Pantheon's "parallel worker pool" thesis at production scale. This is the **core architectural test** — can multiple AI workers produce valid, mergeable code in parallel against real project tasks?

**GCP config:** `a2-ultragpu-4g` (4× A100 80GB, 320GB total VRAM)
**Cost:** ~$8/hr Spot × 2 hrs = **~$16 per session**
**Duration:** 2 hours hard cap
**Pre-committed decision rule:** see `30-DECISION-RULES.md` → Decision 5 (thesis validation)

---

## Why this gate matters

The original Pantheon thesis was: "multiple AI workers, each in its own Git worktree, can produce mergeable code that collectively solves a larger problem." If this gate passes, the thesis is architecturally validated at production capacity. If it fails, Pantheon needs redesign before further investment.

**4× A100 80GB = 320GB VRAM** — this is the closest GCP equivalent to 2× DGX Spark Athena (256GB unified). Upper-bound production-realistic Athena capacity.

---

## Hypotheses being tested

### H-4.1 — Parallel worktrees + parallel dispatch = mergeable code

**Prediction:** Triumvirate creates 4 Git worktrees, dispatches 4 agent tasks simultaneously to workers, each produces code, all 4 merge to test branch cleanly.

**Decision rule:**
- 4/4 tasks complete, 4/4 worktrees merge without conflict → PASS. Core thesis validated.
- 3/4 complete, 3/4 merge → acceptable, investigate the failure.
- < 3/4 → thesis needs refinement before more investment.

### H-4.2 — Pythia context injection produces valid code

**Prediction:** Workers receive Pythia-retrieved context for their tasks. Generated code uses correct import paths, references correct existing functions, respects existing project conventions.

**Decision rule:**
- ≥ 80% of generated code passes basic validation (imports, syntax, signatures) → PASS.
- 50-80% → context injection needs tuning.
- < 50% → Pythia retrieval or prompt template broken.

### H-4.3 — Per-task wall-clock under production budget

**Prediction:** Median task completion ≤ 15 min when 4 tasks run simultaneously. No task exceeds 30 min.

**Decision rule:**
- Median ≤ 15 min, max ≤ 30 min → PASS.
- Median 15-25 min → acceptable, room to optimize.
- Median > 25 min OR any task > 45 min → investigate bottleneck (GPU sharing? KV cache? dispatch overhead?).

### H-4.4 — vLLM tensor parallelism vs multi-process hosting

**Prediction:** Two serving modes compared:
- Mode A: Single vLLM process, Qwen 72B TP=4 across all cards, batched requests
- Mode B: Four vLLM processes (one per card), each serving the same or different model

Pattern that produces better throughput + predictability wins.

**Decision rule:**
- Measure aggregate tok/s + p95 latency for both modes.
- Lower p95 + higher aggregate throughput wins, adopted for Pantheon's Athena config.

---

## Pre-run checklist

- [ ] Preflight complete
- [ ] Gates 0-3 passed
- [ ] A100 80GB quota verified (need ≥ 4)
- [ ] Pythia corpus available on GCS
- [ ] 4 canonical test tasks with known-good solutions + eval rubrics

---

## Runbook

### Step 1 — Provision a2-ultragpu-4g

```bash
export PROJECT_ID="pantheon-validation-v1"
export ZONE="us-central1-a"
export RUN_ID="gate4-athena-swarm-$(date +%Y%m%d-%H%M%S)"
export REGISTRY="us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

RUNNING=$(gcloud compute instances list --filter="status=RUNNING" --format="value(name)")
[ -n "$RUNNING" ] && { echo "ABORT: $RUNNING running"; exit 1; }

gcloud compute instances create pantheon-$RUN_ID \
  --zone=$ZONE \
  --machine-type=a2-ultragpu-4g \
  --provisioning-model=SPOT \
  --instance-termination-action=DELETE \
  --max-run-duration=120m \
  --network=pantheon-net --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-gpu --image-project=$PROJECT_ID \
  --boot-disk-size=100GB \
  --create-disk=name=models-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  --metadata=RUN_ID=$RUN_ID,GATE=4,install-nvidia-driver=True \
  --no-address
```

### Step 2 — SSH + verify 4× A100

```bash
until gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="nvidia-smi" 2>/dev/null; do sleep 20; done

gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE

# On VM:
nvidia-smi   # Expected: 4× A100 80GB, ~80GB free each
sudo mount /dev/disk/by-id/google-*-models /mnt/models
gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
mkdir -p /tmp/evidence/$RUN_ID
```

### Step 3 — Mode A: Single vLLM, Qwen 72B, TP=4 (30 min)

```bash
docker run -d --name vllm-tp4 \
  --gpus all \
  -v /mnt/models/qwen2.5-72b-awq:/model:ro \
  -p 8000:8000 --shm-size=32g \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name qwen-72b-tp4 \
  --tensor-parallel-size 4 \
  --gpu-memory-utilization 0.90 \
  --max-model-len 8192 --max-num-seqs 16 \
  --quantization awq_marlin --dtype float16

until curl -sf http://localhost:8000/v1/models; do sleep 30; done

# Load tests at various concurrency levels
for CONC in 1 4 8 16; do
  docker run --rm --network host \
    -e TEST=h-4.4a-tp4-conc$CONC -e RUN_ID=$RUN_ID \
    $REGISTRY/pantheon-test-harness:main \
    --mode=throughput-sustained --endpoint=http://localhost:8000/v1 \
    --model=qwen-72b-tp4 --concurrency=$CONC --duration=120 \
    --output-dir=/tmp/evidence/$RUN_ID/h-4.4a-tp4-conc$CONC
done

docker stop vllm-tp4 && docker rm vllm-tp4
```

### Step 4 — Mode B: 4× separate vLLM processes, one per card (30 min)

```bash
for GPU_IDX in 0 1 2 3; do
  PORT=$((8000 + GPU_IDX))
  docker run -d --name vllm-card-$GPU_IDX \
    --gpus "\"device=$GPU_IDX\"" \
    -v /mnt/models/qwen2.5-72b-awq:/model:ro \
    -p ${PORT}:8000 --shm-size=8g \
    $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
    --model /model --served-model-name qwen-72b-card$GPU_IDX \
    --tensor-parallel-size 1 \
    --gpu-memory-utilization 0.85 \
    --max-model-len 4096 --max-num-seqs 4 \
    --quantization awq_marlin --dtype float16
done

# Wait for all 4 healthy
for PORT in 8000 8001 8002 8003; do
  until curl -sf http://localhost:$PORT/v1/models; do sleep 15; done
done

# Run concurrent load across all 4 endpoints
docker run --rm --network host \
  -e TEST=h-4.4b-4proc -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=multi-endpoint-concurrent \
  --endpoint-a=http://localhost:8000/v1 --model-a=qwen-72b-card0 \
  --endpoint-b=http://localhost:8001/v1 --model-b=qwen-72b-card1 \
  --endpoint-c=http://localhost:8002/v1 --model-c=qwen-72b-card2 \
  --endpoint-d=http://localhost:8003/v1 --model-d=qwen-72b-card3 \
  --concurrency-a=4 --concurrency-b=4 --concurrency-c=4 --concurrency-d=4 \
  --duration=180 \
  --output-dir=/tmp/evidence/$RUN_ID/h-4.4b-4proc

docker stop vllm-card-0 vllm-card-1 vllm-card-2 vllm-card-3
docker rm vllm-card-0 vllm-card-1 vllm-card-2 vllm-card-3
```

### Step 5 — H-4.1, H-4.2, H-4.3 test: parallel worktree swarm (40 min)

```bash
# Use the WINNING serving mode from Step 3/4 comparison
# Assume Mode A won — redeploy TP=4

docker run -d --name vllm-tp4 \
  --gpus all \
  -v /mnt/models/qwen2.5-72b-awq:/model:ro \
  -p 8000:8000 --shm-size=32g \
  $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
  --model /model --served-model-name qwen-72b --tensor-parallel-size 4 \
  --gpu-memory-utilization 0.90 --max-model-len 8192 --max-num-seqs 16 \
  --quantization awq_marlin --dtype float16

until curl -sf http://localhost:8000/v1/models; do sleep 30; done

# Start Triumvirate daemon with Pythia context
gsutil cp gs://pantheon-pythia-corpus/pythia-corpus-v1.tar.gz /tmp/
tar xzf /tmp/pythia-corpus-v1.tar.gz -C /tmp/

docker run -d --name triumvirate \
  --network host \
  -v /tmp/pythia-snapshot-v1.db:/var/pythia.db:ro \
  -e TRIUMVIRATE_CONFIG=/etc/triumvirate/gate-4.toml \
  -e PYTHIA_DB=/var/pythia.db \
  $REGISTRY/pantheon-triumvirate:main

until curl -sf http://localhost:7788/status; do sleep 10; done

# Download canonical 4-task set
gsutil -m cp -r gs://pantheon-fixtures/agent-tasks-swarm-4/ /tmp/tasks

# Dispatch all 4 simultaneously via Triumvirate worktree mode
docker run --rm --network host \
  -v /tmp/tasks:/tasks:ro \
  -e TEST=h-4.1-swarm -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=parallel-worktree-swarm \
  --triumvirate-url=http://localhost:7788 \
  --tasks=/tasks \
  --num-parallel=4 \
  --eval-rubric=/tasks/eval-rubric.yaml \
  --output-dir=/tmp/evidence/$RUN_ID/h-4.1

cat /tmp/evidence/$RUN_ID/h-4.1/swarm-summary.json
```

**Expected output:**

```json
{
  "test_id": "h-4.1-swarm",
  "worktrees_created": 4,
  "tasks_completed": 4,
  "tasks_passed_eval": 4,
  "tasks_merged_cleanly": 4,
  "merge_conflicts": 0,
  "wall_clock_median_sec": 720,
  "wall_clock_max_sec": 1020,
  "pythia_context_hit_rate": 0.87,
  "pythia_context_relevance_score": 0.82,
  "verdict": "PASS"
}
```

### Step 6 — Collect evidence + self-destruct

```bash
cat > /tmp/evidence/$RUN_ID/manifest.json <<EOF
{
  "run_id": "$RUN_ID",
  "gate": 4,
  "gcp_machine_type": "a2-ultragpu-4g",
  "gcp_accelerators": ["4x nvidia-a100-80gb"],
  "hypotheses_tested": ["H-4.1", "H-4.2", "H-4.3", "H-4.4"]
}
EOF

python3 /opt/pantheon-harness/generate-summary.py --run-id=$RUN_ID --gate=4 \
  --evidence-dir=/tmp/evidence/$RUN_ID --output=/tmp/evidence/$RUN_ID/summary.md

gsutil -m cp -r /tmp/evidence/$RUN_ID gs://pantheon-evidence/gate-4/
python3 /opt/pantheon-harness/cost-tracker.py --run-id=$RUN_ID --output=/tmp/evidence/$RUN_ID/cost-report.json
python3 /opt/pantheon-harness/generate-obsidian-note.py --run-id=$RUN_ID --gate=4 \
  --summary=/tmp/evidence/$RUN_ID/summary.md --output=/tmp/evidence/$RUN_ID/obsidian-note.md
gsutil cp /tmp/evidence/$RUN_ID/{cost-report.json,obsidian-note.md} gs://pantheon-evidence/gate-4/$RUN_ID/

exit
gcloud compute instances delete pantheon-$RUN_ID --zone=$ZONE --quiet
```

---

## Cost accounting

| Line item | Cost |
|---|---|
| a2-ultragpu-4g Spot, 2 hr max | ~$16 |
| **Total per session** | **~$16-18** |

---

## What comes after

Gate 4 PASS = **core Pantheon thesis empirically validated at production scale.** Proceed to Gate 5 (full trinity at true prod equivalence) or directly to Gate 6 (air-gap) depending on priorities.

Gate 4 FAIL = architectural investigation required BEFORE spending more. Most common failures: Pythia context retrieval misaligned, worktree isolation leaky, merge-conflict policy broken.
