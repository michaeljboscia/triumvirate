#!/bin/bash
# Codex SessionStart Hook (Bi-directional)
# Reads session logs on startup and returns context to inject into the session.
#
# INPUT: JSON payload via stdin
# OUTPUT: JSON response with hookSpecificOutput.additionalContext

# Read JSON from stdin
INPUT=$(cat)

# Parse JSON payload
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // empty')
PROJECT_DIR=$(echo "$INPUT" | jq -r '.cwd // empty')
MODEL=$(echo "$INPUT" | jq -r '.hook_event.model // empty')
PROVIDER=$(echo "$INPUT" | jq -r '.hook_event.provider // empty')

# Log session start
HOOK_LOG="/tmp/codex-session.log"
TIMESTAMP=$(date '+%Y-%m-%d %H:%M:%S')
{
  echo "[$TIMESTAMP] Codex session started"
  echo "  Session ID: $SESSION_ID"
  echo "  Model: $MODEL"
  echo "  Provider: $PROVIDER"
  echo "  CWD: $PROJECT_DIR"
  echo "---"
} >> "$HOOK_LOG"

# Try to find and return session log context (like Claude's hook)
if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then
  # Get taxonomy from .claude/taxonomy.json (shared with all agents)
  TAXONOMY_FILE="$PROJECT_DIR/.claude/taxonomy.json"

  if [ -f "$TAXONOMY_FILE" ]; then
    OWNER=$(jq -r '.owner // "unknown"' "$TAXONOMY_FILE")
    CLIENT=$(jq -r '.client // "unknown"' "$TAXONOMY_FILE")
    DOMAIN=$(jq -r '.domain // "unknown"' "$TAXONOMY_FILE")
  else
    OWNER=$(git -C "$PROJECT_DIR" config user.name 2>/dev/null | tr ' ' '-' | tr '[:upper:]' '[:lower:]')
    if [ -z "$OWNER" ]; then
      OWNER="unknown"
    fi
    CLIENT="unknown"
    DOMAIN="unknown"
  fi

  # Get repo name
  if [ -f "$TAXONOMY_FILE" ]; then
    REPO=$(jq -r '.repo // empty' "$TAXONOMY_FILE")
  fi
  if [ -z "$REPO" ]; then
    REPO=$(git -C "$PROJECT_DIR" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
  fi
  if [ -z "$REPO" ]; then
    REPO=$(basename "$PROJECT_DIR")
  fi

  # Session logs directory
  SESSION_DIR="$PROJECT_DIR/session-logs"

  # Find most recent session log (any agent: claude, codex, or gemini)
  LATEST_LOG=$(ls -t "$SESSION_DIR"/${OWNER}--${CLIENT}_${DOMAIN}_${REPO}_*_v*.md 2>/dev/null | head -1)

  # Fall back to old format
  if [ -z "$LATEST_LOG" ]; then
    LATEST_LOG=$(ls -t "$SESSION_DIR"/*_session_v*.md "$PROJECT_DIR"/*_session_v*.md 2>/dev/null | head -1)
  fi

  if [ -n "$LATEST_LOG" ] && [ -f "$LATEST_LOG" ]; then
    CONTENT=$(cat "$LATEST_LOG")
    CONTEXT="🔔 Session log found for Codex!

File: $LATEST_LOG
Taxonomy: ${OWNER}/${CLIENT}/${DOMAIN}/${REPO}

$CONTENT"

    # Return JSON response with context (Claude-compatible format)
    jq -n --arg ctx "$CONTEXT" '{
      "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": $ctx
      }
    }'
    exit 0
  fi
fi

# No session log found - return empty response
echo '{}'
exit 0
