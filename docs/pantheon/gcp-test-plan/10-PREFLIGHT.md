# Pantheon GCP Preflight — Before the First GPU Burns

**Purpose:** Every step required to stand up the Pantheon GCP test environment before executing any gate. Complete this document fully before running Gate 0. All steps here are $0 GPU spend.

**Expected duration:** 12-15 hours of focused work across 2-4 days
**Expected spend:** $6-12 one-time + ~$15/mo storage ongoing
**Prerequisite:** GCP account, billing enabled, Gemini Ultra subscription credit applied

---

## Phase 1 — Project + billing + quota (2-3 hours, $0)

### Step 1.1 — Create GCP project

```bash
export PROJECT_ID="pantheon-validation-v1"
export BILLING_ACCOUNT="YOUR_BILLING_ACCOUNT_ID"   # from gcloud billing accounts list
export DEFAULT_REGION="us-central1"
export DEFAULT_ZONE="us-central1-a"

gcloud projects create $PROJECT_ID --name="Pantheon Validation v1"
gcloud config set project $PROJECT_ID
gcloud beta billing projects link $PROJECT_ID --billing-account=$BILLING_ACCOUNT
gcloud config set compute/region $DEFAULT_REGION
gcloud config set compute/zone $DEFAULT_ZONE
```

### Step 1.2 — Enable required APIs

```bash
gcloud services enable \
  compute.googleapis.com \
  artifactregistry.googleapis.com \
  storage.googleapis.com \
  cloudbuild.googleapis.com \
  logging.googleapis.com \
  monitoring.googleapis.com \
  pubsub.googleapis.com \
  billingbudgets.googleapis.com \
  cloudfunctions.googleapis.com \
  iam.googleapis.com
```

### Step 1.3 — Request GPU quota increases (FILE TODAY — 1-3 day approval)

**Critical:** GCP quota approval takes 1-3 business days. File on Day 1 so approval happens in parallel with preflight work.

Navigate to Cloud Console → IAM & Admin → Quotas. Request increases for `us-central1`:

| Quota | Request | Why |
|---|---|---|
| `NVIDIA A100 80GB GPUs` | 8 | Gates 4, 5 need 4-8× A100 80GB |
| `NVIDIA L4 GPUs` | 4 | Gates 1, 2 need 1-2× L4 |
| `NVIDIA RTX PRO 6000 Virtual Workstation GPUs` | 16 (already confirmed available) | Gate 3 |
| `CPUs (G2)` | 48 | For 2× L4 VM instances (g2-standard-24) |
| `CPUs (A2)` | 96 | For 8× A100 VM instances (a2-ultragpu-8g) |
| `CPUs (G4)` | 192 | For G4 Blackwell instances |

Quota request form message: *"Building validation infrastructure for Pantheon, an AI code-generation architecture. Need graduated compute access for tiered testing. Max concurrent spend capped at $50/run, all VMs use --max-run-duration with auto-delete. Budget $100/month total."*

### Step 1.4 — Create service accounts and IAM

```bash
gcloud iam service-accounts create pantheon-validator \
  --display-name="Pantheon Test Validator" \
  --description="Service account for test VMs; provisions, runs, captures, destroys"

export SA_EMAIL="pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com"

# Grant minimum required roles
for role in \
  "roles/compute.instanceAdmin.v1" \
  "roles/storage.objectAdmin" \
  "roles/artifactregistry.reader" \
  "roles/logging.logWriter" \
  "roles/monitoring.metricWriter" \
  "roles/iam.serviceAccountUser"; do
  gcloud projects add-iam-policy-binding $PROJECT_ID \
    --member="serviceAccount:${SA_EMAIL}" \
    --role="$role"
done

# Create a local key for scripts (guard this file, do not commit)
gcloud iam service-accounts keys create ~/.config/gcloud/pantheon-sa-key.json \
  --iam-account=$SA_EMAIL
chmod 600 ~/.config/gcloud/pantheon-sa-key.json
```

### Step 1.5 — Budget alerts and hard kill-switch

```bash
# Create PubSub topic for billing alerts
gcloud pubsub topics create pantheon-billing-alerts

# Create the hard-kill Cloud Function (code in step 1.6)
# Placeholder here; function code is deployed in Phase 4
```

