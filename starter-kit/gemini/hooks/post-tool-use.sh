#!/bin/bash
# POST-TOOL-USE: Auto-stage edited files, log activity to session log

INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty')
CWD=$(echo "$INPUT" | jq -r '.cwd // empty')
PROJECT_DIR="$CWD"

# Find session log (AI_MEMORY_DIR first, then project-local)
SESSION_LOG=""
if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then
  AI_MEM="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
  REPO=""
  [ -f "$PROJECT_DIR/.claude/taxonomy.json" ] && REPO=$(jq -r '.repo // empty' "$PROJECT_DIR/.claude/taxonomy.json" 2>/dev/null)
  [ -z "$REPO" ] && REPO=$(git -C "$PROJECT_DIR" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
  [ -z "$REPO" ] && REPO=$(basename "$PROJECT_DIR")

  [ -d "$AI_MEM/$REPO" ] && SESSION_LOG=$(ls -t "$AI_MEM/$REPO/"*--*_v*.md 2>/dev/null | head -1)
  [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$PROJECT_DIR/session-logs/"*_v*.md 2>/dev/null | head -1)
  [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$PROJECT_DIR/.gemini/session-logs/"*_v*.md 2>/dev/null | head -1)
fi

log_action() {
  local msg="$1"
  if [ -n "$SESSION_LOG" ] && [ -f "$SESSION_LOG" ]; then
    echo "| $(date '+%H:%M') | $msg |" >> "$SESSION_LOG"
  fi
}

case "$TOOL_NAME" in
  "run_shell_command")
    CMD=$(echo "$INPUT" | jq -r '.tool_input.command // empty')
    EXIT=$(echo "$INPUT" | jq -r '.tool_response.exit_code // 0')

    if echo "$CMD" | grep -q "git commit"; then
      log_action "Git Commit: $CMD | COMMITTED"
    elif echo "$CMD" | grep -qiE "(test|pytest|npm test)"; then
      if [ "$EXIT" = "0" ]; then
        log_action "Tests Passed: $CMD | BLESSED"
      else
        log_action "Tests Failed: $CMD | BROKEN"
      fi
    fi
    ;;
  "write_file"|"replace")
    FILE=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')
    if [ -n "$FILE" ] && [ -n "$PROJECT_DIR" ]; then
      git -C "$PROJECT_DIR" add "$FILE" 2>/dev/null
      log_action "Edited $FILE (Staged) | WORKING"
    fi
    ;;
esac

exit 0
