# Gate 5 — Full Trinity at Production Equivalence

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

**Purpose:** Validate Pantheon's complete three-role architecture (Zeus + Athena + Vulcan) at full production capacity — Llama 405B as Zeus architect, parallel worker pool as Athena, fast-fix specialist as Vulcan — all driving Triumvirate end-to-end. This gate answers "what does the Pantheon Rack/Closet tier actually deliver?"

**GCP config:** Multi-VM composition
- Zeus VM: `a2-ultragpu-8g` (8× A100 80GB = 640GB VRAM) for Llama 405B-Q4
- Athena VM: `a2-ultragpu-4g` (4× A100 80GB = 320GB VRAM) for worker pool
- Vulcan VM: `g2-standard-24` (2× L4 48GB) for fast-fix coder
- Orchestrator: `e2-standard-4` running Triumvirate

**Cost:** ~$20-25/hr aggregate Spot × 1-2 hrs = **~$25-50 per session**
**Duration:** 2 hours hard cap across all VMs
**Pre-committed decision rule:** see `30-DECISION-RULES.md` → Decision 6

---

## What this gate proves

Everything assumed in the "original Pantheon intent." This is the empirical evidence of what $100-400K of local hardware would deliver:

- Llama 405B-Q4 running as real-time architect/reviewer
- Parallel Athena workers dispatching concurrent code-gen tasks
- Vulcan intercepting syntax/type errors for fast-fix path
- Zeus reviewing outputs and driving convergence
- End-to-end task flow: dispatch → worker → vulcan-fix-or-pass → zeus-review → merge

**This gate is the "ultimate Pantheon Rack validation" — run it when deciding whether to pursue enterprise-tier hardware purchase.**

---

## Hypotheses being tested

### H-5.1 — 405B-Q4 runs as orchestrator at usable speed

**Prediction:** Llama 3.1 405B-Q4 (AWQ) on 8× A100 80GB achieves ≥ 30 tok/s single-stream, ≥ 15 tok/s per stream under 4-way batching.

**Decision rule:**
- ≥ 30 tok/s single, ≥ 15 tok/s batched → PASS. Real-time architect capability.
- 15-30 single → usable as "slow oracle" pattern.
- < 15 → investigate TP config, quantization.

### H-5.2 — Full trinity end-to-end task completion

**Prediction:** 8 canonical Pantheon tasks dispatched via Triumvirate, routed through Athena workers, Vulcan intercepts failures, Zeus reviews final outputs. ≥ 6/8 pass full eval with ≤ 3 review cycles average.

**Decision rule:**
- ≥ 6/8 pass, avg cycles ≤ 3 → PASS. Three-role architecture works.
- 4-5/8 pass → partial; identify failure modes.
- < 4/8 → architecture review cycle broken.

### H-5.3 — Zeus APPROVE/REJECT protocol stays structured

**Prediction:** Zeus's review responses parse as structured `{verdict, feedback, confidence}` JSON in ≥ 90% of cases, not unstructured prose.

**Decision rule:**
- ≥ 90% structured → PASS.
- 70-90% → prompt engineering needed.
- < 70% → Zeus model not suitable, revisit model choice.

### H-5.4 — Vulcan fast-fix reduces Zeus review load

**Prediction:** When a worker output fails local validation (syntax/type error), Vulcan's targeted-patch response fixes ≥ 50% without escalating to Zeus.

**Decision rule:**
- ≥ 50% Vulcan-fix success → PASS. Fast-fix pattern earning its keep.
- 30-50% → marginal; Vulcan prompt tuning needed.
- < 30% → Vulcan either too aggressive or not useful; reconsider the role.

### H-5.5 — End-to-end wall-clock meets production budget

**Prediction:** 8 tasks from dispatch to final merge complete in ≤ 60 min total wall-clock.

**Decision rule:**
- ≤ 60 min → PASS.
- 60-90 min → acceptable.
- > 90 min → optimize bottleneck (Zeus review, Pythia retrieval, or dispatch serialization).

---

## Pre-run checklist

- [ ] Preflight complete, Gates 0-4 passed
- [ ] A100 80GB quota ≥ 12 (8 for Zeus + 4 for Athena)
- [ ] L4 quota ≥ 2 for Vulcan
- [ ] Llama 405B-Q4 AWQ cached at `gs://pantheon-models/llama-3.1-405b-awq/`
- [ ] Canonical 8-task set at `gs://pantheon-fixtures/agent-tasks-canonical/`
- [ ] Evaluation rubrics per task type
- [ ] Firewall rule allows internal VPC traffic between VMs (configured in preflight)