Budget creation (via Cloud Console since `gcloud beta billing budgets create` has limitations on threshold-rule complexity):

1. Console → Billing → Budgets & alerts → CREATE BUDGET
2. Name: `pantheon-validation-v1-budget`
3. Time range: Monthly
4. Amount: `$100`
5. Thresholds:
   - `10%` ($10) → email alert
   - `30%` ($30) → email alert + SMS (if configured)
   - `50%` ($50) → email alert + SMS + PubSub topic `pantheon-billing-alerts`
6. Notifications: route 50% threshold to `pantheon-billing-alerts`

### Step 1.6 — Hard-kill Cloud Function (nuclear backstop)

`functions/hard-kill/main.py`:

```python
# When invoked via PubSub, forcibly deletes ALL Pantheon VMs across all regions
import base64
import json
import subprocess
from google.cloud import compute_v1

PROJECT_ID = "pantheon-validation-v1"
REGIONS_TO_CHECK = [
    "us-central1", "us-east1", "us-east4", "us-east5",
    "us-west1", "us-west2", "us-west3", "us-west4",
    "us-south1", "australia-southeast1", "australia-southeast2"
]

def hard_kill(event, context):
    """Entry point when PubSub message arrives on billing-alerts topic at 50% threshold."""
    if 'data' in event:
        payload = json.loads(base64.b64decode(event['data']).decode('utf-8'))
        cost_amount = payload.get('costAmount', 0)
        budget_amount = payload.get('budgetAmount', 100)

        if cost_amount / budget_amount < 0.5:
            print(f"Cost {cost_amount} below 50% threshold; skipping.")
            return

    client = compute_v1.InstancesClient()

    for region in REGIONS_TO_CHECK:
        for zone_suffix in ['a', 'b', 'c', 'd']:
            zone = f"{region}-{zone_suffix}"
            try:
                instances = client.list(project=PROJECT_ID, zone=zone)
                for inst in instances:
                    print(f"KILLING: {inst.name} in {zone}")
                    client.delete(project=PROJECT_ID, zone=zone, instance=inst.name)
            except Exception as e:
                continue   # Zone may not exist; ignore

    print(f"Hard-kill completed. Budget: ${cost_amount}/${budget_amount}")
```

Deploy:

```bash
cd ~/projects/triumvirate/docs/pantheon/gcp-test-plan/harness/functions/hard-kill

gcloud functions deploy pantheon-hard-kill \
  --gen2 \
  --runtime=python311 \
  --region=$DEFAULT_REGION \
  --source=. \
  --entry-point=hard_kill \
  --trigger-topic=pantheon-billing-alerts \
  --service-account=$SA_EMAIL \
  --memory=512Mi \
  --timeout=540s
```

**Test the kill function** (critical — do not skip):

```bash
# Publish a synthetic 51% threshold event
gcloud pubsub topics publish pantheon-billing-alerts \
  --message='{"costAmount": 51, "budgetAmount": 100}'

# Check function logs — should show "Hard-kill completed."
gcloud functions logs read pantheon-hard-kill --limit=10
```

---

## Phase 2 — Network + storage (1-2 hours, $0)

### Step 2.1 — VPC + subnet + firewall

```bash
# Create dedicated VPC for Pantheon (isolated from default)
gcloud compute networks create pantheon-net \
  --subnet-mode=custom \
  --mtu=1500

# Single subnet in us-central1 (expand to other regions if needed)
gcloud compute networks subnets create pantheon-subnet \
  --network=pantheon-net \
  --region=$DEFAULT_REGION \
  --range=10.128.0.0/20 \
  --enable-private-ip-google-access

# Firewall: allow internal between Pantheon VMs
gcloud compute firewall-rules create pantheon-allow-internal \
  --network=pantheon-net \
  --direction=INGRESS \
  --action=ALLOW \
  --rules=tcp,udp,icmp \
  --source-ranges=10.128.0.0/20

# Firewall: allow SSH from Mike's IP only
export MIKE_IP=$(curl -s ifconfig.me)
gcloud compute firewall-rules create pantheon-allow-ssh-mike \
  --network=pantheon-net \
  --direction=INGRESS \
  --action=ALLOW \
  --rules=tcp:22 \
  --source-ranges=${MIKE_IP}/32

# Firewall: egress allowed to GCS + Artifact Registry + Google APIs only
# (For Gate 6 air-gap sanity, this gets replaced with a zero-egress rule)
# Default egress is allow-all; restrictive egress configured during Gate 6.
```

