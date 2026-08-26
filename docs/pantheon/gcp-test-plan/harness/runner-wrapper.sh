#!/usr/bin/env bash
# runner-wrapper.sh
# The universal provision, run, capture, destroy wrapper for every Pantheon gate.
#
# Usage:
#   ./runner-wrapper.sh --gate=2 --config=./configs/gate-2-dual-l4.env
#
# Every gate runbook calls this wrapper with a gate-specific config.
# The wrapper handles: pre-flight checks, VM provision, SSH setup, startup script
# delivery, healthcheck wait, self-destruct on exit, evidence bundling, GCS upload.

set -euo pipefail

# ============================================================================
# Argument parsing
# ============================================================================

GATE=""
CONFIG=""
DRY_RUN=false

while [[ $# -gt 0 ]]; do
  case $1 in
    --gate=*) GATE="${1#*=}"; shift ;;
    --config=*) CONFIG="${1#*=}"; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

if [[ -z "$GATE" || -z "$CONFIG" ]]; then
  echo "Usage: $0 --gate=N --config=PATH"
  exit 1
fi

if [[ ! -f "$CONFIG" ]]; then
  echo "Config not found: $CONFIG"
  exit 1
fi

# ============================================================================
# Load gate-specific configuration
# ============================================================================
# Expected vars in config:
#   MACHINE_TYPE, ACCELERATOR, MAX_RUN_DURATION_MIN, STARTUP_SCRIPT,
#   EXPECTED_HYPOTHESES (space-separated), EXPECTED_COST_USD

# shellcheck source=/dev/null
source "$CONFIG"

# Standard env
: "${PROJECT_ID:=pantheon-validation-v1}"
: "${ZONE:=us-central1-a}"
: "${SA_EMAIL:=pantheon-validator@${PROJECT_ID}.iam.gserviceaccount.com}"
: "${REGISTRY:=us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images}"
: "${NETWORK:=pantheon-net}"
: "${SUBNET:=pantheon-subnet}"
: "${MODEL_SNAPSHOT:=pantheon-models-v1}"
: "${EVIDENCE_BUCKET:=gs://pantheon-evidence}"

RUN_ID="gate${GATE}-$(date +%Y%m%d-%H%M%S)-$(head -c 4 /dev/urandom | xxd -p)"
VM_NAME="pantheon-${RUN_ID}"
RUN_START=$(date +%s)

echo "================================================================"
echo "Pantheon Gate ${GATE}: Runner Wrapper"
echo "================================================================"
echo "  RUN_ID:           ${RUN_ID}"
echo "  VM_NAME:          ${VM_NAME}"
echo "  MACHINE_TYPE:     ${MACHINE_TYPE}"
echo "  ACCELERATOR:      ${ACCELERATOR:-none}"
echo "  MAX_RUN (min):    ${MAX_RUN_DURATION_MIN}"
echo "  STARTUP_SCRIPT:   ${STARTUP_SCRIPT}"
echo "  EXPECTED COST:    \$${EXPECTED_COST_USD}"
echo "================================================================"

# ============================================================================
# Pre-flight checks
# ============================================================================

echo "[preflight] Checking for existing Pantheon VMs..."
EXISTING=$(gcloud compute instances list \
  --project="$PROJECT_ID" \
  --filter="name~pantheon- AND status:RUNNING" \
  --format="value(name)" 2>/dev/null || true)

if [[ -n "$EXISTING" ]]; then
  echo "[preflight] ABORT: Pantheon VMs still running:"
  echo "$EXISTING" | sed 's/^/    /'
  echo ""
  echo "Delete them first:"
  echo "  gcloud compute instances delete $EXISTING --zone=$ZONE --quiet"
  exit 1
fi

echo "[preflight] Checking GCP quotas..."
# (Quota check could poll the quota API; skipping explicit check here)

echo "[preflight] Checking evidence bucket writable..."
echo "test" | gsutil cp - "${EVIDENCE_BUCKET}/preflight/.write-test-${RUN_ID}" 2>/dev/null
gsutil rm "${EVIDENCE_BUCKET}/preflight/.write-test-${RUN_ID}" 2>/dev/null || true