---

## Runbook

Multi-VM gate: all four VMs provision concurrently, hard kill at 120 min each.

### Step 1 — Provision all four VMs in parallel

```bash
export PROJECT_ID="pantheon-validation-v1"
export ZONE="us-central1-a"
export RUN_ID="gate5-full-trinity-$(date +%Y%m%d-%H%M%S)"
export REGISTRY="us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

RUNNING=$(gcloud compute instances list --filter="status=RUNNING" --format="value(name)")
[ -n "$RUNNING" ] && { echo "ABORT: $RUNNING running"; exit 1; }

# Zeus VM — 8× A100 80GB for 405B
gcloud compute instances create pantheon-$RUN_ID-zeus \
  --zone=$ZONE --machine-type=a2-ultragpu-8g \
  --provisioning-model=SPOT --instance-termination-action=DELETE \
  --max-run-duration=120m \
  --network=pantheon-net --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-gpu --image-project=$PROJECT_ID \
  --boot-disk-size=100GB \
  --create-disk=name=models-zeus-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  --metadata=RUN_ID=$RUN_ID,GATE=5,ROLE=zeus,install-nvidia-driver=True \
  --no-address &

# Athena VM — 4× A100 80GB for worker pool
gcloud compute instances create pantheon-$RUN_ID-athena \
  --zone=$ZONE --machine-type=a2-ultragpu-4g \
  --provisioning-model=SPOT --instance-termination-action=DELETE \
  --max-run-duration=120m \
  --network=pantheon-net --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-gpu --image-project=$PROJECT_ID \
  --boot-disk-size=100GB \
  --create-disk=name=models-athena-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  --metadata=RUN_ID=$RUN_ID,GATE=5,ROLE=athena,install-nvidia-driver=True \
  --no-address &

# Vulcan VM — 2× L4 48GB
gcloud compute instances create pantheon-$RUN_ID-vulcan \
  --zone=$ZONE --machine-type=g2-standard-24 \
  --accelerator=type=nvidia-l4,count=2 \
  --provisioning-model=SPOT --instance-termination-action=DELETE \
  --max-run-duration=120m \
  --network=pantheon-net --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-gpu --image-project=$PROJECT_ID \
  --boot-disk-size=100GB \
  --create-disk=name=models-vulcan-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  --metadata=RUN_ID=$RUN_ID,GATE=5,ROLE=vulcan,install-nvidia-driver=True \
  --no-address &

# Orchestrator VM
gcloud compute instances create pantheon-$RUN_ID-orch \
  --zone=$ZONE --machine-type=e2-standard-4 \
  --provisioning-model=SPOT --instance-termination-action=DELETE \
  --max-run-duration=120m \
  --network=pantheon-net --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-orchestrator --image-project=$PROJECT_ID \
  --metadata=RUN_ID=$RUN_ID,GATE=5,ROLE=orch \
  --no-address &

wait
```

### Step 2 — Get internal IPs + wait for all ready

```bash
sleep 60   # Let VMs boot

ZEUS_IP=$(gcloud compute instances describe pantheon-$RUN_ID-zeus --zone=$ZONE --format='value(networkInterfaces[0].networkIP)')
ATHENA_IP=$(gcloud compute instances describe pantheon-$RUN_ID-athena --zone=$ZONE --format='value(networkInterfaces[0].networkIP)')
VULCAN_IP=$(gcloud compute instances describe pantheon-$RUN_ID-vulcan --zone=$ZONE --format='value(networkInterfaces[0].networkIP)')
ORCH_IP=$(gcloud compute instances describe pantheon-$RUN_ID-orch --zone=$ZONE --format='value(networkInterfaces[0].networkIP)')

echo "Zeus: $ZEUS_IP | Athena: $ATHENA_IP | Vulcan: $VULCAN_IP | Orch: $ORCH_IP"

# Wait for all GPU VMs to have nvidia-smi working
for VM in zeus athena vulcan; do
  until gcloud compute ssh pantheon-$RUN_ID-$VM --zone=$ZONE --command="nvidia-smi" 2>/dev/null; do
    echo "Waiting for $VM..."; sleep 20
  done
done
```

### Step 3 — Launch vLLM on Zeus (Llama 405B-Q4)

