#!/bin/bash
# ============================================================================
# TOKEN GATE HOOK — PostToolUse auto-save at configurable token thresholds
#
# Fires after every tool call. Estimates tokens consumed by watching the
# transcript JSONL file grow, then auto-saves session notes when the
# threshold is crossed.
#
# Token estimation: transcript file size / 4 ≈ tokens (4 bytes/token average)
# So 80KB growth ≈ 20K new tokens consumed since last save.
#
# Configuration (env vars, set before starting Claude Code):
#   TOKEN_GATE_THRESHOLD_KB=200   → save after this many KB of transcript growth
#                                    (200KB ≈ 50K tokens, 400KB ≈ 100K tokens)
#   TOKEN_GATE_COOLDOWN_SECS=300  → minimum seconds between saves (default: 5m)
#   TOKEN_GATE_CHECK_EVERY_N=15   → check transcript size every N tool calls
#                                    (reduces stat() overhead on every call)
#   TOKEN_GATE_DISABLE=1          → disable this hook entirely
#
# State file: ~/.claude/token-gate-state.json
# Bypass:     touch ~/.claude/token-gate-bypass (disables for the session)
#
# macOS compatible: stat -f %z, stat -f %m, no mapfile, bash 3.2+
# ============================================================================

set -uo pipefail

# ─── Early exits ─────────────────────────────────────────────────────────────
[[ "${TOKEN_GATE_DISABLE:-0}" == "1" ]] && exit 0
[[ -f "$HOME/.claude/token-gate-bypass" ]] && exit 0

INPUT="$(cat)" || INPUT=""
CWD="$(printf '%s' "$INPUT" | jq -r '.cwd // empty' 2>/dev/null)" || CWD=""

# ─── Config ──────────────────────────────────────────────────────────────────
THRESHOLD_KB="${TOKEN_GATE_THRESHOLD_KB:-200}"
COOLDOWN_SECS="${TOKEN_GATE_COOLDOWN_SECS:-300}"
CHECK_EVERY_N="${TOKEN_GATE_CHECK_EVERY_N:-15}"

# Guard against zero/non-numeric values that would cause division by zero or silent NaN.
# bash arithmetic with 0 in modulo (CALL_COUNT % 0) is a fatal error.
[[ "$THRESHOLD_KB"  =~ ^[1-9][0-9]*$ ]] || THRESHOLD_KB=200
[[ "$CHECK_EVERY_N" =~ ^[1-9][0-9]*$ ]] || CHECK_EVERY_N=15
[[ "$COOLDOWN_SECS" =~ ^[0-9]+$       ]] || COOLDOWN_SECS=300

THRESHOLD_BYTES="$(( THRESHOLD_KB * 1024 ))"

STATE_FILE="$HOME/.claude/token-gate-state.json"
LOG_FILE="$HOME/.claude/artifact-guard-logs/token-gate.log"
PROJECTS_DIR="$HOME/.claude/projects/$(printf '%s' "$HOME" | sed 's|/|-|g' | sed 's/^-//')"

mkdir -p "$(dirname "$LOG_FILE")" 2>/dev/null

# ─── State management ────────────────────────────────────────────────────────
_read_state() {
  if [[ -f "$STATE_FILE" ]]; then
    jq -r "${1} // ${2}" "$STATE_FILE" 2>/dev/null || printf '%s' "$2"
  else
    printf '%s' "$2"
  fi
}

_write_state() {
  # Atomic write via temp file
  local tmp
  tmp="$(mktemp "${STATE_FILE}.XXXXXX")" || return 1
  printf '%s' "$1" > "$tmp" && mv "$tmp" "$STATE_FILE"
}

# Increment tool call counter
CALL_COUNT="$(_read_state '.call_count' '0')"
CALL_COUNT=$(( CALL_COUNT + 1 ))

LAST_SAVE_BYTES="$(_read_state '.last_save_bytes' '0')"
LAST_SAVE_TIME="$(_read_state '.last_save_time' '0')"
SAVES_THIS_SESSION="$(_read_state '.saves_this_session' '0')"

# Write updated count (always — cheap)
_write_state "$(jq -cn \
  --argjson count "$CALL_COUNT" \
  --argjson lsb "$LAST_SAVE_BYTES" \
  --argjson lst "$LAST_SAVE_TIME" \
  --argjson sts "$SAVES_THIS_SESSION" \
  '{call_count:$count, last_save_bytes:$lsb, last_save_time:$lst, saves_this_session:$sts}')" \
  2>/dev/null

