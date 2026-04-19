#!/usr/bin/env bash
# request-quotas.sh
# Fire all Pantheon GPU quota increase requests in one shot via `gcloud alpha quotas preferences create`.
#
# Usage:
#   ./request-quotas.sh              # Dry-run: print every command that WOULD execute
#   ./request-quotas.sh --confirm    # Actually submit the requests
#   ./request-quotas.sh --confirm --scope=critical   # Only the 3 most important
#   ./request-quotas.sh --confirm --scope=all        # Everything (default)

set -uo pipefail
# Intentionally NOT using `set -e` — a single failed quota submission (e.g. duplicate)
# should not kill the loop. Each submission's success/failure is tracked explicitly.

PROJECT_ID="${PROJECT_ID:-aerial-jigsaw-467620-m8}"
CONTACT_EMAIL="${CONTACT_EMAIL:-michaeljboscia@gmail.com}"
SERVICE="compute.googleapis.com"

CONFIRM=false
SCOPE="all"

for arg in "$@"; do
  case $arg in
    --confirm) CONFIRM=true ;;
    --scope=*) SCOPE="${arg#*=}" ;;
    --project=*) PROJECT_ID="${arg#*=}" ;;
    --email=*) CONTACT_EMAIL="${arg#*=}" ;;
  esac
done

JUSTIFICATION="Building Pantheon, an AI code-generation infrastructure validation platform. Graduated GPU testing across L4, A100 80GB, H100, H200, B200, RTX Pro 6000 hardware tiers. All workloads run with --provisioning-model=SPOT, --max-run-duration=120m, and --instance-termination-action=DELETE as hard budget controls. Monthly spend budget-capped at \$100 with automated billing alerts + Cloud Function kill-switch at 50% threshold. Individual sessions 1-2 hours max. No production serving; AI inference + fine-tuning experimentation only. Multi-region allocations requested for Spot pool resilience."

# ============================================================================
# Quota request matrix
# Format: quota-id|region|preferred-value|priority
# Priority: critical | high | medium | low
# ============================================================================

QUOTAS=(
  # Note: quota IDs use UPPERCASE-DASHED-METRIC-per-project-region casing
  # per the Cloud Quotas API. Verify with:
  #   curl -H "Authorization: Bearer $(gcloud auth print-access-token)" \
  #     "https://cloudquotas.googleapis.com/v1/projects/<PROJECT>/locations/global/services/compute.googleapis.com/quotaInfos?pageSize=500"

  # ──── CRITICAL (blocks test plan execution) ────
  "PREEMPTIBLE-NVIDIA-A100-80GB-GPUS-per-project-region|us-central1|32|critical"
  "PREEMPTIBLE-NVIDIA-L4-GPUS-per-project-region|us-central1|16|critical"

  # ──── HIGH (enables aggressive gate execution) ────
  "PREEMPTIBLE-NVIDIA-A100-80GB-GPUS-per-project-region|us-east4|16|high"
  "PREEMPTIBLE-NVIDIA-L4-GPUS-per-project-region|us-east4|8|high"
  "PREEMPTIBLE-NVIDIA-H100-GPUS-per-project-region|us-central1|8|high"
  "PREEMPTIBLE-NVIDIA-H100-MEGA-GPUS-per-project-region|us-central1|16|high"

  # ──── MEDIUM (future-proofing for next-gen hardware) ────
  "PREEMPTIBLE-NVIDIA-H200-GPUS-per-project-region|us-central1|8|medium"
  "PREEMPTIBLE-NVIDIA-B200-GPUS-per-project-region|us-central1|4|medium"
  "PREEMPTIBLE-NVIDIA-H100-GPUS-per-project-region|us-east4|4|medium"
  "PREEMPTIBLE-NVIDIA-H200-GPUS-per-project-region|us-east4|4|medium"

  # ──── LOW (belt-and-suspenders tertiary region) ────
  "PREEMPTIBLE-NVIDIA-A100-80GB-GPUS-per-project-region|us-west1|8|low"
  "PREEMPTIBLE-NVIDIA-L4-GPUS-per-project-region|us-west1|4|low"
)