```bash
gcloud compute ssh pantheon-$RUN_ID-zeus --zone=$ZONE --command="
  sudo mount /dev/disk/by-id/google-*-models /mnt/models
  gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
  docker run -d --name vllm-zeus \
    --gpus all \
    -v /mnt/models/llama-3.1-405b-awq:/model:ro \
    -p 8000:8000 --shm-size=64g \
    $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
    --model /model --served-model-name llama-405b \
    --tensor-parallel-size 8 \
    --gpu-memory-utilization 0.90 \
    --max-model-len 8192 --max-num-seqs 8 \
    --quantization awq_marlin --dtype float16
"

# Wait 10-15 min for 405B model load
until curl -sf http://$ZEUS_IP:8000/v1/models 2>/dev/null; do
  echo "Waiting for Zeus vLLM model load..."; sleep 30
done
```

### Step 4 — Launch vLLM on Athena (Qwen 72B TP=4)

```bash
gcloud compute ssh pantheon-$RUN_ID-athena --zone=$ZONE --command="
  sudo mount /dev/disk/by-id/google-*-models /mnt/models
  gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
  docker run -d --name vllm-athena \
    --gpus all \
    -v /mnt/models/qwen2.5-72b-awq:/model:ro \
    -p 8000:8000 --shm-size=32g \
    $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
    --model /model --served-model-name qwen-72b \
    --tensor-parallel-size 4 \
    --gpu-memory-utilization 0.90 \
    --max-model-len 8192 --max-num-seqs 16 \
    --quantization awq_marlin --dtype float16
"

until curl -sf http://$ATHENA_IP:8000/v1/models; do sleep 30; done
```

### Step 5 — Launch vLLM on Vulcan (Qwen Coder 32B-Q4)

```bash
gcloud compute ssh pantheon-$RUN_ID-vulcan --zone=$ZONE --command="
  sudo mount /dev/disk/by-id/google-*-models /mnt/models
  gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
  docker run -d --name vllm-vulcan \
    --gpus all \
    -v /mnt/models/qwen2.5-coder-32b-awq:/model:ro \
    -p 8000:8000 --shm-size=16g \
    $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
    --model /model --served-model-name qwen-coder-32b-vulcan \
    --tensor-parallel-size 2 \
    --gpu-memory-utilization 0.90 \
    --max-model-len 4096 --max-num-seqs 8 \
    --quantization awq_marlin --dtype float16
"

until curl -sf http://$VULCAN_IP:8000/v1/models; do sleep 20; done
```

### Step 6 — Launch Triumvirate orchestrator

```bash
gcloud compute ssh pantheon-$RUN_ID-orch --zone=$ZONE --command="
  gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
  gsutil cp gs://pantheon-pythia-corpus/pythia-corpus-v1.tar.gz /tmp/
  tar xzf /tmp/pythia-corpus-v1.tar.gz -C /tmp/

  cat > /tmp/gate-5.toml <<EOF
[inference.endpoints.zeus]
url = \"http://$ZEUS_IP:8000/v1\"
model = \"llama-405b\"

[inference.endpoints.athena]
url = \"http://$ATHENA_IP:8000/v1\"
model = \"qwen-72b\"

[inference.endpoints.vulcan]
url = \"http://$VULCAN_IP:8000/v1\"
model = \"qwen-coder-32b-vulcan\"

[dispatch.rules]
default_worker_endpoint = \"athena\"
reviewer_endpoint = \"zeus\"
fast_fix_endpoint = \"vulcan\"
EOF

  docker run -d --name triumvirate --network host \
    -v /tmp/gate-5.toml:/etc/triumvirate/config.toml:ro \
    -v /tmp/pythia-snapshot-v1.db:/var/pythia.db:ro \
    -e TRIUMVIRATE_CONFIG=/etc/triumvirate/config.toml \
    -e PYTHIA_DB=/var/pythia.db \
    $REGISTRY/pantheon-triumvirate:main
"

until curl -sf http://$ORCH_IP:7788/status; do sleep 10; done
```

### Step 7 — H-5.1 test: 405B throughput