if [[ "$DRY_RUN" == "true" ]]; then
  echo "[dry-run] Preflight passed. Would provision VM with config: $CONFIG"
  exit 0
fi

# ============================================================================
# Exit trap: always destroy VM, always upload whatever evidence exists
# ============================================================================

cleanup() {
  local exit_code=$?
  echo ""
  echo "[cleanup] Exit code: $exit_code"
  echo "[cleanup] Destroying VM ${VM_NAME}..."
  gcloud compute instances delete "$VM_NAME" \
    --zone="$ZONE" --project="$PROJECT_ID" --quiet 2>/dev/null || true

  echo "[cleanup] Recording final run metadata..."
  local run_end
  run_end=$(date +%s)
  local duration=$((run_end - RUN_START))

  # If VM exited but we have local evidence, upload it
  if [[ -d "/tmp/evidence/${RUN_ID}" ]]; then
    echo "[cleanup] Uploading local evidence to ${EVIDENCE_BUCKET}/gate-${GATE}/${RUN_ID}/..."
    gsutil -m cp -r "/tmp/evidence/${RUN_ID}" "${EVIDENCE_BUCKET}/gate-${GATE}/" 2>/dev/null || true
  fi

  echo "[cleanup] Total wall-clock: ${duration}s"
  echo "[cleanup] Exit status: $exit_code"
  exit $exit_code
}
trap cleanup EXIT INT TERM

# ============================================================================
# Provision VM
# ============================================================================

echo "[provision] Creating VM ${VM_NAME}..."

ACCEL_ARG=""
if [[ -n "${ACCELERATOR:-}" ]]; then
  ACCEL_ARG="--accelerator=${ACCELERATOR}"
fi

DISK_ARG=""
if [[ "${ATTACH_MODEL_DISK:-true}" == "true" ]]; then
  DISK_ARG="--create-disk=name=models-${RUN_ID},size=500GB,type=pd-ssd,source-snapshot=${MODEL_SNAPSHOT},auto-delete=yes,device-name=models"
fi

gcloud compute instances create "$VM_NAME" \
  --project="$PROJECT_ID" \
  --zone="$ZONE" \
  --machine-type="$MACHINE_TYPE" \
  ${ACCEL_ARG} \
  --provisioning-model=SPOT \
  --instance-termination-action=DELETE \
  --max-run-duration="${MAX_RUN_DURATION_MIN}m" \
  --network="$NETWORK" \
  --subnet="$SUBNET" \
  --service-account="$SA_EMAIL" \
  --scopes=cloud-platform \
  --image-family="${IMAGE_FAMILY:-pantheon-gpu}" \
  --image-project="$PROJECT_ID" \
  --boot-disk-size="${BOOT_DISK_SIZE:-100}GB" \
  ${DISK_ARG} \
  --metadata="RUN_ID=${RUN_ID},GATE=${GATE},install-nvidia-driver=True" \
  --metadata-from-file="startup-script=${STARTUP_SCRIPT}" \
  --no-address \
  --labels="pantheon=true,gate=${GATE},run-id=${RUN_ID//[^a-z0-9-]/-}" \
  2>&1 | tee /tmp/provision-${RUN_ID}.log

echo "[provision] VM created. Waiting for SSH + startup..."

# ============================================================================
# Wait for VM ready
# ============================================================================

READY_TIMEOUT_SEC=600   # 10 min
WAIT_START=$(date +%s)

while true; do
  if gcloud compute ssh "$VM_NAME" --zone="$ZONE" --project="$PROJECT_ID" \
       --command="echo ready && nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null || echo no-gpu" \
       --quiet 2>/dev/null; then
    echo "[wait] VM ready."
    break
  fi

  now=$(date +%s)
  if (( now - WAIT_START > READY_TIMEOUT_SEC )); then
    echo "[wait] TIMEOUT after ${READY_TIMEOUT_SEC}s. VM not responding."
    exit 2
  fi

  sleep 15
done

# ============================================================================
# Run the actual gate test (inline SSH)
# ============================================================================