### Step 2.2 — GCS buckets

```bash
# Model weights cache (~250GB)
gcloud storage buckets create gs://pantheon-models \
  --location=$DEFAULT_REGION \
  --uniform-bucket-level-access \
  --public-access-prevention

# Evidence bundles (grows with every run)
gcloud storage buckets create gs://pantheon-evidence \
  --location=$DEFAULT_REGION \
  --uniform-bucket-level-access \
  --public-access-prevention

# Pythia corpus (SQLite + embeddings, ~5GB)
gcloud storage buckets create gs://pantheon-pythia-corpus \
  --location=$DEFAULT_REGION \
  --uniform-bucket-level-access \
  --public-access-prevention

# Test fixtures (test tasks, scoring rubrics)
gcloud storage buckets create gs://pantheon-fixtures \
  --location=$DEFAULT_REGION \
  --uniform-bucket-level-access \
  --public-access-prevention

# Daemon / container startup scripts
gcloud storage buckets create gs://pantheon-runners \
  --location=$DEFAULT_REGION \
  --uniform-bucket-level-access \
  --public-access-prevention
```

### Step 2.3 — Artifact Registry

```bash
gcloud artifacts repositories create pantheon-images \
  --repository-format=docker \
  --location=$DEFAULT_REGION \
  --description="Pantheon Docker images: vllm, triumvirate, yellingtoad, test-harness"

# Authenticate local Docker to Artifact Registry
gcloud auth configure-docker ${DEFAULT_REGION}-docker.pkg.dev
```

---

## Phase 3 — Docker image pre-bake (3-5 hours, ~$2-5 in Cloud Build)

### Step 3.1 — Base images

Pull upstream images, retag, and push to your internal Artifact Registry for fast same-region pulls:

```bash
export REGISTRY="${DEFAULT_REGION}-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

# vLLM GPU image
docker pull vllm/vllm-openai:v0.6.5
docker tag vllm/vllm-openai:v0.6.5 ${REGISTRY}/pantheon-vllm-gpu:v0.6.5
docker push ${REGISTRY}/pantheon-vllm-gpu:v0.6.5

# vLLM CPU image (for Gate 0/1 plumbing tests without GPU)
docker pull vllm/vllm-openai:v0.6.5-cpu
docker tag vllm/vllm-openai:v0.6.5-cpu ${REGISTRY}/pantheon-vllm-cpu:v0.6.5
docker push ${REGISTRY}/pantheon-vllm-cpu:v0.6.5

# NATS broker
docker pull nats:2.10-alpine
docker tag nats:2.10-alpine ${REGISTRY}/pantheon-nats:2.10
docker push ${REGISTRY}/pantheon-nats:2.10
```

### Step 3.2 — Triumvirate image

Build from source with Cloud Build for reproducibility. `triumvirate/cloudbuild.yaml`:

```yaml
steps:
  - name: 'gcr.io/cloud-builders/docker'
    args:
      - 'build'
      - '-f'
      - 'daemon/Dockerfile'
      - '-t'
      - '${_REGISTRY}/pantheon-triumvirate:${SHORT_SHA}'
      - '-t'
      - '${_REGISTRY}/pantheon-triumvirate:main'
      - '.'
images:
  - '${_REGISTRY}/pantheon-triumvirate:${SHORT_SHA}'
  - '${_REGISTRY}/pantheon-triumvirate:main'
substitutions:
  _REGISTRY: '${DEFAULT_REGION}-docker.pkg.dev/${PROJECT_ID}/pantheon-images'
options:
  logging: CLOUD_LOGGING_ONLY
```

```bash
cd ~/projects/triumvirate
gcloud builds submit --config=cloudbuild.yaml
```

### Step 3.3 — Test harness image

`harness/Dockerfile`:

```dockerfile
FROM python:3.12-slim

RUN apt-get update && apt-get install -y \
    curl git jq \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY harness/ ./harness/
COPY fixtures/ ./fixtures/

ENTRYPOINT ["python3", "-m", "harness.runner"]
```

