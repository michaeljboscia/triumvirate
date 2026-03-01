#!/bin/bash
# Shared helper: find the latest session log for a project directory.
# Sources into parent hook — sets SESSION_LOG variable.
#
# Usage:
#   source "$HOME/.claude/hooks/_find-session-log.sh"
#   _find_session_log "/path/to/project"
#   echo "$SESSION_LOG"   # → full path to latest log, or empty
#
# Resolution order:
#   1. $AI_MEMORY_DIR/<repo>/  (central memory store, default ~/.ai-memory)
#   2. <project>/session-logs/ (project-local, new naming format)
#   3. <project>/             (project-local, legacy root-level)
#   4. <project>/session-logs/ + <project>/ (old _session_v* format)

_find_session_log() {
  local pdir="$1"
  local ai_mem="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
  local repo=""
  SESSION_LOG=""

  [ -z "$pdir" ] && return
  [ ! -d "$pdir" ] && return

  # Get repo name from taxonomy → git remote → directory basename
  [ -f "$pdir/.claude/taxonomy.json" ] && repo=$(jq -r '.repo // empty' "$pdir/.claude/taxonomy.json" 2>/dev/null)
  [ -z "$repo" ] && repo=$(git -C "$pdir" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
  [ -z "$repo" ] && repo=$(basename "$pdir")

  # 1. AI memory (central store)
  [ -d "$ai_mem/$repo" ] && SESSION_LOG=$(ls -t "$ai_mem/$repo/"*--*_v*.md 2>/dev/null | head -1)

  # 2. Project-local session-logs/ (new format)
  [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$pdir/session-logs/"*--*_v*.md 2>/dev/null | head -1)

  # 3. Project root (legacy new-format)
  [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$pdir"/*--*_v*.md 2>/dev/null | head -1)

  # 4. Old _session_v* format (both locations)
  [ -z "$SESSION_LOG" ] && SESSION_LOG=$(ls -t "$pdir/session-logs/"*_session_v*.md "$pdir"/*_session_v*.md 2>/dev/null | head -1)
}
