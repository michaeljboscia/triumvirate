# Gate 0 — Plumbing (CPU-only Orchestration Sanity)

**Purpose:** First execution after preflight. Validates that Docker Compose + NATS + Triumvirate daemon + a mock vLLM endpoint work together end-to-end. Zero GPU risk. Zero inference validation — this gate is pure orchestration plumbing.

**GCP config:** `e2-standard-4` (4 vCPU, 16GB RAM, no GPU)
**Cost:** ~$0.13/hr Spot × 45 min = **~$0.10 per session**
**Duration:** 45 min hard cap
**Pre-committed decision rule:** see `30-DECISION-RULES.md` → Decision 4

---

## Why this gate exists

Every expensive GPU gate that follows assumes the orchestration layer works. If Triumvirate can't dispatch to a mock vLLM, it can't dispatch to a real one — and finding that out on a $12/hr A100 is a waste. This gate debugs orchestration at $0.13/hr.

**Specifically validates:**
- Docker Compose starts NATS + Triumvirate + mock vLLM without errors
- Triumvirate successfully reads config, registers endpoints, starts HTTP server
- NATS JetStream accepts task dispatches
- Mock vLLM responds to OpenAI-compat requests
- Triumvirate's OpenAI-compat HTTP client parses responses correctly
- Evidence bundle format lands in GCS

## What it does NOT test

- Real model inference
- Real worker pool behavior
- Real protocol quirks (those are Gates 1+)
- Real GPU scheduling

---

## Hypotheses being tested

### H-0.1 — Orchestration layer starts cleanly

**Prediction:** `docker compose up -d` brings up NATS + Triumvirate + mock vLLM in <60 sec. All three report healthy on their status endpoints.

**Decision rule:** All three healthy within 60 sec → PASS. Anything fails → debug before Gate 1.

### H-0.2 — End-to-end task dispatch works

**Prediction:** Test harness sends 5 canned tasks via Triumvirate's HTTP API. All 5 are dispatched to mock vLLM, responses flow back, harness receives structured results.

**Decision rule:** 5/5 tasks complete round-trip → PASS. Any hang or error → debug.

### H-0.3 — Evidence bundle emission works

**Prediction:** Runner script produces standard evidence bundle at `gs://pantheon-evidence/gate-0/{run_id}/` with manifest, logs, metrics, summary.

**Decision rule:** Bundle structure matches `20-EVIDENCE-BUNDLE-SPEC.md` → PASS.

---

## Pre-run checklist

- [ ] Preflight complete per `10-PREFLIGHT.md`
- [ ] `pantheon-orchestrator-v1` custom VM image exists
- [ ] `pantheon-triumvirate:main`, `pantheon-test-harness:main`, `pantheon-nats:2.10`, `pantheon-vllm-cpu:v0.6.5` images pushed
- [ ] No other Pantheon VMs live
- [ ] GCS evidence bucket writable

---

## Runbook

### Step 1 — Provision CPU-only VM (2-3 min)

```bash
export PROJECT_ID="pantheon-validation-v1"
export ZONE="us-central1-a"
export RUN_ID="gate0-plumbing-$(date +%Y%m%d-%H%M%S)"
export REGISTRY="us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

# Pre-flight inventory check
RUNNING=$(gcloud compute instances list --filter="status=RUNNING" --format="value(name)")
[ -n "$RUNNING" ] && { echo "ABORT: $RUNNING still running"; exit 1; }

gcloud compute instances create pantheon-$RUN_ID \
  --zone=$ZONE \
  --project=$PROJECT_ID \
  --machine-type=e2-standard-4 \
  --provisioning-model=SPOT \
  --instance-termination-action=DELETE \
  --max-run-duration=45m \
  --network=pantheon-net \
  --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-orchestrator \
  --image-project=$PROJECT_ID \
  --boot-disk-size=50GB \
  --metadata=RUN_ID=$RUN_ID,GATE=0 \
  --no-address
```

### Step 2 — SSH and verify environment

```bash
until gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="docker ps" 2>/dev/null; do
  sleep 10
done

gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE

# On VM:
docker ps
gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
mkdir -p /tmp/evidence/$RUN_ID
```

### Step 3 — Launch docker-compose stack

Pull the compose file shipped with the Triumvirate image:

```yaml
# docker-compose.gate-0.yml
version: '3.8'
services:
  nats:
    image: us-central1-docker.pkg.dev/pantheon-validation-v1/pantheon-images/pantheon-nats:2.10
    command: ["-js", "-m", "8222"]
    ports: ["4222:4222", "8222:8222"]
    healthcheck:
      test: ["CMD", "wget", "-qO-", "http://localhost:8222/healthz"]
      interval: 5s
      timeout: 3s
      retries: 5

  mock-vllm:
    image: us-central1-docker.pkg.dev/pantheon-validation-v1/pantheon-images/pantheon-test-harness:main
    command: ["--mode=mock-vllm-server"]
    ports: ["8000:8000"]
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:8000/v1/models"]
      interval: 5s
      timeout: 3s
      retries: 5

  triumvirate:
    image: us-central1-docker.pkg.dev/pantheon-validation-v1/pantheon-images/pantheon-triumvirate:main
    environment:
      RUST_LOG: info
      TRIUMVIRATE_CONFIG: /etc/triumvirate/gate-0.toml
    volumes:
      - ./config:/etc/triumvirate:ro
    ports: ["7788:7788"]
    depends_on:
      nats: {condition: service_healthy}
      mock-vllm: {condition: service_healthy}
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:7788/status"]
      interval: 5s
      timeout: 3s
      retries: 10
```