```bash
# From orchestrator VM, run throughput harness against Zeus
gcloud compute ssh pantheon-$RUN_ID-orch --zone=$ZONE --command="
  for CONC in 1 4 8; do
    docker run --rm --network host \
      -e TEST=h-5.1-conc\$CONC -e RUN_ID=$RUN_ID \
      $REGISTRY/pantheon-test-harness:main \
      --mode=throughput-sustained --endpoint=http://$ZEUS_IP:8000/v1 \
      --model=llama-405b --concurrency=\$CONC --duration=180 \
      --output-dir=/tmp/evidence/$RUN_ID/h-5.1-conc\$CONC
  done
"
```

### Step 8 — H-5.2 through H-5.5 test: full trinity agent swarm (40 min)

```bash
gcloud compute ssh pantheon-$RUN_ID-orch --zone=$ZONE --command="
  gsutil -m cp -r gs://pantheon-fixtures/agent-tasks-canonical/ /tmp/tasks

  docker run --rm --network host \
    -v /tmp/tasks:/tasks:ro \
    -e TEST=h-5.2-full-trinity -e RUN_ID=$RUN_ID \
    $REGISTRY/pantheon-test-harness:main \
    --mode=full-trinity-swarm \
    --triumvirate-url=http://$ORCH_IP:7788 \
    --tasks=/tasks --num-parallel=4 \
    --enable-zeus-review=true \
    --enable-vulcan-fastfix=true \
    --eval-rubric=/tasks/eval-rubric.yaml \
    --output-dir=/tmp/evidence/$RUN_ID/h-5.2

  cat /tmp/evidence/$RUN_ID/h-5.2/trinity-summary.json
"
```

### Step 9 — Evidence collection + all-VM self-destruct

```bash
gcloud compute ssh pantheon-$RUN_ID-orch --zone=$ZONE --command="
  cat > /tmp/evidence/$RUN_ID/manifest.json <<EOF
{
  \"run_id\": \"$RUN_ID\",
  \"gate\": 5,
  \"gcp_machine_types\": [\"a2-ultragpu-8g\", \"a2-ultragpu-4g\", \"g2-standard-24\", \"e2-standard-4\"],
  \"gcp_accelerators\": [\"8x nvidia-a100-80gb\", \"4x nvidia-a100-80gb\", \"2x nvidia-l4\"],
  \"hypotheses_tested\": [\"H-5.1\", \"H-5.2\", \"H-5.3\", \"H-5.4\", \"H-5.5\"]
}
EOF

  python3 /opt/pantheon-harness/generate-summary.py --run-id=$RUN_ID --gate=5 \
    --evidence-dir=/tmp/evidence/$RUN_ID --output=/tmp/evidence/$RUN_ID/summary.md

  gsutil -m cp -r /tmp/evidence/$RUN_ID gs://pantheon-evidence/gate-5/
  python3 /opt/pantheon-harness/cost-tracker.py --run-id=$RUN_ID --output=/tmp/evidence/$RUN_ID/cost-report.json
  python3 /opt/pantheon-harness/generate-obsidian-note.py --run-id=$RUN_ID --gate=5 \
    --summary=/tmp/evidence/$RUN_ID/summary.md --output=/tmp/evidence/$RUN_ID/obsidian-note.md
  gsutil cp /tmp/evidence/$RUN_ID/{cost-report.json,obsidian-note.md} gs://pantheon-evidence/gate-5/$RUN_ID/
"

# Delete all four VMs
for VM in zeus athena vulcan orch; do
  gcloud compute instances delete pantheon-$RUN_ID-$VM --zone=$ZONE --quiet &
done
wait
```

---

## Cost accounting

| Line item | Cost |
|---|---|
| Zeus (a2-ultragpu-8g, 2 hr) | ~$24 |
| Athena (a2-ultragpu-4g, 2 hr) | ~$16 |
| Vulcan (g2-standard-24, 2 hr) | ~$0.84 |
| Orchestrator (e2-standard-4, 2 hr) | ~$0.30 |
| PD snapshots (transient) | ~$0.20 |
| **Total per session** | **~$42** |

Budget across 2-3 validation runs: ~$100-150 total for Gate 5.

---

## What comes after

Gate 5 PASS = full Pantheon architecture is empirically validated at production equivalence. Enterprise pitch is grounded in real data. Pantheon Rack tier purchase decisions ($150-500K hardware) can be made confidently.

Gate 5 FAIL = significant architecture investigation required. Most common failures: 405B-review-latency dominates wall-clock, Vulcan fast-fix not actually fixing things, Zeus responses losing structure under load.

Next: Gate 6 (air-gap sanity). Runbook: `runbooks/gate-6-airgap-sanity.md`.
