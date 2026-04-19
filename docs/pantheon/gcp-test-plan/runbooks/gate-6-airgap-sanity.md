# Gate 6 — Air-Gap Sovereign Sanity

**Purpose:** Prove that Pantheon's "100% sovereign / air-gap capable" claim is an actual empirical property, not marketing copy. A deliberately firewall-locked VM runs the full Pantheon stack with ZERO outbound traffic. Any packet leaving the VM = fail. Required before any customer sovereign demo.

**GCP config:** `g4-standard-32` (1× RTX Pro 6000) OR `a2-ultragpu-4g` (4× A100) — reusable from prior gates
**Cost:** ~$3-10/hr × 1 hr = **~$3-10 per session**
**Duration:** 1 hour hard cap
**Pre-committed decision rule:** see `30-DECISION-RULES.md` → Decision 7 (shipping gate)

---

## Why this gate matters

The sovereign product tier depends on ONE testable claim: when deployed in an air-gapped environment, Pantheon does not attempt any outbound network connection. Customers in regulated industries (banking, legal, medical, government) will audit this empirically. Being wrong = instantly disqualified from the customer.

**This gate simulates the audit before the audit happens.**

---

## What it validates

- Pantheon components make ZERO outbound HTTP/HTTPS/DNS requests during operation
- Model weights loaded entirely from local disk (no HuggingFace Hub calls)
- Docker containers started from local cache (no Artifact Registry pulls)
- Triumvirate daemon doesn't phone home
- vLLM doesn't query any telemetry endpoint
- Aider/Goose (Layer 4 clients) don't call any cloud API
- If Pythia retrieval is local, no external lookup
- Evidence-bundle upload still works (to local-to-VPC bucket only)

## What it does NOT validate

