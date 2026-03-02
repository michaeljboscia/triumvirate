#!/bin/bash
# IRON LAW: AUTO-SAVE session state before compaction (NO USER ACTION REQUIRED)
# Naming: <owner>--<client>_<domain>_<repo>_<feature>_<YYYYMMDD>_v<N>.md

INPUT=$(cat)
PROJECT_DIR=$(echo "$INPUT" | jq -r '.cwd // empty')
TRANSCRIPT_PATH=$(echo "$INPUT" | jq -r '.transcript_path // empty')
TRIGGER=$(echo "$INPUT" | jq -r '.trigger // "auto"')
TIMESTAMP=$(date '+%Y%m%d_%H%M%S')
DATE_ONLY=$(date '+%Y%m%d')

if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then

  # HOME GUARD: If still in home dir, use spec fallback (~/.claude/session-logs/)
  # This prevents the ~/session-logs/ dumping ground bug
  _HOME_BOOTSTRAP=false
  if [ "$PROJECT_DIR" = "$HOME" ] || [ "$PROJECT_DIR" = "$HOME/" ]; then
    _HOME_BOOTSTRAP=true
    SESSION_DIR="$HOME/.claude/session-logs"
    mkdir -p "$SESSION_DIR"
    # Use bootstrap taxonomy — not the stale ~/.claude/taxonomy.json
    OWNER="$(whoami)"
    CLIENT="home"
    DOMAIN="bootstrap"
    REPO="home"
    FEATURE="unsorted"
  fi

  # Get taxonomy from .claude/taxonomy.json or use defaults (skip if HOME guard already set them)
  # NOTE: Uses dedicated flag, not $OWNER emptiness, to avoid env var leakage (Codex review finding)
  if ! $_HOME_BOOTSTRAP; then
    TAXONOMY_FILE="$PROJECT_DIR/.claude/taxonomy.json"

    if [ -f "$TAXONOMY_FILE" ]; then
      OWNER=$(jq -r '.owner // "unknown"' "$TAXONOMY_FILE")
      CLIENT=$(jq -r '.client // "unknown"' "$TAXONOMY_FILE")
      DOMAIN=$(jq -r '.domain // "unknown"' "$TAXONOMY_FILE")
      FEATURE=$(jq -r '.feature // "general"' "$TAXONOMY_FILE")
    else
      OWNER=$(git -C "$PROJECT_DIR" config user.name 2>/dev/null | tr ' ' '-' | tr '[:upper:]' '[:lower:]')
      if [ -z "$OWNER" ]; then
        OWNER="unknown"
      fi
      CLIENT="unknown"
      DOMAIN="unknown"
      FEATURE="general"
    fi

    # Get repo name from taxonomy, git remote, or fallback to directory basename
    if [ -f "$TAXONOMY_FILE" ]; then
      REPO=$(jq -r '.repo // empty' "$TAXONOMY_FILE")
    fi
    if [ -z "$REPO" ]; then
      REPO=$(git -C "$PROJECT_DIR" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
    fi
    if [ -z "$REPO" ]; then
      REPO=$(basename "$PROJECT_DIR")
    fi

    # Determine session log directory:
    # Prefer $AI_MEMORY_DIR/<repo>/ (central memory store) over project-local
    _AI_MEM_DIR="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
    if [ -d "$_AI_MEM_DIR" ] && [ -n "$REPO" ]; then
      SESSION_DIR="$_AI_MEM_DIR/$REPO"
    else
      SESSION_DIR="$PROJECT_DIR/session-logs"
    fi
    mkdir -p "$SESSION_DIR"
  fi

  # ── Concurrency lock ─────────────────────────────────────────────────────────
  # Prevents token-gate background run + native compaction from racing on NEXT_VERSION.
  # mkdir is atomic on macOS (HFS+/APFS) — safe alternative to flock (Linux-only).
  LOCK_DIR="/tmp/claude-precompact-$(printf '%s' "$SESSION_DIR" | md5 -q 2>/dev/null || printf '%s' "$SESSION_DIR" | md5sum | cut -c1-32).lock"
  if ! mkdir "$LOCK_DIR" 2>/dev/null; then
    # Lock exists — check if the holding process is still alive (stale lock detection).
    # If the process was SIGKILL'd, the trap won't have fired and the lock is stale.
    _LOCK_PID=""
    [ -f "$LOCK_DIR/pid" ] && _LOCK_PID=$(cat "$LOCK_DIR/pid" 2>/dev/null)
    if [ -n "$_LOCK_PID" ] && kill -0 "$_LOCK_PID" 2>/dev/null; then
      echo "⚠️ pre-compact.sh already running (pid $_LOCK_PID) — skipping (race prevention)" >&2
      exit 0
    else
      # Stale lock — remove and proceed
      echo "⚠️ pre-compact.sh: removing stale lock (pid $_LOCK_PID dead or missing)" >&2
      rm -rf "$LOCK_DIR"
      mkdir "$LOCK_DIR" 2>/dev/null || { echo "⚠️ pre-compact.sh: could not acquire lock after stale removal" >&2; exit 0; }
    fi
  fi
  # Write PID so stale detection works if we're killed without running trap
  echo $$ > "$LOCK_DIR/pid" 2>/dev/null || true
  # Guaranteed cleanup: remove lock on exit, interrupt, or termination
  trap "rm -rf '$LOCK_DIR'" EXIT INT TERM

  # Find existing session logs to determine version.
  # Count ONLY this feature's files — avoids version inflation from other features
  # or old-format logs in the same directory.
  EXISTING=$(ls "$SESSION_DIR"/${OWNER}--${CLIENT}_${DOMAIN}_${REPO}_${FEATURE}_*_v*.md 2>/dev/null | wc -l | tr -d ' ')
  NEXT_VERSION=$((EXISTING + 1))

  # Get previous log for reference (check AI memory, then session-logs, then legacy root)
  PREV_LOG=$(ls -t "$SESSION_DIR"/${OWNER}--${CLIENT}_${DOMAIN}_${REPO}_*_v*.md 2>/dev/null | head -1)
  # Also check project-local if SESSION_DIR is AI memory (for migration continuity)
  if [ -z "$PREV_LOG" ] && [ "$SESSION_DIR" != "$PROJECT_DIR/session-logs" ]; then
    PREV_LOG=$(ls -t "$PROJECT_DIR/session-logs/"${OWNER}--${CLIENT}_${DOMAIN}_${REPO}_*_v*.md 2>/dev/null | head -1)
  fi
  if [ -z "$PREV_LOG" ]; then
    PREV_LOG=$(ls -t "$SESSION_DIR"/*_session_v*.md "$PROJECT_DIR"/*_session_v*.md 2>/dev/null | head -1)
  fi
  PREV_LOG_NAME=$(basename "$PREV_LOG" 2>/dev/null || echo "none")

  # Extract transcript UUID from path (e.g., 17cc67d0-e44c-4874-a7ed-ff30b45bc69a from full path)
  TRANSCRIPT_UUID=""
  TRANSCRIPT_SIZE=""
  if [ -n "$TRANSCRIPT_PATH" ] && [ -f "$TRANSCRIPT_PATH" ]; then
    TRANSCRIPT_UUID=$(basename "$TRANSCRIPT_PATH" .jsonl)
    TRANSCRIPT_SIZE=$(du -h "$TRANSCRIPT_PATH" 2>/dev/null | cut -f1)
  fi

  # Build TRANSCRIPT HISTORY: read from previous log and append current
  TRANSCRIPT_HISTORY=""
  if [ -n "$PREV_LOG" ] && [ -f "$PREV_LOG" ]; then
    # Extract existing history from previous log (between ## TRANSCRIPT HISTORY and next ---)
    PREV_HISTORY=$(sed -n '/^## TRANSCRIPT HISTORY/,/^---$/p' "$PREV_LOG" 2>/dev/null | grep "^|" | grep -v "Date" | grep -v "|\-\-\-" || true)
    if [ -n "$PREV_HISTORY" ]; then
      TRANSCRIPT_HISTORY="$PREV_HISTORY"
    fi
  fi

  # New log filename with full taxonomy (in session-logs subdirectory)
  # Agent identifier for cross-agent session log compatibility
  AGENT="claude"

  # New log filename with full taxonomy + agent suffix (in session-logs subdirectory)
  NEW_LOG="$SESSION_DIR/${OWNER}--${CLIENT}_${DOMAIN}_${REPO}_${FEATURE}_${DATE_ONLY}_v${NEXT_VERSION}_${AGENT}.md"

  # SUMMARIZATION STRATEGY:
  # 1. jq narrative (ALWAYS runs first) — structured event log: tool calls, user messages,
  #    Claude actions. Captures what actually happened, not just what was said.
  # 2. Gemini Pro (primary) — reads the jq narrative, produces intelligent structured summary.
  #    Uses Gemini's token budget. If Gemini fails/times out, narrative is used as-is.
  # KEY FIX: Gemini used to receive text-only extraction (select(.type=="text")) which
  # stripped all tool_use/tool_result events. Now Gemini sees the full event log.

  GEMINI_SUMMARY=""
  SESSION_NARRATIVE=""

  if [ -n "$TRANSCRIPT_PATH" ] && [ -f "$TRANSCRIPT_PATH" ]; then

    # ── Step 1: jq structured event log (always — this feeds Gemini AND is the fallback) ──
    SESSION_NARRATIVE=$(jq -r '
      select(.type == "assistant" or .type == "user") |
      if .type == "user" then
        if (.message.content | type) == "string" then
          if (.message.content | test("<command-name>")) then
            "USER invoked skill: " + (
              .message.content |
              capture("<command-name>(?<cmd>[^<]+)</command-name>([\\s\\S]*<command-args>(?<args>[^<]*)</command-args>)?") |
              .cmd + (if .args and .args != "" then " " + .args else "" end)
            )
          else "USER: " + .message.content end
        else
          if ((.message.content | length) == 1
              and (.message.content[0].type == "text")
              and (.message.content[0].text | test("\\nARGUMENTS: [^\\n]+$"))) then
            empty
          else
            [(.message.content // [])[] |
              if .type == "tool_result" then
                if (.content | type) == "string" then
                  if (.content | length) < 200 then "  → returned: " + .content
                  else "  → returned " + (.content | length | tostring) + " chars" end
                else "  → returned result" end
              elif .type == "text" then "USER: " + .text
              else empty end
            ] | join("\n") | select(. != "")
          end
        end
      elif .type == "assistant" then
        [(.message.content // [])[] |
          if .type == "tool_use" then
            if .name == "Bash" then "Claude ran: " + (.input.command // "" | .[0:200])
            elif .name == "Read" then "Claude read: " + (.input.file_path // "")
            elif .name == "Write" then "Claude wrote: " + (.input.file_path // "")
            elif .name == "Edit" then "Claude edited: " + (.input.file_path // "")
            elif .name == "Glob" then "Claude searched: " + (.input.pattern // "")
            elif .name == "Grep" then "Claude grep: " + (.input.pattern // "")
            elif .name == "Task" then "Claude launched subagent: " + (.input.description // "")
            else "Claude used " + .name + ": " + (.input | tojson | .[0:100]) end
          elif .type == "text" then
            if (.text | length) < 400 then "Claude: " + .text
            else "Claude: " + .text[0:400] + "... [" + (.text | length | tostring) + " chars]" end
          else empty end
        ] | join("\n") | select(. != "")
      else empty end
    ' "$TRANSCRIPT_PATH" 2>/dev/null | tr -d '\000')

    # Cap narrative at 150KB — take the TAIL (most recent context is most useful)
    NARRATIVE_SIZE=${#SESSION_NARRATIVE}
    MAX_STORE=150000
    if [ "$NARRATIVE_SIZE" -gt "$MAX_STORE" ]; then
      SESSION_NARRATIVE="[TRUNCATED — showing most recent ${MAX_STORE} of ${NARRATIVE_SIZE} chars. Full detail in transcript: \`$TRANSCRIPT_UUID.jsonl\`]

$(echo "$SESSION_NARRATIVE" | tail -c "$MAX_STORE")"
    fi

    # ── Step 2: Gemini Pro summarization (primary — reads jq narrative, not raw text) ──
    if [ -n "$SESSION_NARRATIVE" ]; then
      GEMINI_BIN="${GEMINI_CLI_PATH:-$(command -v gemini 2>/dev/null || echo gemini)}"
      if command -v "$GEMINI_BIN" &>/dev/null || [ -x "$GEMINI_BIN" ]; then
        # Use timeout if available (GNU coreutils / macOS brew install coreutils)
        _TIMEOUT_CMD=""
        command -v timeout &>/dev/null && _TIMEOUT_CMD="timeout 120"

        GEMINI_SUMMARY=$(printf '%s' "$SESSION_NARRATIVE" | \
          $_TIMEOUT_CMD "$GEMINI_BIN" --output-format text --approval-mode yolo \
          -p "You are preserving context for an AI coding assistant that is about to lose its memory. The input is a structured event log of everything that happened: tool calls (Claude ran/read/wrote/edited), user messages, and results. Summarize comprehensively — this is the AI's ONLY memory.

Structure your response EXACTLY as:

## OVERALL GOAL
What was the user trying to accomplish? What problem were they solving?

## KEY DECISIONS
Every significant decision and WHY. Technical choices, architecture, trade-offs.

## WHAT WAS BUILT / CHANGED
Specific files created or modified (include full paths), commits made (include hashes), integrations configured.

## WHAT WORKS
Things verified working. Include evidence (test results, successful runs, commit hashes).

## WHAT DOESN'T WORK / KNOWN ISSUES
Problems encountered, failures, open questions, bugs found.

## CURRENT STATE
Exactly where did we leave off? What is the IMMEDIATE next step?

## CRITICAL CONTEXT
API keys, URLs, connection strings, gotchas, warnings — anything needed to continue effectively.

Be SPECIFIC: file names, function names, commit hashes, error messages, fix patterns. This summary IS the AI's memory." \
          2>/dev/null | grep -v "^DeprecationWarning" | grep -v "^Hook registry" | grep -v "^Loaded cached" || true)
      fi
    fi

    # SESSION_NARRATIVE already computed — used as fallback if Gemini empty (no extra step)

  fi

  # AUTO-SAVE: Create session log with narrative
  {
    echo "# Session Log: ${OWNER}/${CLIENT}/${DOMAIN}/${REPO}"
    echo ""
    echo "**Feature:** ${FEATURE}"
    echo "**Generated:** $(TZ='America/New_York' date '+%Y-%m-%d %H:%M:%S %Z')"
    echo "**Trigger:** ${TRIGGER} compaction"
    echo "**Previous Log:** ${PREV_LOG_NAME}"
    echo ""
    echo "---"
    echo ""
    echo "## TAXONOMY"
    echo ""
    echo "| Level | Value |"
    echo "|-------|-------|"
    echo "| Owner | ${OWNER} |"
    echo "| Client | ${CLIENT} |"
    echo "| Domain | ${DOMAIN} |"
    echo "| Repo | ${REPO} |"
    echo "| Feature | ${FEATURE} |"
    echo "| Transcript | \`${TRANSCRIPT_UUID}\` |"
    echo ""
    echo "---"
    echo ""
    echo "## TRANSCRIPT HISTORY"
    echo ""
    echo "**Cumulative list of all transcripts for this feature. Use to find raw conversations if Gemini missed details.**"
    echo ""
    echo "| Date | Session | Transcript UUID | Size |"
    echo "|------|---------|-----------------|------|"
    # Include previous history
    if [ -n "$TRANSCRIPT_HISTORY" ]; then
      echo "$TRANSCRIPT_HISTORY"
    fi
    # Add current transcript
    echo "| $(TZ='America/New_York' date '+%Y-%m-%d %H:%M') | v${NEXT_VERSION} | \`${TRANSCRIPT_UUID}\` | ${TRANSCRIPT_SIZE} |"
    echo ""
    echo "**To read a transcript:** \`cat ~/.claude/projects/$(printf '%s' "$HOME" | sed 's|/|-|g' | sed 's/^-//')/<UUID>.jsonl | jq .\`"
    echo ""
    echo "---"
    echo ""
    if [ -n "$GEMINI_SUMMARY" ]; then
      echo "## 🧠 GEMINI CONTEXT SUMMARY"
      echo ""
      echo "**Auto-generated by Gemini Pro before compaction. This IS your memory.**"
      echo "**Full transcript available at:** \`$TRANSCRIPT_UUID.jsonl\`"
      echo ""
      echo "$GEMINI_SUMMARY"
    else
      echo "## 📋 SESSION NARRATIVE (jq fallback — Gemini unavailable)"
      echo ""
      echo "**Auto-extracted from transcript. Gemini summarization failed or returned empty.**"
      echo "**Full detail available in transcript:** \`$TRANSCRIPT_UUID.jsonl\`"
      echo ""
      if [ -n "$SESSION_NARRATIVE" ]; then
        echo "$SESSION_NARRATIVE"
      else
        echo "⚠️ Both Gemini and jq extraction failed. Check transcript manually:"
        echo "\`$TRANSCRIPT_PATH\`"
      fi
    fi
    echo ""
    echo "---"
    echo ""
    echo "## CONTEXT COMPACTION OCCURRED"
    echo ""
    echo "This log was auto-generated by the PreCompact hook."
    echo ""
    echo "### Transcript Location"
    echo "\`$TRANSCRIPT_PATH\`"
    echo ""
    echo "### Working Directory"
    echo "\`$PROJECT_DIR\`"
    echo ""
    echo "---"
    echo ""
    echo "## SESSION ACTIVITY LOG"
    echo ""
    echo "| Time | Action | Outcome |"
    echo "|------|--------|---------|"
    if [ -n "$GEMINI_SUMMARY" ]; then
      echo "| $(TZ='America/New_York' date '+%H:%M') | PreCompact auto-save + Gemini summary | Created ${NEW_LOG##*/} |"
    else
      echo "| $(TZ='America/New_York' date '+%H:%M') | PreCompact auto-save + jq narrative (Gemini fallback) | Created ${NEW_LOG##*/} |"
    fi
    echo ""
    echo "---"
    echo ""
    echo "## INSTRUCTIONS FOR NEXT SESSION"
    echo ""
    echo "1. Read the SESSION NARRATIVE above — that's your memory"
    echo "2. Check the previous log (${PREV_LOG_NAME}) for additional history"
    echo "3. **Need more detail?** Use TRANSCRIPT HISTORY to \`cat\` the raw .jsonl"
    echo "4. Continue from where you left off"
    echo ""
  } > "$NEW_LOG"

  # Git commit the auto-save (find the git repo that contains the log)
  LOG_GIT_DIR=$(git -C "$(dirname "$NEW_LOG")" rev-parse --show-toplevel 2>/dev/null)
  if [ -n "$LOG_GIT_DIR" ]; then
    cd "$LOG_GIT_DIR"

    if [ "$TRIGGER" = "token-gate" ]; then
      # BACKGROUND RUN (token-gate): skip the index dance to prevent a race
      # condition where git-reset wipes active staged files between PostToolUse
      # auto-stages and the re-stage loop. Just commit the log directly — any
      # other staged files will be included, which is acceptable since they were
      # auto-staged by hooks and represent Claude's own in-progress edits.
      git add "$NEW_LOG" 2>/dev/null
      git commit -m "Auto-save: TokenGate session log ($(TZ='America/New_York' date '+%Y-%m-%d %H:%M'))" 2>/dev/null
    else
      # SYNCHRONOUS RUN (native PreCompact): full isolation — commit only the
      # session log, not unrelated staged files. Safe because no concurrent
      # PostToolUse can fire while PreCompact is running synchronously.
      PREV_STAGED=$(git diff --cached --name-only 2>/dev/null)
      if [ -n "$PREV_STAGED" ]; then
        git reset HEAD --quiet 2>/dev/null
      fi
      git add "$NEW_LOG" 2>/dev/null
      git commit -m "Auto-save: PreCompact session log ($(TZ='America/New_York' date '+%Y-%m-%d %H:%M'))" 2>/dev/null
      # Re-stage previously staged files (while/read handles spaces in filenames)
      if [ -n "$PREV_STAGED" ]; then
        while IFS= read -r _staged_file; do
          [ -n "$_staged_file" ] && git add "$_staged_file" 2>/dev/null
        done <<< "$PREV_STAGED"
      fi
    fi
  fi

  # ClickUp: Update active task with session progress (background, non-blocking)
  # Gate: only fires when ENABLE_CLICKUP=1 is set (prevents errors for users without ClickUp)
  if [ "${ENABLE_CLICKUP:-0}" = "1" ]; then
  (
    CLICKUP_API_SH="$HOME/.claude/hooks/clickup-api.sh"
    if [ -f "$CLICKUP_API_SH" ]; then
      source "$CLICKUP_API_SH"
      CU_PROJECT=$(clickup_repo_to_project "$PROJECT_DIR")
      if [ -n "$CU_PROJECT" ]; then
        CU_LIST_ID=$(clickup_get_list_id "$CU_PROJECT")
        if [ -n "$CU_LIST_ID" ]; then
          GIT_BRANCH=$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null || echo "$FEATURE")
          CU_TASK_ID=$(clickup_find_task "$CU_LIST_ID" "$GIT_BRANCH")
          if [ -n "$CU_TASK_ID" ]; then
            # Update session_log custom field with new log path
            clickup_set_custom_field "$CU_TASK_ID" "$(clickup_get_field_id 'session_log')" "$NEW_LOG"
            # Add compaction comment
            SUMMARY_PREVIEW=""
            if [ -n "$GEMINI_SUMMARY" ]; then
              SUMMARY_PREVIEW=$(echo "$GEMINI_SUMMARY" | head -5 | tr '\n' ' ' | cut -c1-200)
            fi
            clickup_add_comment "$CU_TASK_ID" "Session compacted (v${NEXT_VERSION}). Log: ${NEW_LOG##*/}. Summary: ${SUMMARY_PREVIEW}..."
          fi
        fi
      fi
    fi
  ) &
  fi

  # Echo visible message to stderr (shows in UI)
  echo "📝 Session log saved: ${NEW_LOG##*/}" >&2
  echo "   Full path: $NEW_LOG" >&2

  # Return context telling Claude what happened
  jq -n --arg log "$NEW_LOG" --arg tax "${OWNER}/${CLIENT}/${DOMAIN}/${REPO}" '{
    "hookSpecificOutput": {
      "hookEventName": "PreCompact",
      "additionalContext": ("AUTO-SAVED session log before compaction:\n" + $log + "\nTaxonomy: " + $tax)
    }
  }'
fi

exit 0