`harness/requirements.txt`:

```
openai>=1.50.0
httpx>=0.27.0
google-cloud-storage>=2.18.0
google-cloud-logging>=3.11.0
pytest>=8.3.0
pydantic>=2.9.0
tiktoken>=0.8.0
rich>=13.9.0
```

Build:

```bash
cd ~/projects/triumvirate/docs/pantheon/gcp-test-plan
docker build -f harness/Dockerfile -t ${REGISTRY}/pantheon-test-harness:main .
docker push ${REGISTRY}/pantheon-test-harness:main
```

---

## Phase 4 — Model weights cached to GCS (6-10 hours unattended, ~$2-4)

Model downloads happen on a cheap CPU-only VM (not a GPU VM!) to avoid burning GPU dollars on downloads.

### Step 4.1 — Spin up download VM

```bash
gcloud compute instances create pantheon-model-downloader \
  --zone=$DEFAULT_ZONE \
  --machine-type=e2-standard-4 \
  --network=pantheon-net \
  --subnet=pantheon-subnet \
  --image-family=debian-12 \
  --image-project=debian-cloud \
  --boot-disk-size=500GB \
  --boot-disk-type=pd-balanced \
  --service-account=$SA_EMAIL \
  --scopes=cloud-platform \
  --metadata=enable-oslogin=TRUE \
  --max-run-duration=12h \
  --instance-termination-action=DELETE
```

### Step 4.2 — Download models to GCS

SSH in and run:

```bash
gcloud compute ssh pantheon-model-downloader --zone=$DEFAULT_ZONE

# On the VM:
sudo apt-get update && sudo apt-get install -y python3-pip
pip install huggingface_hub 'huggingface_hub[cli]'
huggingface-cli login   # paste your HF token

mkdir -p /tmp/models

# TinyLlama (Gate 0/1 plumbing tests)
huggingface-cli download TinyLlama/TinyLlama-1.1B-Chat-v1.0 \
  --local-dir /tmp/models/tinyllama-1.1b

# Qwen 2.5 Coder 32B AWQ (4-bit)
huggingface-cli download Qwen/Qwen2.5-Coder-32B-Instruct-AWQ \
  --local-dir /tmp/models/qwen2.5-coder-32b-awq

# Qwen 2.5 72B AWQ (Zeus role)
huggingface-cli download Qwen/Qwen2.5-72B-Instruct-AWQ \
  --local-dir /tmp/models/qwen2.5-72b-awq

# DeepSeek-Coder-V2-Lite-16B Instruct AWQ
huggingface-cli download deepseek-ai/DeepSeek-Coder-V2-Lite-Instruct \
  --local-dir /tmp/models/deepseek-coder-v2-lite-16b

# BGE-Large-en-v1.5 (embeddings)
huggingface-cli download BAAI/bge-large-en-v1.5 \
  --local-dir /tmp/models/bge-large-en-v1.5

# Phi-4 14B
huggingface-cli download microsoft/phi-4 \
  --local-dir /tmp/models/phi-4-14b

# Whisper Large v3
huggingface-cli download openai/whisper-large-v3 \
  --local-dir /tmp/models/whisper-large-v3

# Llama 3.1 405B Q4 (for Gate 5 — ~200GB, takes hours)
huggingface-cli download hugging-quants/Meta-Llama-3.1-405B-Instruct-AWQ-INT4 \
  --local-dir /tmp/models/llama-3.1-405b-awq

# Copy everything to GCS
for dir in /tmp/models/*/; do
  model_name=$(basename "$dir")
  gsutil -m cp -r "$dir" "gs://pantheon-models/${model_name}/"
done

# Compute + store checksums
for dir in /tmp/models/*/; do
  model_name=$(basename "$dir")
  (cd "$dir" && find . -type f -name '*.safetensors' -o -name '*.bin' -o -name '*.gguf' | \
    xargs sha256sum > /tmp/${model_name}.sha256)
  gsutil cp /tmp/${model_name}.sha256 gs://pantheon-models/${model_name}/MANIFEST.sha256
done

# Verify sizes in GCS
gsutil du -sh gs://pantheon-models/*
```