- Production-representative Mac Studio Metal air-gap (Metal has its own behaviors; test on actual hardware later)
- Long-running resilience against accidental config drift (Gate 7 soak covers that)
- Third-party integrations that explicitly need cloud (they're excluded by product tier)

---

## Hypotheses being tested

### H-6.1 — Zero outbound traffic during model boot + inference

**Prediction:** After firewall lockdown applied, VM logs zero outbound connection attempts that reach the firewall. `tcpdump` on VM shows no DNS queries, no HTTPS to external IPs.

**Decision rule:**
- Zero outbound attempts detected → PASS. Sovereign claim holds.
- 1-10 attempts → investigate specific source, patch component, re-test.
- > 10 attempts → fundamental architectural leak, sovereign claim cannot ship.

### H-6.2 — Full agent task completes air-gapped

**Prediction:** 4-task canonical swarm completes end-to-end with firewall locked. All tasks pass eval. No stalls on timeout waiting for external resource.

**Decision rule:**
- 4/4 tasks complete successfully → PASS.
- Any task stalls / times out → identify the blocked call, remove the dependency, re-test.

### H-6.3 — Evidence bundle still emits within VPC

**Prediction:** GCS upload to same-region bucket succeeds (via Private Google Access). Evidence bundle lands normally.

**Decision rule:**
- Bundle lands → PASS.
- Upload fails → confirm Private Google Access + VPC routing; fix before customer deployment.

---

## Pre-run checklist

- [ ] Preflight complete, Gates 3 or 4 passed recently
- [ ] Custom firewall rule defined: zero egress except to Google APIs private endpoints
- [ ] Private Google Access enabled on `pantheon-subnet` (already in preflight)
- [ ] Model weights + PD snapshot ready (loaded into local disk, no network needed)
- [ ] Pre-baked Docker images in local image cache on custom VM image

---

## Runbook

### Step 1 — Provision VM + apply zero-egress firewall

```bash
export PROJECT_ID="pantheon-validation-v1"
export ZONE="us-central1-a"
export RUN_ID="gate6-airgap-$(date +%Y%m%d-%H%M%S)"
export REGISTRY="us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

RUNNING=$(gcloud compute instances list --filter="status=RUNNING" --format="value(name)")
[ -n "$RUNNING" ] && { echo "ABORT"; exit 1; }

# Provision normally — we'll apply firewall DURING the test, after stack is up
gcloud compute instances create pantheon-$RUN_ID \
  --zone=$ZONE --machine-type=g4-standard-32 \
  --accelerator=type=nvidia-rtx-pro-6000,count=1 \
  --provisioning-model=SPOT --instance-termination-action=DELETE \
  --max-run-duration=60m \
  --network=pantheon-net --subnet=pantheon-subnet \
  --service-account=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com \
  --scopes=cloud-platform \
  --image-family=pantheon-gpu --image-project=$PROJECT_ID \
  --boot-disk-size=100GB \
  --create-disk=name=models-$RUN_ID,size=500GB,type=pd-ssd,source-snapshot=pantheon-models-v1,auto-delete=yes,device-name=models \
  --metadata=RUN_ID=$RUN_ID,GATE=6,install-nvidia-driver=True \
  --tags=pantheon-airgap-test \
  --no-address
```

### Step 2 — Prepare stack BEFORE applying lockdown

```bash
until gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="nvidia-smi" 2>/dev/null; do sleep 20; done

gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="
  sudo mount /dev/disk/by-id/google-*-models /mnt/models
  gcloud auth configure-docker us-central1-docker.pkg.dev --quiet

  # Pre-pull any images we'll need — this MUST happen before lockdown
  docker pull $REGISTRY/pantheon-vllm-gpu:v0.6.5
  docker pull $REGISTRY/pantheon-triumvirate:main
  docker pull $REGISTRY/pantheon-test-harness:main

  # Pre-download task fixtures to VM disk (no network calls during test)
  gsutil -m cp -r gs://pantheon-fixtures/agent-tasks-canonical /tmp/tasks
  gsutil cp gs://pantheon-pythia-corpus/pythia-corpus-v1.tar.gz /tmp/
  tar xzf /tmp/pythia-corpus-v1.tar.gz -C /tmp/

  # Start tcpdump monitor to catch ALL outbound traffic
  mkdir -p /tmp/evidence/$RUN_ID
  sudo tcpdump -i any -w /tmp/evidence/$RUN_ID/airgap-traffic.pcap \
    'not (dst net 10.128.0.0/20 or dst net 127.0.0.0/8)' &
  echo \$! > /tmp/tcpdump.pid

  echo 'Stack prepared. tcpdump started. Ready for lockdown.'
"
```

### Step 3 — Apply zero-egress firewall rule

```bash
# On laptop — create deny-all-egress rule scoped to this VM
gcloud compute firewall-rules create pantheon-airgap-deny-egress-$RUN_ID \
  --network=pantheon-net \
  --direction=EGRESS \
  --action=DENY \
  --rules=all \
  --destination-ranges=0.0.0.0/0 \
  --target-tags=pantheon-airgap-test \
  --priority=100

# Allow ONLY Private Google Access for eventual evidence bundle upload
gcloud compute firewall-rules create pantheon-airgap-allow-pga-$RUN_ID \
  --network=pantheon-net \
  --direction=EGRESS \
  --action=ALLOW \
  --rules=tcp:443 \
  --destination-ranges=199.36.153.8/30,199.36.153.4/30 \
  --target-tags=pantheon-airgap-test \
  --priority=50

# Verify rules applied
gcloud compute firewall-rules list --filter="name~pantheon-airgap"
```

### Step 4 — H-6.1 test: start full stack under lockdown

```bash
gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="
  # Start vLLM (image already cached, model already on local disk)
  docker run -d --name vllm \
    --gpus all -v /mnt/models/qwen2.5-72b-awq:/model:ro \
    -p 8000:8000 --shm-size=16g \
    $REGISTRY/pantheon-vllm-gpu:v0.6.5 \
    --model /model --served-model-name qwen-72b \
    --tensor-parallel-size 1 --gpu-memory-utilization 0.90 \
    --max-model-len 8192 --max-num-seqs 8 \
    --quantization awq_marlin --dtype float16 \
    --disable-log-stats \
    --disable-custom-all-reduce

  # Wait for vLLM — should work entirely from local
  until curl -sf http://localhost:8000/v1/models; do sleep 20; done

  # Start Triumvirate (image cached, Pythia local)
  docker run -d --name triumvirate --network host \
    -v /tmp/pythia-snapshot-v1.db:/var/pythia.db:ro \
    -e PYTHIA_DB=/var/pythia.db \
    $REGISTRY/pantheon-triumvirate:main

  until curl -sf http://localhost:7788/status; do sleep 10; done

  echo 'Stack running under air-gap lockdown.'
"
```

### Step 5 — H-6.2 test: run full agent swarm air-gapped

```bash
gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="
  docker run --rm --network host \
    -v /tmp/tasks:/tasks:ro \
    -e TEST=h-6.2-airgap-swarm -e RUN_ID=$RUN_ID \
    $REGISTRY/pantheon-test-harness:main \
    --mode=full-agent-swarm \
    --triumvirate-url=http://localhost:7788 \
    --tasks=/tasks --num-parallel=4 \
    --timeout-per-task=300 \
    --eval-rubric=/tasks/eval-rubric.yaml \
    --output-dir=/tmp/evidence/$RUN_ID/h-6.2

  cat /tmp/evidence/$RUN_ID/h-6.2/swarm-summary.json
"
```

### Step 6 — Stop tcpdump, analyze captured traffic (H-6.1)

```bash
gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="
  sudo kill \$(cat /tmp/tcpdump.pid) || true

  # Convert pcap to readable summary
  sudo tcpdump -r /tmp/evidence/$RUN_ID/airgap-traffic.pcap -nn -t > /tmp/evidence/$RUN_ID/airgap-traffic.txt

  # Count outbound attempts
  OUTBOUND_COUNT=\$(wc -l < /tmp/evidence/$RUN_ID/airgap-traffic.txt)
  echo \"Outbound packet attempts captured: \$OUTBOUND_COUNT\"

  # Any traffic outside the allowed PGA endpoints is a LEAK
  cat > /tmp/evidence/$RUN_ID/airgap-verdict.json <<EOF
{
  \"test_id\": \"h-6.1-zero-egress\",
  \"outbound_packets_total\": \$OUTBOUND_COUNT,
  \"pga_destinations_allowed\": [\"199.36.153.8/30\", \"199.36.153.4/30\"],
  \"verdict\": \"\$([ \$OUTBOUND_COUNT -lt 5 ] && echo PASS || echo FAIL)\"
}
EOF

  cat /tmp/evidence/$RUN_ID/airgap-verdict.json
"
```

### Step 7 — H-6.3 test: evidence upload works through PGA only

```bash
gcloud compute ssh pantheon-$RUN_ID --zone=$ZONE --command="
  cat > /tmp/evidence/$RUN_ID/manifest.json <<EOF
{
  \"run_id\": \"$RUN_ID\",
  \"gate\": 6,
  \"gcp_machine_type\": \"g4-standard-32\",
  \"test\": \"air-gap-sovereign-validation\",
  \"hypotheses_tested\": [\"H-6.1\", \"H-6.2\", \"H-6.3\"]
}
EOF

  python3 /opt/pantheon-harness/generate-summary.py --run-id=$RUN_ID --gate=6 \
    --evidence-dir=/tmp/evidence/$RUN_ID --output=/tmp/evidence/$RUN_ID/summary.md

  # This gsutil call MUST succeed through Private Google Access only
  gsutil -m cp -r /tmp/evidence/$RUN_ID gs://pantheon-evidence/gate-6/
  echo \"Upload result: \$?\"
"
```

### Step 8 — Teardown firewall + self-destruct

```bash
# Clean up firewall rules — important, do NOT leave deny rules in place
gcloud compute firewall-rules delete pantheon-airgap-deny-egress-$RUN_ID --quiet
gcloud compute firewall-rules delete pantheon-airgap-allow-pga-$RUN_ID --quiet

gcloud compute instances delete pantheon-$RUN_ID --zone=$ZONE --quiet
```

---

## Decision rule application

### PASS (all three):
- H-6.1: outbound packets ≤ 5 (allowing for incidental retry noise)
- H-6.2: 4/4 canonical tasks complete successfully
- H-6.3: evidence bundle lands in GCS via PGA

→ Sovereign claim validated. Ready to ship Pantheon Sovereign / Vault tier.

### FAIL on H-6.1 (outbound leak):
- Identify source via pcap analysis — common culprits:
  - vLLM telemetry (disable with `--disable-log-stats`)
  - Hugging Face Hub lookups (set `HF_HUB_OFFLINE=1` and `TRANSFORMERS_OFFLINE=1`)
  - NTP sync (leave enabled if allowed; document the PGA exception)
  - Container runtime pulling unlinked layers (pre-pull EVERYTHING in bake step)
- Patch the component, rebuild image, re-test at Gate 6.

### FAIL on H-6.2 (task stalls):
- Workers timing out waiting for external resources — same root cause investigation as H-6.1.

### FAIL on H-6.3 (evidence upload broken):
- Confirm Private Google Access is enabled on subnet + PGA endpoints whitelisted in firewall.

---

## Cost accounting

| Line item | Cost |
|---|---|
| g4-standard-32 Spot, 1 hr | ~$3-4 |
| PD snapshot (transient) | negligible |
| **Total per session** | **~$3-5** |

Re-run Gate 6 before every customer sovereign demo. It's cheap and the cost of a failed demo is massive.

---

## What comes after

Gate 6 PASS = Pantheon Sovereign is demo-ready and audit-defensible.

Proceed to Gate 7 (soak/stress) for long-session stability validation. Runbook: `runbooks/gate-7-soak-stress.md`.