Triumvirate config (`config/gate-0.toml`):

```toml
[nats]
url = "nats://nats:4222"

[inference.endpoints.mock]
url = "http://mock-vllm:8000/v1"
model = "mock-model-v1"

[dispatch.rules]
default_endpoint = "mock"
```

Launch:

```bash
cat > /tmp/docker-compose.gate-0.yml <<'EOF'
# [paste content from above]
EOF

mkdir -p /tmp/config
cat > /tmp/config/gate-0.toml <<'EOF'
# [paste content from above]
EOF

cd /tmp
docker compose -f docker-compose.gate-0.yml up -d

# Wait for all three healthy
sleep 15
docker compose -f docker-compose.gate-0.yml ps
```

### Step 4 — H-0.1 test: health check all three

```bash
# NATS
curl -sf http://localhost:8222/healthz && echo "NATS: OK"

# Mock vLLM
curl -sf http://localhost:8000/v1/models | jq . && echo "Mock vLLM: OK"

# Triumvirate
curl -sf http://localhost:7788/status | jq . && echo "Triumvirate: OK"

# Record into evidence
{
  echo "H-0.1: orchestration layer health check"
  echo "Timestamp: $(date -Iseconds)"
  echo "NATS: $(curl -sf http://localhost:8222/healthz && echo OK || echo FAIL)"
  echo "Mock vLLM: $(curl -sf http://localhost:8000/v1/models >/dev/null && echo OK || echo FAIL)"
  echo "Triumvirate: $(curl -sf http://localhost:7788/status >/dev/null && echo OK || echo FAIL)"
} > /tmp/evidence/$RUN_ID/h-0.1.txt
```

### Step 5 — H-0.2 test: end-to-end task dispatch

```bash
# Fire 5 canned tasks via Triumvirate HTTP API
docker run --rm \
  --network host \
  -e RUN_ID=$RUN_ID \
  -e GATE=0 \
  -e TEST=h-0.2-dispatch \
  $REGISTRY/pantheon-test-harness:main \
  --mode=task-dispatch-smoke \
  --triumvirate-url=http://localhost:7788 \
  --num-tasks=5 \
  --output-dir=/tmp/evidence/$RUN_ID/h-0.2

cat /tmp/evidence/$RUN_ID/h-0.2/metrics.json
```

**Expected output:**

```json
{
  "test_id": "h-0.2-dispatch",
  "run_id": "gate0-plumbing-...",
  "tasks_dispatched": 5,
  "tasks_completed": 5,
  "tasks_errored": 0,
  "round_trip_median_ms": 45,
  "verdict": "PASS"
}
```

### Step 6 — H-0.3 test: evidence bundle emission

```bash
# Generate manifest + summary via shared harness
python3 /opt/pantheon-harness/generate-manifest.py \
  --run-id=$RUN_ID --gate=0 \
  --output=/tmp/evidence/$RUN_ID/manifest.json

python3 /opt/pantheon-harness/generate-summary.py \
  --run-id=$RUN_ID --gate=0 \
  --evidence-dir=/tmp/evidence/$RUN_ID \
  --output=/tmp/evidence/$RUN_ID/summary.md

# Upload to GCS
gsutil -m cp -r /tmp/evidence/$RUN_ID gs://pantheon-evidence/gate-0/

# Verify structure matches spec
gsutil ls -r gs://pantheon-evidence/gate-0/$RUN_ID/
```

### Step 7 — Capture logs and self-destruct

```bash
docker compose -f /tmp/docker-compose.gate-0.yml logs > /tmp/evidence/$RUN_ID/docker-compose.log 2>&1
gsutil cp /tmp/evidence/$RUN_ID/docker-compose.log gs://pantheon-evidence/gate-0/$RUN_ID/

# Generate cost report
python3 /opt/pantheon-harness/cost-tracker.py \
  --run-id=$RUN_ID --output=/tmp/evidence/$RUN_ID/cost-report.json
gsutil cp /tmp/evidence/$RUN_ID/cost-report.json gs://pantheon-evidence/gate-0/$RUN_ID/

# Auto-generate Obsidian note
python3 /opt/pantheon-harness/generate-obsidian-note.py \
  --run-id=$RUN_ID --gate=0 \
  --summary=/tmp/evidence/$RUN_ID/summary.md \
  --output=/tmp/evidence/$RUN_ID/obsidian-note.md
gsutil cp /tmp/evidence/$RUN_ID/obsidian-note.md gs://pantheon-evidence/gate-0/$RUN_ID/

# Self-destruct
exit
gcloud compute instances delete pantheon-$RUN_ID --zone=$ZONE --quiet
```

### Step 8 — Verify + drop note into vault

```bash
gsutil ls -r gs://pantheon-evidence/gate-0/$RUN_ID/
gsutil cp gs://pantheon-evidence/gate-0/$RUN_ID/obsidian-note.md \
  ~/Documents/pantheon-vault/runs/$RUN_ID.md
cd ~/Documents/pantheon-vault && git add runs/$RUN_ID.md && git commit -m "gate-0 run $RUN_ID"
```

---

## Cost accounting

| Line item | Cost |
|---|---|
| e2-standard-4 Spot, 45 min max | ~$0.10 |
| GCS evidence write | negligible |
| **Total per session** | **~$0.10** |

---

## What comes after

Gate 0 PASS → Gate 1 (single L4 baseline) — first real GPU burn, validates inference plumbing. Runbook: `runbooks/gate-1-single-l4.md`.

Gate 0 FAIL → fix the broken component BEFORE touching GPU hardware. Cheapest debug available.