### Step 4.3 — Delete the download VM

```bash
gcloud compute instances delete pantheon-model-downloader --zone=$DEFAULT_ZONE --quiet
```

### Step 4.4 — Cost verification

```bash
# Estimated storage cost
echo "GCS standard storage: ~\$0.020/GB/mo"
gsutil du -sh gs://pantheon-models/ | awk '{print "Storage cost: ~$" $1*0.020 "/month"}'
```

Expected: ~$5-8/month for all models in standard storage.

---

## Phase 5 — PD snapshots for fast model mount (1-2 hours, $15/mo ongoing)

Model weights on persistent-disk snapshots mount to a VM in ~20 seconds vs 3-5 minutes via `gsutil cp`. Worth the $15/mo.

### Step 5.1 — Stage models to a disk

```bash
# Create a disk large enough for all models
gcloud compute disks create pantheon-model-staging \
  --zone=$DEFAULT_ZONE \
  --size=500GB \
  --type=pd-ssd

# Spin up temporary VM, attach disk
gcloud compute instances create pantheon-model-stager \
  --zone=$DEFAULT_ZONE \
  --machine-type=e2-standard-4 \
  --disk=name=pantheon-model-staging,mode=rw \
  --service-account=$SA_EMAIL \
  --scopes=cloud-platform \
  --max-run-duration=4h \
  --instance-termination-action=DELETE

gcloud compute ssh pantheon-model-stager --zone=$DEFAULT_ZONE

# On the VM:
sudo mkfs.ext4 /dev/disk/by-id/google-persistent-disk-1
sudo mkdir /mnt/models
sudo mount /dev/disk/by-id/google-persistent-disk-1 /mnt/models
sudo chown -R $USER /mnt/models

# Pull models from GCS to the disk
gsutil -m cp -r gs://pantheon-models/* /mnt/models/
```

### Step 5.2 — Snapshot

```bash
# Log out of VM, then:
gcloud compute disks snapshot pantheon-model-staging \
  --zone=$DEFAULT_ZONE \
  --snapshot-names=pantheon-models-v1 \
  --description="All Pantheon model weights, v1, 2026-04-18"

# Delete the staging VM + disk
gcloud compute instances delete pantheon-model-stager --zone=$DEFAULT_ZONE --quiet
gcloud compute disks delete pantheon-model-staging --zone=$DEFAULT_ZONE --quiet

# Verify snapshot
gcloud compute snapshots list --filter="name:pantheon-models-v1"
```

### Step 5.3 — Usage pattern in future gates

Every gate VM creates a fresh disk from this snapshot at startup:

```bash
gcloud compute instances create my-gate-vm \
  --zone=$DEFAULT_ZONE \
  --machine-type=g2-standard-4 \
  --create-disk=name=models-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  ...
```

The disk is created from snapshot (fast, ~30 sec), attached to the VM, auto-deleted when VM dies.

---

## Phase 6 — Custom VM images (2-3 hours, ~$1-2)

VMs boot faster from custom images that have CUDA + Docker + pre-pulled images already baked in.

### Step 6.1 — Base orchestrator image

```bash
# Spin up a debian-12 VM, install docker + gcloud tooling + pre-pull images
gcloud compute instances create pantheon-baker-orchestrator \
  --zone=$DEFAULT_ZONE \
  --machine-type=e2-standard-2 \
  --image-family=debian-12 \
  --image-project=debian-cloud \
  --service-account=$SA_EMAIL \
  --scopes=cloud-platform \
  --max-run-duration=3h \
  --instance-termination-action=DELETE

gcloud compute ssh pantheon-baker-orchestrator --zone=$DEFAULT_ZONE

# On VM:
sudo apt-get update
sudo apt-get install -y docker.io docker-compose-plugin jq curl git
sudo usermod -aG docker $USER
newgrp docker

# Pre-pull Pantheon images
gcloud auth configure-docker ${DEFAULT_REGION}-docker.pkg.dev
for img in \
  "pantheon-triumvirate:main" \
  "pantheon-test-harness:main" \
  "pantheon-nats:2.10" \
  "pantheon-vllm-cpu:v0.6.5"; do
  docker pull ${REGISTRY}/${img}
done

# Stop SSH, back on laptop: snapshot this VM as a custom image
```

