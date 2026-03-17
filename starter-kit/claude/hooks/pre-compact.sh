#!/bin/bash
# PRE-COMPACT v2 — Gap-Fill Only
#
# Role: At compaction time, find what the stenographer missed and have Gemini
# fill ONLY the gaps. The stenographer handles incremental saves (Ollama).
# This hook handles the final gap-fill (Gemini CLI, free via subscription).
#
# Architecture:
#   1. gap_fill.py extracts the transcript gap (from stenographer's last save to EOF)
#   2. If gap exists: pipe gap + existing notes to Gemini CLI for gap-fill summary
#   3. Append gap-fill section to the rolling session log
#   4. Git commit the log
#
# Key fixes over v1:
#   - Uses session_log_path.py for ALL path resolution (no more 4 implementations)
#   - Appends to rolling file (no more _v47 explosion)
#   - Processes ONLY the gap (no more 15x full re-summarization)
#   - Gemini stdout filtered with start/end markers (no more "MCP issues" pollution)

INPUT=$(cat)
PROJECT_DIR=$(echo "$INPUT" | jq -r '.cwd // empty')
TRANSCRIPT_PATH=$(echo "$INPUT" | jq -r '.transcript_path // empty')
TRIGGER=$(echo "$INPUT" | jq -r '.trigger // "auto"')

# ── ASYNC SELF-BACKGROUND ────────────────────────────────────────────────────
# The Gemini gap-fill can take 2-16 minutes (CLI startup + API call).
# We NEVER block compaction for that long — return immediately and do all
# heavy work in a background re-invocation.
#
# Pattern: first run (no PRECOMPACT_BG env) → re-invoke self in background → exit 0
#          second run (PRECOMPACT_BG=1)     → do the actual work
if [ "${PRECOMPACT_BG:-0}" != "1" ]; then
  printf '%s' "$INPUT" | PRECOMPACT_BG=1 "$0" >> "$HOME/.triumvirate/precompact-bg.log" 2>&1 &
  disown

  # Return immediately to unblock compaction
  jq -n '{
    "hookSpecificOutput": {
      "hookEventName": "PreCompact",
      "additionalContext": "Gap-fill running in background (async). Log will be updated when Gemini responds."
    }
  }'
  exit 0
fi

# Source credentials vault — exports AI_MEMORY_DIR and other env vars
if [ -f "$HOME/.claude/.env" ]; then
  set -a
  source "$HOME/.claude/.env"
  set +a
fi

STENOGRAPHER_DIR="$HOME/.triumvirate/stenographer"
GAP_FILL_PY="$STENOGRAPHER_DIR/gap_fill.py"

if [ -z "$PROJECT_DIR" ] || [ ! -d "$PROJECT_DIR" ]; then
  exit 0
fi

# ── Concurrency lock ─────────────────────────────────────────────────────────
# Prevents token-gate background run + native compaction from racing.
# mkdir is atomic on macOS (HFS+/APFS) — safe alternative to flock (Linux-only).
LOCK_HASH=$(printf '%s' "$PROJECT_DIR" | md5 -q 2>/dev/null || printf '%s' "$PROJECT_DIR" | md5sum | cut -c1-32)
LOCK_DIR="/tmp/claude-precompact-${LOCK_HASH}.lock"
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  _LOCK_PID=""
  [ -f "$LOCK_DIR/pid" ] && _LOCK_PID=$(cat "$LOCK_DIR/pid" 2>/dev/null)
  if [ -z "$_LOCK_PID" ]; then
    _LOCK_AGE=$(( $(date +%s) - $(stat -f %m "$LOCK_DIR" 2>/dev/null || stat -c %Y "$LOCK_DIR" 2>/dev/null || echo 0) ))
    if [ "$_LOCK_AGE" -lt 10 ]; then
      echo "pre-compact.sh: lock exists with no PID yet (age ${_LOCK_AGE}s) — skipping" >&2
      exit 0
    else
      echo "pre-compact.sh: removing stale lock with no PID (age ${_LOCK_AGE}s)" >&2
      rm -rf "$LOCK_DIR"
      mkdir "$LOCK_DIR" 2>/dev/null || { echo "pre-compact.sh: could not acquire lock after stale removal" >&2; exit 0; }
    fi
  elif kill -0 "$_LOCK_PID" 2>/dev/null; then
    echo "pre-compact.sh already running (pid $_LOCK_PID) — skipping (race prevention)" >&2
    exit 0
  else
    echo "pre-compact.sh: removing stale lock (pid $_LOCK_PID dead)" >&2
    rm -rf "$LOCK_DIR"
    mkdir "$LOCK_DIR" 2>/dev/null || { echo "pre-compact.sh: could not acquire lock after stale removal" >&2; exit 0; }
  fi
