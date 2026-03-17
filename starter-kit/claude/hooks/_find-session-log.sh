#!/bin/bash
# Shared helper: find the latest session log for a project directory.
# Sources into parent hook — sets SESSION_LOG variable.
#
# v2: Delegates to session_log_path.py (single source of truth) with
# legacy fallbacks for backward compatibility.
#
# Usage:
#   source "$HOME/.claude/hooks/_find-session-log.sh"
#   _find_session_log "/path/to/project"
#   echo "$SESSION_LOG"   # → full path to latest log, or empty

_find_session_log() {
  local pdir="$1"
  local transcript="$2"  # optional transcript path for more precise lookup
  SESSION_LOG=""

  [ -z "$pdir" ] && return
  [ ! -d "$pdir" ] && return

  local slp="$HOME/.triumvirate/stenographer/session_log_path.py"

  # Primary: use session_log_path.py (single source of truth)
  if [ -f "$slp" ]; then
    if [ -n "$transcript" ]; then
      # If transcript provided, use --find with transcript for exact match
      SESSION_LOG=$(python3 "$slp" --find "$pdir" --transcript "$transcript" --agent claude 2>/dev/null)
    fi

    # If no exact match, try recovery mode (most recent for this repo)
    if [ -z "$SESSION_LOG" ]; then
      SESSION_LOG=$(python3 "$slp" --recover "$pdir" --agent claude 2>/dev/null)
    fi
  fi

  # Legacy fallbacks (in case session_log_path.py is unavailable or returns empty)
  if [ -z "$SESSION_LOG" ]; then
    local ai_mem="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
    local repo=""

    [ -f "$pdir/.claude/taxonomy.json" ] && repo=$(jq -r '.repo // empty' "$pdir/.claude/taxonomy.json" 2>/dev/null)
    [ -z "$repo" ] && repo=$(git -C "$pdir" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
    [ -z "$repo" ] && repo=$(basename "$pdir")

    # V2 format: *_claude.md (rolling files with agent suffix)
    [ -d "$ai_mem/$repo" ] && SESSION_LOG=$(ls -t "$ai_mem/$repo/"*_claude.md 2>/dev/null | head -1)

    # V1 format: *--*_v*.md
    [ -z "$SESSION_LOG" ] && [ -d "$ai_mem/$repo" ] && SESSION_LOG=$(ls -t "$ai_mem/$repo/"*--*_v*.md 2>/dev/null | head -1)

    # Project-local fallbacks
    [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$pdir/session-logs/"*_claude.md 2>/dev/null | head -1)
    [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$pdir/session-logs/"*--*_v*.md 2>/dev/null | head -1)
    [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$pdir"/*--*_v*.md 2>/dev/null | head -1)
    [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$pdir/session-logs/"*_session_v*.md "$pdir"/*_session_v*.md 2>/dev/null | head -1)
  fi
}
