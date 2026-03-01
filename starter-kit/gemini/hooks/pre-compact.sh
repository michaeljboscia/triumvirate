#!/bin/bash
# IRON LAW: AUTO-SAVE session state before compaction (NO USER ACTION REQUIRED)
# Naming: <owner>--<client>_<domain>_<repo>_<feature>_<YYYYMMDD>_v<N>_gemini.md
# Uses Gemini Flash to summarize its own transcript before memory loss.

INPUT=$(cat)
PROJECT_DIR=$(echo "$INPUT" | jq -r '.cwd // empty')
TRANSCRIPT_PATH=$(echo "$INPUT" | jq -r '.transcript_path // empty')
TRIGGER="auto"
TIMESTAMP=$(date '+%Y%m%d_%H%M%S')
DATE_ONLY=$(date '+%Y%m%d')

if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then

  # Get taxonomy — check .claude/ first (shared), then .gemini/
  TAXONOMY_FILE="$PROJECT_DIR/.claude/taxonomy.json"
  [ ! -f "$TAXONOMY_FILE" ] && TAXONOMY_FILE="$PROJECT_DIR/.gemini/taxonomy.json"

  if [ -f "$TAXONOMY_FILE" ]; then
    OWNER=$(jq -r '.owner // "unknown"' "$TAXONOMY_FILE")
    CLIENT=$(jq -r '.client // "unknown"' "$TAXONOMY_FILE")
    DOMAIN=$(jq -r '.domain // "unknown"' "$TAXONOMY_FILE")
    FEATURE=$(jq -r '.feature // "general"' "$TAXONOMY_FILE")
    REPO=$(jq -r '.repo // empty' "$TAXONOMY_FILE")
  else
    OWNER=$(git -C "$PROJECT_DIR" config user.name 2>/dev/null | tr ' ' '-' | tr '[:upper:]' '[:lower:]')
    [ -z "$OWNER" ] && OWNER="unknown"
    CLIENT="unknown"
    DOMAIN="unknown"
    FEATURE="general"
  fi

  # Get repo name
  [ -z "$REPO" ] && REPO=$(git -C "$PROJECT_DIR" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
  [ -z "$REPO" ] && REPO=$(basename "$PROJECT_DIR")

  # Determine session log directory (AI_MEMORY_DIR preferred)
  AI_MEM="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
  if [ -d "$AI_MEM/$REPO" ]; then
    SESSION_DIR="$AI_MEM/$REPO"
  else
    SESSION_DIR="$PROJECT_DIR/session-logs"
  fi
  mkdir -p "$SESSION_DIR"

  # Find existing logs to determine version
  EXISTING=$(ls "$SESSION_DIR"/${OWNER}--${CLIENT}_${DOMAIN}_${REPO}_*_v*.md 2>/dev/null | wc -l | tr -d ' ')
  NEXT_VERSION=$((EXISTING + 1))

  # Agent identifier for cross-agent session log compatibility
  AGENT="gemini"
  NEW_LOG="$SESSION_DIR/${OWNER}--${CLIENT}_${DOMAIN}_${REPO}_${FEATURE}_${DATE_ONLY}_v${NEXT_VERSION}_${AGENT}.md"

  # Extract and summarize transcript
  GEMINI_SUMMARY=""
  if [ -n "$TRANSCRIPT_PATH" ] && [ -f "$TRANSCRIPT_PATH" ]; then
    FULL_CONTEXT=$(cat "$TRANSCRIPT_PATH")

    # Cap at 3MB to avoid blowing up the summarization call
    CONTEXT_SIZE=${#FULL_CONTEXT}
    if [ "$CONTEXT_SIZE" -gt 3000000 ]; then
      RECENT_CONTEXT=$(echo "$FULL_CONTEXT" | tail -c 3000000)
      TRUNCATION_NOTE="[TRUNCATED - showing last 3M chars] "
    else
      RECENT_CONTEXT="$FULL_CONTEXT"
      TRUNCATION_NOTE=""
    fi

    if [ -n "$RECENT_CONTEXT" ]; then
      GEMINI_BIN="${GEMINI_CLI_PATH:-gemini}"
      GEMINI_SUMMARY=$( echo "${TRUNCATION_NOTE}${RECENT_CONTEXT}" | \
        "$GEMINI_BIN" run --output-format text \
        "You are helping preserve context for an AI coding assistant that is about to lose its memory.
Summarize this ENTIRE conversation comprehensively.
Structure:
## Goal
## Key Decisions
## What Was Built/Changed
## What Works
## What Doesn't Work
## Current State
## Critical Context
Be SPECIFIC." 2>/dev/null | grep -v "DeprecationWarning" )
    fi
  fi

  # Write session log
  {
    echo "# Session Log: ${OWNER}/${CLIENT}/${DOMAIN}/${REPO}"
    echo "**Feature:** ${FEATURE}"
    echo "**Generated:** $(date '+%Y-%m-%d %H:%M:%S')"
    echo "**Trigger:** ${TRIGGER} compaction"
    echo "**Agent:** ${AGENT}"
    echo ""
    echo "## 🧠 GEMINI CONTEXT SUMMARY"
    if [ -n "$GEMINI_SUMMARY" ]; then
      echo "$GEMINI_SUMMARY"
    else
      echo "⚠️ Gemini summarization failed or transcript was empty."
    fi
    echo ""
    echo "---"
    echo "## SESSION ACTIVITY LOG"
    echo "| Time | Action | Outcome |"
    echo "|------|--------|---------|"
    echo "| $(date '+%H:%M') | PreCompact auto-save | Created ${NEW_LOG##*/} |"
  } > "$NEW_LOG"

  # Git commit
  if [ -d "$PROJECT_DIR/.git" ]; then
    git -C "$PROJECT_DIR" add "$NEW_LOG" 2>/dev/null
    git -C "$PROJECT_DIR" commit -m "auto-save: gemini pre-compact session log" 2>/dev/null
  fi

  jq -n --arg log "$NEW_LOG" '{
    "hookSpecificOutput": {
      "hookEventName": "PreCompact",
      "additionalContext": "✅ AUTO-SAVED session log: " + $log
    }
  }'
fi

exit 0