fi
echo $$ > "$LOCK_DIR/pid" 2>/dev/null || true
trap "rm -rf '$LOCK_DIR'" EXIT INT TERM

# ── Gap extraction ───────────────────────────────────────────────────────────
# gap_fill.py handles: path resolution (session_log_path.py), stenographer state,
# Claude parser delta extraction. Outputs JSON.
GAP_JSON=""
if [ -n "$TRANSCRIPT_PATH" ] && [ -f "$TRANSCRIPT_PATH" ] && [ -f "$GAP_FILL_PY" ]; then
  GAP_JSON=$(python3 "$GAP_FILL_PY" \
    --project-dir "$PROJECT_DIR" \
    --transcript "$TRANSCRIPT_PATH" \
    --agent claude 2>/dev/null)
fi

# Parse gap_fill.py output
LOG_PATH=""
HAS_GAP=false
GAP_TEXT=""
EXISTING_NOTES=""

if [ -n "$GAP_JSON" ]; then
  LOG_PATH=$(echo "$GAP_JSON" | jq -r '.log_path // empty')
  HAS_GAP=$(echo "$GAP_JSON" | jq -r '.has_gap // false')
  GAP_TEXT=$(echo "$GAP_JSON" | jq -r '.gap_text // empty')
  EXISTING_NOTES=$(echo "$GAP_JSON" | jq -r '.existing_notes // empty')
fi

# Fallback: if gap_fill.py didn't provide a log path, use session_log_path.py directly
if [ -z "$LOG_PATH" ]; then
  SESSION_LOG_PY="$STENOGRAPHER_DIR/session_log_path.py"
  if [ -f "$SESSION_LOG_PY" ]; then
    if [ -n "$TRANSCRIPT_PATH" ]; then
      LOG_PATH=$(python3 "$SESSION_LOG_PY" --get "$PROJECT_DIR" --transcript "$TRANSCRIPT_PATH" --agent claude 2>/dev/null)
    else
      LOG_PATH=$(python3 "$SESSION_LOG_PY" --recover "$PROJECT_DIR" --agent claude 2>/dev/null)
    fi
  fi
fi

# If we still have no log path, fall back to creating one in the right directory
if [ -z "$LOG_PATH" ]; then
  SESSION_LOG_PY="$STENOGRAPHER_DIR/session_log_path.py"
  if [ -f "$SESSION_LOG_PY" ]; then
    LOG_DIR=$(python3 "$SESSION_LOG_PY" --dir "$PROJECT_DIR" 2>/dev/null)
  fi
  if [ -z "$LOG_DIR" ]; then
    # Ultimate fallback
    if [ "$PROJECT_DIR" = "$HOME" ] || [ "$PROJECT_DIR" = "$HOME/" ]; then
      LOG_DIR="$HOME/.claude/session-logs"
    else
      _AI_MEM="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
      _REPO=$(basename "$PROJECT_DIR")
      LOG_DIR="$_AI_MEM/$_REPO"
    fi
  fi
  mkdir -p "$LOG_DIR"
  TRANSCRIPT_UUID=$(basename "${TRANSCRIPT_PATH:-.}" .jsonl)
  LOG_PATH="$LOG_DIR/precompact_fallback_${TRANSCRIPT_UUID}_claude.md"
  echo "# Session Log (pre-compact fallback)" > "$LOG_PATH"
  echo "" >> "$LOG_PATH"
  echo "**Created:** $(date '+%Y-%m-%d %H:%M:%S %Z')" >> "$LOG_PATH"
  echo "" >> "$LOG_PATH"