# Only check transcript size every N calls (reduces I/O on every tool call)
SHOULD_CHECK=$(( CALL_COUNT % CHECK_EVERY_N ))
[[ "$SHOULD_CHECK" -ne 0 ]] && exit 0

# ─── Cooldown check ──────────────────────────────────────────────────────────
NOW="$(date +%s)"
ELAPSED="$(( NOW - LAST_SAVE_TIME ))"
[[ "$ELAPSED" -lt "$COOLDOWN_SECS" ]] && exit 0

# ─── Find current transcript ─────────────────────────────────────────────────
# The active transcript is the most recently modified JSONL in the projects dir.
TRANSCRIPT="$(ls -t "$PROJECTS_DIR"/*.jsonl 2>/dev/null | head -1)"
if [[ -z "$TRANSCRIPT" || ! -f "$TRANSCRIPT" ]]; then
  exit 0
fi

CURRENT_BYTES="$(stat -f %z "$TRANSCRIPT" 2>/dev/null)" || exit 0

# ─── Threshold check ─────────────────────────────────────────────────────────
GROWTH="$(( CURRENT_BYTES - LAST_SAVE_BYTES ))"

if [[ "$GROWTH" -lt "$THRESHOLD_BYTES" ]]; then
  exit 0
fi

# ─── Threshold crossed — save! ────────────────────────────────────────────────
TOKENS_APPROX="$(( CURRENT_BYTES / 4 / 1000 ))"
GROWTH_TOKENS_APPROX="$(( GROWTH / 4 / 1000 ))"
SAVES_THIS_SESSION="$(( SAVES_THIS_SESSION + 1 ))"

printf '%s TOKEN_GATE save=%d growth_bytes=%d growth_ktokens≈%d total_bytes=%d transcript=%s\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "$SAVES_THIS_SESSION" "$GROWTH" "$GROWTH_TOKENS_APPROX" "$CURRENT_BYTES" \
  "$(basename "$TRANSCRIPT")" \
  >> "$LOG_FILE" 2>/dev/null

# Update state BEFORE launching background save (prevent double-trigger)
_write_state "$(jq -cn \
  --argjson count "$CALL_COUNT" \
  --argjson lsb "$CURRENT_BYTES" \
  --argjson lst "$NOW" \
  --argjson sts "$SAVES_THIS_SESSION" \
  '{call_count:$count, last_save_bytes:$lsb, last_save_time:$lst, saves_this_session:$sts}')" \
  2>/dev/null

# ─── Background save ─────────────────────────────────────────────────────────
# Reuses pre-compact.sh with synthesized PreCompact event JSON.
# Runs fully async — doesn't block the tool call that triggered this.
PRE_COMPACT_HOOK="$HOME/.claude/hooks/pre-compact.sh"
SAVE_CWD="${CWD:-$HOME}"

if [[ -x "$PRE_COMPACT_HOOK" ]]; then
  (
    # Use jq to build JSON so paths with quotes/backslashes/spaces are safe
    jq -cn --arg cwd "$SAVE_CWD" --arg tp "$TRANSCRIPT" \
      '{cwd:$cwd, transcript_path:$tp, trigger:"token-gate"}' \
      | bash "$PRE_COMPACT_HOOK" \
      >> "$HOME/.claude/artifact-guard-logs/token-gate-save.log" 2>&1
  ) &
  disown $! 2>/dev/null
fi

# ─── Notify Claude via additionalContext ─────────────────────────────────────
jq -cn \
  --arg saves "$SAVES_THIS_SESSION" \
  --arg growth "${GROWTH_TOKENS_APPROX}k" \
  --arg total "${TOKENS_APPROX}k" \
  --arg threshold "${THRESHOLD_KB}KB" \
  '{
    hookSpecificOutput: {
      hookEventName: "PostToolUse",
      additionalContext: (
        "💾 TOKEN GATE (save #" + $saves + "): ~" + $growth + " new tokens since last save " +
        "(~" + $total + " total this session). " +
        "Threshold: " + $threshold + " transcript growth. " +
        "Auto-save running in background via pre-compact.sh → Gemini summary → session log → git commit. " +
        "You do NOT need to stop — this is non-blocking. " +
        "If you want a manual full save right now, ask Claude to update the session log."
      )
    }
  }'

exit 0
