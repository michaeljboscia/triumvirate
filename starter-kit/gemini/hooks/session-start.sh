#!/bin/bash
# GEMINI SESSION START: Recover context from session log on startup or compaction
# Looks for session logs in AI_MEMORY_DIR or .gemini/session-logs/

INPUT=$(cat)
PROJECT_DIR=$(echo "$INPUT" | jq -r '.cwd // empty')
TRIGGER=$(echo "$INPUT" | jq -r '.trigger // "manual"')

if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then
  # Determine session log location
  AI_MEM="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
  REPO=""
  [ -f "$PROJECT_DIR/.claude/taxonomy.json" ] && REPO=$(jq -r '.repo // empty' "$PROJECT_DIR/.claude/taxonomy.json" 2>/dev/null)
  [ -z "$REPO" ] && [ -f "$PROJECT_DIR/.gemini/taxonomy.json" ] && REPO=$(jq -r '.repo // empty' "$PROJECT_DIR/.gemini/taxonomy.json" 2>/dev/null)
  [ -z "$REPO" ] && REPO=$(git -C "$PROJECT_DIR" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
  [ -z "$REPO" ] && REPO=$(basename "$PROJECT_DIR")

  # Search: AI_MEMORY_DIR first, then project-local
  LATEST_LOG=""
  [ -d "$AI_MEM/$REPO" ] && LATEST_LOG=$(ls -t "$AI_MEM/$REPO/"*--*_v*.md 2>/dev/null | head -1)
  [ -z "$LATEST_LOG" ] && LATEST_LOG=$(ls -t "$PROJECT_DIR/.gemini/session-logs/"*_v*.md 2>/dev/null | head -1)
  [ -z "$LATEST_LOG" ] && LATEST_LOG=$(ls -t "$PROJECT_DIR/session-logs/"*_v*.md 2>/dev/null | head -1)

  if [ -n "$LATEST_LOG" ] && [ -f "$LATEST_LOG" ]; then
    # Extract Gemini Summary section if available
    GEMINI_SUMMARY=$(awk '
      /^## 🧠 GEMINI CONTEXT SUMMARY/{found=1; next}
      found && /^## (CONTEXT COMPACTION OCCURRED|SESSION ACTIVITY LOG|INSTRUCTIONS FOR NEXT SESSION|TRANSCRIPT HISTORY|TAXONOMY)/{exit}
      found{print}
    ' "$LATEST_LOG" 2>/dev/null)

    if [ -n "$GEMINI_SUMMARY" ]; then
      MSG="🔔 SESSION LOG FOUND: $LATEST_LOG

$GEMINI_SUMMARY"
    else
      MSG="🔔 SESSION LOG FOUND: $LATEST_LOG (No summary section — read it for context)"
    fi

    jq -n --arg msg "$MSG" '{
      "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": $msg
      }
    }'
  else
    jq -n '{
      "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": "🛑 NO SESSION LOG FOUND. Create .claude/taxonomy.json in your project to enable session persistence."
      }
    }'
  fi
else
  echo '{}'
fi
exit 0