# Filter by scope
filter_by_scope() {
  local filter=$1
  for quota in "${QUOTAS[@]}"; do
    IFS='|' read -r qid region value priority <<< "$quota"
    case "$filter" in
      critical)
        [[ "$priority" == "critical" ]] && echo "$quota"
        ;;
      high)
        [[ "$priority" =~ ^(critical|high)$ ]] && echo "$quota"
        ;;
      all)
        echo "$quota"
        ;;
    esac
  done
}

# ============================================================================
# Execute
# ============================================================================

echo "================================================================"
echo "Pantheon GCP Quota Request Batch"
echo "================================================================"
echo "  Project:        $PROJECT_ID"
echo "  Contact:        $CONTACT_EMAIL"
echo "  Scope:          $SCOPE"
echo "  Confirmed:      $CONFIRM"
echo "================================================================"
echo ""

COUNTER=0
FILTERED=$(filter_by_scope "$SCOPE")

if [[ -z "$FILTERED" ]]; then
  echo "No quotas match scope '$SCOPE'. Valid scopes: critical, high, all"
  exit 1
fi

while IFS= read -r quota || [[ -n "$quota" ]]; do
  [[ -z "$quota" ]] && continue
  IFS='|' read -r qid region value priority <<< "$quota" || true
  COUNTER=$((COUNTER + 1))

  # Generate a unique preference ID — required by API
  pref_id="pantheon-$(date +%Y%m%d)-${region}-${qid//preemptible-nvidia-/}-${value}"
  pref_id="${pref_id//-per-project-region/}"
  # Trim to 63 char max, lowercase, only [a-z0-9-]
  pref_id=$(echo "$pref_id" | tr '[:upper:]' '[:lower:]' | cut -c1-63)

  printf "%2d. [%-8s] %-55s %s → %d\n" "$COUNTER" "$priority" "$qid" "$region" "$value"

  CMD=(
    gcloud alpha quotas preferences create
    --service="$SERVICE"
    --quota-id="$qid"
    --preferred-value="$value"
    --dimensions="region=$region"
    --email="$CONTACT_EMAIL"
    --justification="$JUSTIFICATION"
    --preference-id="$pref_id"
    --project="$PROJECT_ID"
  )

  if [[ "$CONFIRM" == "true" ]]; then
    echo "    submitting..."
    if "${CMD[@]}" 2>&1 | sed 's/^/      /'; then
      echo "    ✅ submitted"
    else
      echo "    ⚠️  failed (may be duplicate / already requested)"
    fi
    echo ""
  else
    echo "    [dry-run] Would submit:"
    printf "      %s \\\\\n" "${CMD[0]}"
    for flag in "${CMD[@]:1}"; do
      printf "        %s \\\\\n" "$flag"
    done
    echo ""
  fi

  # Brief pause between API calls to avoid rate-limiting
  if [[ "$CONFIRM" == "true" ]]; then sleep 2; fi
done <<< "$FILTERED"

echo "================================================================"
echo "Done. $COUNTER quota preferences processed."
echo ""

if [[ "$CONFIRM" != "true" ]]; then
  echo "This was a DRY RUN. To actually submit:"
  echo "  ./request-quotas.sh --confirm"
  echo ""
  echo "To submit only the critical subset first:"
  echo "  ./request-quotas.sh --confirm --scope=critical"
  echo ""
else
  echo "Check status anytime with:"
  echo "  gcloud alpha quotas preferences list --project=$PROJECT_ID --service=$SERVICE"
  echo ""
  echo "Approval timelines vary:"
  echo "  - Simple bumps (L4 1→16, A100 Spot 0→32): 1-3 business days"
  echo "  - New SKU access (H100, H200, B200 where current is 0): 3-7 days"
  echo "  - You'll receive email notifications as each is approved"
fi
echo "================================================================"