fi

# ── Gemini gap-fill summarization ────────────────────────────────────────────
GEMINI_SUMMARY=""

if [ "$HAS_GAP" = "true" ] && [ -n "$GAP_TEXT" ]; then
  GEMINI_BIN="${GEMINI_CLI_PATH:-$(command -v gemini 2>/dev/null || echo gemini)}"

  if command -v "$GEMINI_BIN" &>/dev/null || [ -x "$GEMINI_BIN" ]; then
    # macOS-compatible timeout via Python (GNU 'timeout' is not available on macOS by default)
    # This wraps the Gemini call with a hard 120s deadline regardless of OS.
    _GEMINI_TIMEOUT=120

    # Build the gap-fill prompt with start/end markers to filter stdout pollution
    GEMINI_PROMPT="You are preserving context for an AI coding assistant that is about to lose its memory due to context compaction. Below you will find:

1. EXISTING NOTES — what has already been documented by the incremental stenographer
2. GAP TRANSCRIPT — the portion of the session NOT yet captured in the notes

Your job: write ONLY what is missing. Do NOT repeat what is already in the notes. Focus on:
- Decisions made and why
- What was built or changed (file paths, function names)
- What worked vs what failed
- Current state and immediate next steps
- Any critical context (API keys, URLs, gotchas)

Write 3-7 substantive paragraphs covering ONLY the gap. Be specific with file paths, function names, error messages. Write in past tense.

IMPORTANT: Wrap your ENTIRE response between these exact markers:
===BEGIN_SUMMARY===
[your summary here]
===END_SUMMARY===

--- EXISTING NOTES ---
$(echo "$EXISTING_NOTES" | head -c 50000)