```bash
# On laptop (not VM):
gcloud compute instances stop pantheon-baker-orchestrator --zone=$DEFAULT_ZONE

gcloud compute images create pantheon-orchestrator-v1 \
  --source-disk=pantheon-baker-orchestrator \
  --source-disk-zone=$DEFAULT_ZONE \
  --family=pantheon-orchestrator \
  --description="Debian 12 + Docker + Pantheon images pre-pulled, v1"

gcloud compute instances delete pantheon-baker-orchestrator --zone=$DEFAULT_ZONE --quiet
```

### Step 6.2 — GPU image (L4 + A100 + RTX Pro 6000 compatible)

```bash
# Spin a VM from GCP's Deep Learning image — has CUDA 12.6 + drivers baked
gcloud compute instances create pantheon-baker-gpu \
  --zone=$DEFAULT_ZONE \
  --machine-type=g2-standard-4 \
  --accelerator=type=nvidia-l4,count=1 \
  --provisioning-model=SPOT \
  --image-family=common-cu126 \
  --image-project=deeplearning-platform-release \
  --boot-disk-size=100GB \
  --service-account=$SA_EMAIL \
  --scopes=cloud-platform \
  --max-run-duration=3h \
  --instance-termination-action=DELETE \
  --metadata=install-nvidia-driver=True

# SSH in, pre-pull GPU-specific images
gcloud compute ssh pantheon-baker-gpu --zone=$DEFAULT_ZONE

# On VM:
gcloud auth configure-docker ${DEFAULT_REGION}-docker.pkg.dev
docker pull ${REGISTRY}/pantheon-vllm-gpu:v0.6.5
docker pull ${REGISTRY}/pantheon-triumvirate:main

# Verify CUDA + GPU access
nvidia-smi
docker run --rm --gpus all nvidia/cuda:12.6.0-base-ubuntu22.04 nvidia-smi
```

```bash
# On laptop:
gcloud compute instances stop pantheon-baker-gpu --zone=$DEFAULT_ZONE

gcloud compute images create pantheon-gpu-v1 \
  --source-disk=pantheon-baker-gpu \
  --source-disk-zone=$DEFAULT_ZONE \
  --family=pantheon-gpu \
  --description="Deep Learning VM + Pantheon GPU images pre-pulled, v1"

gcloud compute instances delete pantheon-baker-gpu --zone=$DEFAULT_ZONE --quiet
```

---

## Phase 7 — Fixtures + Pythia seed (1-2 hours, $0)

### Step 7.1 — Export Pythia corpus from Server

```bash
# On Server:
cd ~/projects/triumvirate
sqlite3 data/pythia.db ".backup /tmp/pythia-snapshot-v1.db"
tar czf /tmp/pythia-corpus-v1.tar.gz -C /tmp pythia-snapshot-v1.db

# Upload to GCS
gsutil cp /tmp/pythia-corpus-v1.tar.gz gs://pantheon-pythia-corpus/
```

### Step 7.2 — Test task fixtures

Upload canonical test tasks to `gs://pantheon-fixtures/`:

```bash
# Structure:
#   gs://pantheon-fixtures/
#     test-corpus-triumvirate/       ← 50KLOC Triumvirate repo for embedding tests
#     agent-tasks-rust/              ← 4 canonical Rust code-gen tasks
#     agent-tasks-python/            ← 4 canonical Python tasks
#     agent-tasks-sql/               ← 4 canonical SQL tasks
#     lora-training-corpus-v1/       ← curated fine-tune dataset
#     eval-scorers/                  ← scoring rubrics per task type

cd ~/projects/triumvirate/docs/pantheon/gcp-test-plan/fixtures
gsutil -m cp -r * gs://pantheon-fixtures/
```

---

## Phase 8 — Tooling validation (30 min, ~$0.15)

### Step 8.1 — Dry-run orchestrator-only smoke test

