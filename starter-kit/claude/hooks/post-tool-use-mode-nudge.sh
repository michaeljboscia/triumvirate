#!/bin/bash
# ============================================================================
# MODE NUDGE — PostToolUse Soft Trigger (Execution Modes)
#
# Purpose: After 15+ tool calls or 20+ minutes without a mode set, inject a
#          soft suggestion to formalize as EXPLORE. Fires once per session.
#
# Performance: Lightweight — reads/writes a small JSON file in /tmp.
#
# Created: 2026-03-19
# Source: PRD-2026-001 / FEAT-002
# ============================================================================

set -uo pipefail

# ---------------------------------------------------------------------------
# FIND PROJECT ROOT
# ---------------------------------------------------------------------------
INPUT="$(cat)" || INPUT=""
CWD="$(echo "$INPUT" | jq -r '.cwd // empty' 2>/dev/null)" || CWD=""
[[ -z "$CWD" ]] && CWD="$(pwd)"

_find_project_root() {
  local dir="$1"
  while [[ "$dir" != "/" && "$dir" != "$HOME" ]]; do
    [[ -d "$dir/.git" ]] && { echo "$dir"; return 0; }
    [[ -f "$dir/.claude/execution-mode.json" ]] && { echo "$dir"; return 0; }
    dir="$(dirname "$dir")"
  done
  [[ -d "$HOME/.git" ]] && { echo "$HOME"; return 0; }
  echo ""
  return 1
}

PROJECT_ROOT="$(_find_project_root "$CWD")" || PROJECT_ROOT=""
[[ -z "$PROJECT_ROOT" ]] && exit 0

# ---------------------------------------------------------------------------
# CHECK: mode already set?
# ---------------------------------------------------------------------------
MODE_FILE="$PROJECT_ROOT/.claude/execution-mode.json"
[[ -f "$MODE_FILE" ]] && exit 0

# ---------------------------------------------------------------------------
# SESSION METRICS (ephemeral, per project)
# Key by project root hash. In production, Claude Code's PPID would also
# work, but hashing the project root is more robust across test harnesses.
# Counter resets on reboot (/tmp) or when mode is set (early exit above).
# ---------------------------------------------------------------------------
METRICS_DIR="/tmp/claude-scope-guard"
mkdir -p "$METRICS_DIR" 2>/dev/null
_proj_hash="$(printf '%s' "$PROJECT_ROOT" | shasum | awk '{print $1}')"
METRICS_FILE="$METRICS_DIR/${_proj_hash}.json"

# Read or initialize metrics
if [[ -f "$METRICS_FILE" ]]; then
  TOOL_COUNT="$(jq -r '.tool_call_count // 0' "$METRICS_FILE" 2>/dev/null)" || TOOL_COUNT=0
  SESSION_START="$(jq -r '.session_start // empty' "$METRICS_FILE" 2>/dev/null)" || SESSION_START=""
  SOFT_FIRED="$(jq -r '.soft_trigger_fired // false' "$METRICS_FILE" 2>/dev/null)" || SOFT_FIRED="false"
else
  TOOL_COUNT=0
  SESSION_START="$(date +%s)"
  SOFT_FIRED="false"
fi

# Increment counter
TOOL_COUNT=$((TOOL_COUNT + 1))

# Write updated metrics
jq -n \
  --argjson pid "$PPID" \
  --argjson count "$TOOL_COUNT" \
  --arg start "$SESSION_START" \
  --argjson fired "$SOFT_FIRED" \
  '{pid:$pid, tool_call_count:$count, session_start:$start, soft_trigger_fired:$fired}' \
  > "$METRICS_FILE.tmp" 2>/dev/null && mv "$METRICS_FILE.tmp" "$METRICS_FILE" 2>/dev/null

# Already fired this session
[[ "$SOFT_FIRED" == "true" ]] && exit 0

# ---------------------------------------------------------------------------
# CHECK THRESHOLDS
# ---------------------------------------------------------------------------
NOW="$(date +%s)"
ELAPSED=$((NOW - ${SESSION_START:-$NOW}))
THRESHOLD_CALLS=15
THRESHOLD_SECS=1200  # 20 minutes

if [[ "$TOOL_COUNT" -ge "$THRESHOLD_CALLS" || "$ELAPSED" -ge "$THRESHOLD_SECS" ]]; then
  # Fire soft suggestion — once
  jq -n \
    --argjson pid "$PPID" \
    --argjson count "$TOOL_COUNT" \
    --arg start "$SESSION_START" \
    --argjson fired true \
    '{pid:$pid, tool_call_count:$count, session_start:$start, soft_trigger_fired:$fired}' \
    > "$METRICS_FILE.tmp" 2>/dev/null && mv "$METRICS_FILE.tmp" "$METRICS_FILE" 2>/dev/null

  _MSG=$'💡 You\'ve been digging into this for a while. Want to formalize as EXPLORE?\n   - Set an appetite (how long is this worth?)\n   - Output goes to research/ so it doesn\'t get lost\n\nOr keep chatting — no pressure.\n\nTo activate:\n  python3 ~/.claude/hooks/activate-mode.py --mode explore --flavor project --topic "<topic>" --appetite "1 sitting" --project-root '"$PROJECT_ROOT"

  jq -n --arg c "$_MSG" \
    '{hookSpecificOutput:{hookEventName:"PostToolUse",additionalContext:$c}}'
  exit 0
fi

exit 0
