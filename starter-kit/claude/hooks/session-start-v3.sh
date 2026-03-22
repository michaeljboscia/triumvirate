#!/bin/bash
# ============================================================================
# SESSION-START v3.1 — Orphan recovery via session-save-ctl.py
#
# Fires on SessionStart. Cleans up orphaned saves and stale locks.
# Temp files scoped to transcript UUID (no cross-session interference).
# ============================================================================

set -uo pipefail
INPUT="$(cat)" || INPUT=""

V3_DIR="$HOME/.triumvirate/stenographer"
STATE_FILE="$HOME/.triumvirate/session-state.json"

# Only proceed if v3 state file exists
[[ ! -f "$STATE_FILE" ]] && exit 0

# Run orphan recovery
python3 "$V3_DIR/session-save-ctl.py" recover 2>/dev/null

# Get transcript UUID for scoped temp files
TUUID=$(jq -r '.transcript.path // empty' "$STATE_FILE" 2>/dev/null \
    | xargs basename 2>/dev/null | sed 's/\.jsonl$//')

# Reset scoped temp files for fresh session
printf '0' > "/tmp/claude-tg-counter-${TUUID:-global}" 2>/dev/null
: > "/tmp/claude-file-write-batch-${TUUID:-global}" 2>/dev/null
rm -f "/tmp/claude-silence-breached-${TUUID:-global}" 2>/dev/null

# Silent — no additionalContext
exit 0
