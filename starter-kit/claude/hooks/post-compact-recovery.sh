#!/bin/bash
# PSEUDO-POSTCOMPACT: Fires ONLY after compaction via SessionStart matcher: "compact"
# The heavy lifting (Gemini summarization OR jq fallback) already happened in PreCompact.
# This hook reads the pre-computed summary/narrative and injects it.
# Works with both section headings written by pre-compact.sh:
#   "## 🧠 GEMINI CONTEXT SUMMARY" (Gemini succeeded)
#   "## 📋 SESSION NARRATIVE (jq fallback — Gemini unavailable)" (Gemini failed)

INPUT=$(cat)
PROJECT_DIR=$(echo "$INPUT" | jq -r '.cwd // empty')
TIMESTAMP=$(TZ='America/New_York' date '+%Y-%m-%d %H:%M:%S %Z')

LESSONS_MAX_CHARS=6000
ADDED_LESSON_PATHS="|"

# Source shared session log finder
source "$HOME/.claude/hooks/_find-session-log.sh" 2>/dev/null

# Find the most recent session log (contains the Gemini summary from PreCompact)
SESSION_LOG=""
if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then
  _find_session_log "$PROJECT_DIR"
fi

# Extract the session summary from the session log.
# pre-compact.sh writes one of two section headings:
#   "## 🧠 GEMINI CONTEXT SUMMARY"  — Gemini succeeded (preferred)
#   "## 📋 SESSION NARRATIVE"        — jq fallback
# Try Gemini section first; fall back to jq narrative section.
# Cap at 80KB (~20K tokens) — take the TAIL (most recent = most useful)
GEMINI_SUMMARY=""
if [ -n "$SESSION_LOG" ] && [ -f "$SESSION_LOG" ]; then
  # Inject the full session log — Gemini summary, Stenographer notes, manually
  # written SESSION UPDATE sections, all of it. Skip boilerplate header sections
  # (TAXONOMY, TRANSCRIPT HISTORY, CONTEXT COMPACTION, ACTIVITY LOG, INSTRUCTIONS)
  # since those are metadata, not working context.
  RAW_NARRATIVE=$(awk '
    /^## (TAXONOMY|TRANSCRIPT HISTORY|CONTEXT COMPACTION OCCURRED|SESSION ACTIVITY LOG|INSTRUCTIONS FOR NEXT SESSION)/{skip=1}
    /^## / && !/^## (TAXONOMY|TRANSCRIPT HISTORY|CONTEXT COMPACTION OCCURRED|SESSION ACTIVITY LOG|INSTRUCTIONS FOR NEXT SESSION)/{skip=0}
    !skip{print}
  ' "$SESSION_LOG" 2>/dev/null)

  NARRATIVE_SIZE=${#RAW_NARRATIVE}
  MAX_CHARS=80000
  if [ "$NARRATIVE_SIZE" -gt "$MAX_CHARS" ]; then
    GEMINI_SUMMARY="[SESSION LOG TRUNCATED — showing most recent ${MAX_CHARS} of ${NARRATIVE_SIZE} chars. Full log: $SESSION_LOG]

$(echo "$RAW_NARRATIVE" | tail -c "$MAX_CHARS")"
  else
    GEMINI_SUMMARY="$RAW_NARRATIVE"
  fi
fi

# Build the recovery message
if [ -n "$GEMINI_SUMMARY" ]; then
  RECOVERY_MSG="🔄 COMPACTION RECOVERY - CONTEXT RESTORED

**Timestamp:** $TIMESTAMP
**Session Log:** $SESSION_LOG

$GEMINI_SUMMARY

---

**You're back.** The narrative above was extracted from the transcript right before compaction.
Read it, orient yourself, and continue where you left off."
else
  # Fallback if no summary found
  RECOVERY_MSG="🔄 COMPACTION OCCURRED

**Timestamp:** $TIMESTAMP
**Session Log:** $SESSION_LOG

⚠️ No session narrative found. Please read the session log manually.
The session log should contain the narrative from before compaction."
fi

# Inject lessons into recovery context (dedup + capped).
append_lessons() {
  local label="$1"
  local path="$2"
  [ -z "$path" ] && return
  [ ! -f "$path" ] && return

  # Normalize to avoid double-including same file via different labels.
  local norm
  norm="$(cd "$(dirname "$path")" 2>/dev/null && pwd)/$(basename "$path")"
  case "$ADDED_LESSON_PATHS" in
    *"|$norm|"*) return ;;
  esac
  ADDED_LESSON_PATHS="${ADDED_LESSON_PATHS}${norm}|"

  local body
  body="$(cat "$path")"
  local size=${#body}
  if [ "$size" -gt "$LESSONS_MAX_CHARS" ]; then
    body="$(printf "%s" "$body" | head -c "$LESSONS_MAX_CHARS")"
    body="$body

[LESSONS TRUNCATED — showing first ${LESSONS_MAX_CHARS} of ${size} chars]"
  fi

  RECOVERY_MSG="$RECOVERY_MSG

---
$label ($path):
$body"
}

append_lessons "📚 GLOBAL LESSONS" "$HOME/.claude/lessons.md"
if [ -n "$PROJECT_DIR" ]; then
  append_lessons "📁 PROJECT LESSONS" "$PROJECT_DIR/lessons.md"
fi

# Note: /dev/tty write was attempted here but caused Claude Code's Ink UI to
# corrupt and the hook to error. Removed. Context is available via additionalContext
# below — Claude receives it and can surface key points in its first response.

# Output for Claude to see
jq -n --arg msg "$RECOVERY_MSG" '{
  "hookSpecificOutput": {
    "hookEventName": "SessionStart",
    "additionalContext": $msg
  }
}'

exit 0