--- GAP TRANSCRIPT ---
$GAP_TEXT"

    # Pipe to Gemini CLI with Python-enforced timeout (macOS-compatible)
    # Python's subprocess.run(timeout=) works on all platforms; GNU 'timeout' does not.
    RAW_GEMINI=$(printf '%s' "$GEMINI_PROMPT" | python3 -c "
import subprocess, sys, os
data = sys.stdin.buffer.read()
try:
    r = subprocess.run(
        ['$GEMINI_BIN', '--output-format', 'text', '--approval-mode', 'yolo',
         '-p', 'Process the input and respond with ONLY the gap-fill summary wrapped in ===BEGIN_SUMMARY=== and ===END_SUMMARY=== markers.'],
        input=data, capture_output=True, timeout=$_GEMINI_TIMEOUT
    )
    sys.stdout.buffer.write(r.stdout)
except subprocess.TimeoutExpired:
    sys.stderr.write('Gemini timed out after ${_GEMINI_TIMEOUT}s\n')
except Exception as e:
    sys.stderr.write(f'Gemini error: {e}\n')
" 2>/dev/null || true)

    # Extract ONLY between markers — this filters out ALL stdout pollution
    # (MCP diagnostics, deprecation warnings, hook registry messages, etc.)
    if [ -n "$RAW_GEMINI" ]; then
      GEMINI_SUMMARY=$(echo "$RAW_GEMINI" | sed -n '/===BEGIN_SUMMARY===/,/===END_SUMMARY===/p' | sed '1d;$d')
    fi

    # Validation gate: summary must be substantive
    if [ -n "$GEMINI_SUMMARY" ]; then
      _SUMMARY_LEN=${#GEMINI_SUMMARY}
      if [ "$_SUMMARY_LEN" -lt 200 ]; then
        echo "pre-compact.sh: Gemini summary too short (${_SUMMARY_LEN} chars) — discarding" >&2
        GEMINI_SUMMARY=""
      fi
    fi

    # If marker extraction failed but raw output is long, try cleaning it
    if [ -z "$GEMINI_SUMMARY" ] && [ -n "$RAW_GEMINI" ]; then
      # Strip known pollution patterns
      CLEANED=$(echo "$RAW_GEMINI" | \
        grep -v "^DeprecationWarning" | \
        grep -v "^Hook registry" | \
        grep -v "^Loaded cached" | \
        grep -v "^MCP issues detected" | \
        grep -v "^Run /mcp" | \
        grep -v "^===BEGIN_SUMMARY===" | \
        grep -v "^===END_SUMMARY===")
      _CLEANED_LEN=${#CLEANED}
      if [ "$_CLEANED_LEN" -gt 200 ]; then
        GEMINI_SUMMARY="$CLEANED"
      fi
    fi
  fi
fi

# ── Append gap-fill section to rolling session log ──────────────────────────
TIMESTAMP=$(date '+%H:%M %Z')
DATE_STR=$(date '+%Y-%m-%d')

if [ -n "$GEMINI_SUMMARY" ]; then
  {
    echo ""
    echo "---"
    echo ""
    echo "## ${TIMESTAMP} — Gemini Gap-Fill (${DATE_STR}, compaction)"
    echo ""
    echo "$GEMINI_SUMMARY"
    echo ""
    echo "*Gap-fill by Gemini at compaction. Trigger: ${TRIGGER}.*"
    echo ""
  } >> "$LOG_PATH"
  echo "pre-compact.sh: Gap-fill appended to $(basename "$LOG_PATH")" >&2
elif [ "$HAS_GAP" = "true" ]; then
  # Gap existed but Gemini failed — append raw gap as fallback
  {
    echo ""
    echo "---"
    echo ""
    echo "## ${TIMESTAMP} — Raw Gap (Gemini unavailable, ${DATE_STR})"
    echo ""
    echo "**Gemini gap-fill failed. Raw transcript gap below. Trigger: ${TRIGGER}.**"
    echo ""
    echo "$GAP_TEXT" | head -c 100000
    echo ""
  } >> "$LOG_PATH"
  echo "pre-compact.sh: Raw gap appended (Gemini failed) to $(basename "$LOG_PATH")" >&2
else
  # No gap — stenographer covered everything. Just note the compaction.
  {
    echo ""
    echo "---"
    echo ""
    echo "## ${TIMESTAMP} — Context Compaction (${DATE_STR})"
    echo ""
    echo "Compaction occurred. Stenographer notes are up to date — no gap to fill."
    echo "Trigger: ${TRIGGER}."
    echo ""
  } >> "$LOG_PATH"
  echo "pre-compact.sh: No gap — compaction marker appended to $(basename "$LOG_PATH")" >&2
fi

# ── Git commit ──────────────────────────────────────────────────────────────
LOG_GIT_DIR=$(git -C "$(dirname "$LOG_PATH")" rev-parse --show-toplevel 2>/dev/null)
if [ -n "$LOG_GIT_DIR" ]; then
  cd "$LOG_GIT_DIR"

  if [ "$TRIGGER" = "token-gate" ]; then
    # BACKGROUND RUN: skip the index dance to prevent race conditions
    git add "$LOG_PATH" 2>/dev/null
    git commit -m "Auto-save: TokenGate gap-fill ($(date '+%Y-%m-%d %H:%M'))" 2>/dev/null
  else
    # SYNCHRONOUS RUN: full isolation — commit only the session log
    PREV_STAGED=$(git diff --cached --name-only 2>/dev/null)
    if [ -n "$PREV_STAGED" ]; then
      git reset HEAD --quiet 2>/dev/null
    fi
    git add "$LOG_PATH" 2>/dev/null
    git commit -m "Auto-save: PreCompact gap-fill ($(date '+%Y-%m-%d %H:%M'))" 2>/dev/null
    if [ -n "$PREV_STAGED" ]; then
      while IFS= read -r _staged_file; do
        [ -n "$_staged_file" ] && git add "$_staged_file" 2>/dev/null
      done <<< "$PREV_STAGED"
    fi
  fi
fi

# ── Hook output ──────────────────────────────────────────────────────────────
echo "pre-compact v2: $(basename "$LOG_PATH")" >&2

jq -n --arg log "$LOG_PATH" --arg gap "$HAS_GAP" '{
  "hookSpecificOutput": {
    "hookEventName": "PreCompact",
    "additionalContext": ("AUTO-SAVED session log (gap-fill v2):\n" + $log + "\nHad gap: " + $gap)
  }
}'

exit 0