```bash
export RUN_ID="preflight-smoke-$(date +%Y%m%d-%H%M%S)"

gcloud compute instances create pantheon-preflight-smoke-$RUN_ID \
  --zone=$DEFAULT_ZONE \
  --machine-type=e2-standard-4 \
  --image-family=pantheon-orchestrator \
  --image-project=$PROJECT_ID \
  --network=pantheon-net \
  --subnet=pantheon-subnet \
  --service-account=$SA_EMAIL \
  --scopes=cloud-platform \
  --max-run-duration=30m \
  --instance-termination-action=DELETE \
  --metadata=RUN_ID=$RUN_ID,startup-script='#!/bin/bash
    set -e
    docker run --rm \
      -e RUN_ID=$RUN_ID \
      '${REGISTRY}'/pantheon-test-harness:main --mode=smoke-test
    gsutil cp /tmp/smoke-result.json gs://pantheon-evidence/preflight/
    gcloud compute instances delete $(hostname) --zone='$DEFAULT_ZONE' --quiet
  '

# Wait for completion (~5 min)
sleep 300

# Verify result
gsutil ls gs://pantheon-evidence/preflight/
gsutil cat gs://pantheon-evidence/preflight/smoke-result.json
```

### Step 8.2 — Verify hard-kill function

```bash
# Publish a synthetic 60% threshold to trigger hard-kill
gcloud pubsub topics publish pantheon-billing-alerts \
  --message='{"costAmount": 60, "budgetAmount": 100}'

# Check that any live VMs get killed (should be zero since smoke-test cleaned up)
sleep 30
gcloud compute instances list --format="table(name,zone,status)"
# Expected: empty list
```

---

## Preflight completion checklist

Before proceeding to Gate 0, verify ALL of these:

- [ ] GCP project created, billing linked, Gemini Ultra credit active
- [ ] All required APIs enabled
- [ ] GPU quota increases requested (wait for approval email)
- [ ] `pantheon-validator` service account created with minimum IAM
- [ ] Budget alert configured at `$10 / $30 / $50` thresholds
- [ ] PubSub topic `pantheon-billing-alerts` created
- [ ] Cloud Function `pantheon-hard-kill` deployed AND TESTED (nuclear backstop verified)
- [ ] VPC `pantheon-net` + subnet `pantheon-subnet` created
- [ ] Firewall rules for internal + SSH from Mike's IP created
- [ ] GCS buckets: pantheon-models, pantheon-evidence, pantheon-pythia-corpus, pantheon-fixtures, pantheon-runners
- [ ] Artifact Registry `pantheon-images` created
- [ ] Docker images built + pushed: pantheon-vllm-gpu, pantheon-vllm-cpu, pantheon-triumvirate, pantheon-test-harness, pantheon-nats
- [ ] Model weights cached to `gs://pantheon-models/` for all 8 models
- [ ] Model checksums stored in `MANIFEST.sha256` per model
- [ ] PD snapshot `pantheon-models-v1` created
- [ ] Custom VM images created: `pantheon-orchestrator-v1`, `pantheon-gpu-v1`
- [ ] Pythia corpus backup uploaded to `gs://pantheon-pythia-corpus/`
- [ ] Test fixtures uploaded to `gs://pantheon-fixtures/`
- [ ] Preflight smoke test passed (evidence bundle landed in GCS)
- [ ] Hard-kill function verified (synthetic PubSub test triggered deletion behavior)

**When every box is checked, proceed to `runbooks/gate-0-plumbing.md`.**

---

## Cost accounting for preflight

| Phase | Spend |
|---|---|
| Phase 1-2 (accounts, network, storage) | $0 |
| Phase 3 (Cloud Build) | $2-5 |
| Phase 4 (model downloads on e2-standard-4) | $2-4 |
| Phase 5 (PD snapshot staging) | $1-2 |
| Phase 6 (custom VM image baking) | $1-2 |
| Phase 7 (fixture upload) | $0 |
| Phase 8 (smoke test) | ~$0.15 |
| **Total one-time** | **$6-13** |
| **Ongoing storage (monthly)** | **$15-20** |

All within Gemini Ultra GCP credit. Effective cost to Mike: $0.

---

## What comes next

With preflight complete, proceed to:

**`runbooks/gate-0-plumbing.md`** — first test run, CPU-only, $0.50 spend, ~45 min. Validates Docker Compose + NATS + Triumvirate daemon + mock vLLM work together end-to-end. Zero GPU risk. Evidence bundle lands, Obsidian note auto-generates, your first Pantheon run is immortalized.
