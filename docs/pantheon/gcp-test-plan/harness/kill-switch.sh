#!/usr/bin/env bash
# kill-switch.sh
# Emergency all-region VM teardown for Pantheon.
#
# Use when:
#   - A gate run hangs and the trap-based cleanup didn't fire
#   - Budget alert triggers and you need to stop everything immediately
#   - You're leaving for the night and want to guarantee nothing is billing
#   - Paranoia check before sleep / travel
#
# Usage:
#   ./kill-switch.sh              # Dry-run: list what WOULD be deleted
#   ./kill-switch.sh --confirm    # Actually delete everything matching Pantheon filters
#   ./kill-switch.sh --nuclear    # Delete ALL VMs in project (use with caution)
#   ./kill-switch.sh --scope=run-id=gate2-...  # Only kill VMs for specific run

set -euo pipefail

PROJECT_ID="${PROJECT_ID:-pantheon-validation-v1}"
REGIONS=(
  "us-central1" "us-east1" "us-east4" "us-east5"
  "us-west1" "us-west2" "us-west3" "us-west4"
  "us-south1"
  "australia-southeast1" "australia-southeast2"
)

CONFIRM=false
NUCLEAR=false
SCOPE_FILTER=""

while [[ $# -gt 0 ]]; do
  case $1 in
    --confirm) CONFIRM=true; shift ;;
    --nuclear) NUCLEAR=true; CONFIRM=true; shift ;;
    --scope=*) SCOPE_FILTER="${1#*=}"; shift ;;
    --project=*) PROJECT_ID="${1#*=}"; shift ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

echo "================================================================"
echo "Pantheon Kill-Switch"
echo "================================================================"
echo "  Project:         $PROJECT_ID"
echo "  Scope:           ${SCOPE_FILTER:-all Pantheon VMs}"
echo "  Mode:            $([ "$NUCLEAR" = "true" ] && echo 'NUCLEAR (all VMs)' || echo 'Pantheon VMs only')"
echo "  Confirmed:       $CONFIRM"
echo "================================================================"

# ============================================================================
# Build the filter
# ============================================================================

if [[ "$NUCLEAR" == "true" ]]; then
  FILTER=""   # everything in project
elif [[ -n "$SCOPE_FILTER" ]]; then
  FILTER="labels.${SCOPE_FILTER}"
else
  FILTER="labels.pantheon=true OR name~^pantheon-"
fi

# ============================================================================
# Enumerate instances across all regions
# ============================================================================

ALL_INSTANCES=()

for region in "${REGIONS[@]}"; do
  # Each region may have multiple zones; list by region to cover all zones
  if [[ -n "$FILTER" ]]; then
    instances=$(gcloud compute instances list \
      --project="$PROJECT_ID" \
      --filter="$FILTER AND zone~${region}" \
      --format="csv[no-heading](name,zone,status,creationTimestamp,machineType.basename())" \
      2>/dev/null || true)
  else
    instances=$(gcloud compute instances list \
      --project="$PROJECT_ID" \
      --filter="zone~${region}" \
      --format="csv[no-heading](name,zone,status,creationTimestamp,machineType.basename())" \
      2>/dev/null || true)
  fi

  if [[ -n "$instances" ]]; then
    while IFS=, read -r name zone status created machine; do
      ALL_INSTANCES+=("$name,$zone,$status,$created,$machine")
    done <<< "$instances"
  fi
done

if [[ ${#ALL_INSTANCES[@]} -eq 0 ]]; then
  echo "No matching VMs found. Nothing to kill."
  exit 0
fi

# ============================================================================
# Display what would be deleted
# ============================================================================

echo ""
echo "Instances matching filter:"
printf "  %-40s %-20s %-10s %-25s %-20s\n" "NAME" "ZONE" "STATUS" "CREATED" "MACHINE"
echo "  -----------------------------------------------------------------------------------------------------------"
for instance in "${ALL_INSTANCES[@]}"; do
  IFS=, read -r name zone status created machine <<< "$instance"
  printf "  %-40s %-20s %-10s %-25s %-20s\n" "$name" "$zone" "$status" "$created" "$machine"
done

# ============================================================================
# Estimate dollars being burned
# ============================================================================

RUNNING_COUNT=0
for instance in "${ALL_INSTANCES[@]}"; do
  IFS=, read -r name zone status created machine <<< "$instance"
  if [[ "$status" == "RUNNING" ]]; then
    ((RUNNING_COUNT++))
  fi
done

if [[ $RUNNING_COUNT -gt 0 ]]; then
  echo ""
  echo "  ⚠️  $RUNNING_COUNT instance(s) are currently RUNNING and billing."
fi

# ============================================================================
# Confirmation gate
# ============================================================================

if [[ "$CONFIRM" != "true" ]]; then
  echo ""
  echo "DRY RUN — no VMs deleted."
  echo ""
  echo "To actually delete these, re-run with --confirm"
  if [[ $RUNNING_COUNT -gt 0 ]]; then
    echo ""
    echo "  Suggested: ./kill-switch.sh --confirm"
  fi
  exit 0
fi

# ============================================================================
# Delete all matching VMs in parallel
# ============================================================================

echo ""
echo "DELETING ${#ALL_INSTANCES[@]} VMs in parallel..."

DELETE_PIDS=()

for instance in "${ALL_INSTANCES[@]}"; do
  IFS=, read -r name zone _ _ _ <<< "$instance"
  (
    echo "  deleting: $name (zone=$zone)"
    gcloud compute instances delete "$name" \
      --project="$PROJECT_ID" \
      --zone="$zone" \
      --quiet 2>&1 | sed "s/^/    [$name] /"
  ) &
  DELETE_PIDS+=($!)
done

# Wait for all deletes to complete
for pid in "${DELETE_PIDS[@]}"; do
  wait $pid || true
done

# ============================================================================
# Final verification
# ============================================================================

echo ""
echo "Verifying kill-switch result..."

VERIFY=$(gcloud compute instances list \
  --project="$PROJECT_ID" \
  --filter="${FILTER:-}" \
  --format="value(name)" 2>/dev/null || true)

if [[ -z "$VERIFY" ]]; then
  echo "✅ All targeted VMs deleted."
else
  echo "⚠️  Some VMs still present:"
  echo "$VERIFY" | sed 's/^/    /'
  echo ""
  echo "These may be in STOPPING state. Re-run in 30 sec to re-check."
fi

# ============================================================================
# Orphan disk check (cost leak)
# ============================================================================

echo ""
echo "Checking for orphan disks (VM deleted but disk remained)..."

ORPHAN_DISKS=$(gcloud compute disks list \
  --project="$PROJECT_ID" \
  --filter="NOT users:*" \
  --format="csv[no-heading](name,zone,sizeGb,type)" \
  2>/dev/null || true)

if [[ -n "$ORPHAN_DISKS" ]]; then
  echo "  ⚠️  Orphan disks found (NOT deleted automatically):"
  echo "$ORPHAN_DISKS" | sed 's/^/    /'
  echo ""
  echo "To delete orphan disks manually:"
  echo "  gcloud compute disks list --filter='NOT users:*' --format='value(name,zone)' | xargs -n2 gcloud compute disks delete --quiet --zone"
else
  echo "  No orphan disks found."
fi

echo ""
echo "Kill-switch complete."