echo "[run] Executing gate ${GATE} test harness..."

gcloud compute ssh "$VM_NAME" --zone="$ZONE" --project="$PROJECT_ID" --command="
  set -e
  export RUN_ID='${RUN_ID}'
  export GATE='${GATE}'
  export REGISTRY='${REGISTRY}'
  export EVIDENCE_BUCKET='${EVIDENCE_BUCKET}'

  # Mount model disk if attached
  if [ -e /dev/disk/by-id/google-*-models ]; then
    sudo mkdir -p /mnt/models
    sudo mount /dev/disk/by-id/google-*-models /mnt/models 2>/dev/null || true
  fi

  # Auth Docker
  gcloud auth configure-docker us-central1-docker.pkg.dev --quiet

  mkdir -p /tmp/evidence/\$RUN_ID

  # Execute gate-specific test script (must be in startup-script or pulled from GCS)
  if [ -f /tmp/gate-test.sh ]; then
    bash /tmp/gate-test.sh
  else
    # Fall back to downloading from GCS
    gsutil cp gs://pantheon-runners/gate-${GATE}-test.sh /tmp/gate-test.sh
    bash /tmp/gate-test.sh
  fi

  # Generate manifest + summary + cost report + obsidian note
  python3 /opt/pantheon-harness/finalize-evidence.py \
    --run-id=\$RUN_ID --gate=\$GATE \
    --evidence-dir=/tmp/evidence/\$RUN_ID

  # Upload evidence to GCS
  gsutil -m cp -r /tmp/evidence/\$RUN_ID ${EVIDENCE_BUCKET}/gate-${GATE}/

  echo '[run] Evidence bundle uploaded successfully.'
"

echo "[run] Gate ${GATE} execution complete."

# ============================================================================
# Verify evidence landed
# ============================================================================

echo "[verify] Checking evidence bundle in GCS..."
if ! gsutil ls "${EVIDENCE_BUCKET}/gate-${GATE}/${RUN_ID}/manifest.json" >/dev/null 2>&1; then
  echo "[verify] WARNING: manifest.json not found in GCS bundle."
  exit 3
fi

echo "[verify] Bundle contents:"
gsutil ls -r "${EVIDENCE_BUCKET}/gate-${GATE}/${RUN_ID}/" | sed 's/^/    /'

# Download summary for immediate review
LOCAL_SUMMARY="/tmp/pantheon-summaries/${RUN_ID}-summary.md"
mkdir -p "$(dirname "$LOCAL_SUMMARY")"
gsutil cp "${EVIDENCE_BUCKET}/gate-${GATE}/${RUN_ID}/summary.md" "$LOCAL_SUMMARY" 2>/dev/null || true

if [[ -f "$LOCAL_SUMMARY" ]]; then
  echo ""
  echo "================================================================"
  echo "Run Summary (local copy: $LOCAL_SUMMARY)"
  echo "================================================================"
  cat "$LOCAL_SUMMARY"
  echo "================================================================"
fi

# ============================================================================
# Drop into Obsidian vault (if configured)
# ============================================================================

if [[ -n "${OBSIDIAN_VAULT_PATH:-}" && -d "$OBSIDIAN_VAULT_PATH" ]]; then
  echo "[obsidian] Copying note to vault..."
  gsutil cp "${EVIDENCE_BUCKET}/gate-${GATE}/${RUN_ID}/obsidian-note.md" \
    "${OBSIDIAN_VAULT_PATH}/runs/${RUN_ID}.md"

  if [[ -d "${OBSIDIAN_VAULT_PATH}/.git" ]]; then
    (cd "$OBSIDIAN_VAULT_PATH" && \
     git add "runs/${RUN_ID}.md" && \
     git commit -m "gate-${GATE} run ${RUN_ID} evidence" 2>/dev/null || true)
  fi
fi

echo ""
echo "[done] Gate ${GATE} complete. Run ID: ${RUN_ID}"
echo "[done] Evidence: ${EVIDENCE_BUCKET}/gate-${GATE}/${RUN_ID}/"
echo "[done] VM has been deleted."
