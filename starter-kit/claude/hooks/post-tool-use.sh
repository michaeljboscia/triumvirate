#!/bin/bash
# IRON LAW ENFORCEMENT: Auto-stage changes + log to session log
#
# Policy:
#   - After Edit/Write → git add (staged, experimental)
#   - After successful test → git commit (blessed)
#   - After git commit → update session log
#   - Everything logged to session log

INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty')
CWD=$(echo "$INPUT" | jq -r '.cwd // empty')

# Source shared session log finder
source "$HOME/.claude/hooks/_find-session-log.sh" 2>/dev/null
TIMESTAMP=$(TZ='America/New_York' date '+%H:%M')
WALL_TIME=$(TZ='America/New_York' date '+%H:%M %Z')
EMITTED=""  # Track if we already emitted JSON output

# Derive PROJECT_DIR from file path (smarter than just cwd)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')
PROJECT_AMBIGUOUS=""

if [ -n "$FILE_PATH" ] && [ -f "$FILE_PATH" ]; then
    # Find git root from file path
    PROJECT_DIR=$(git -C "$(dirname "$FILE_PATH")" rev-parse --show-toplevel 2>/dev/null)
elif [ -n "$FILE_PATH" ]; then
    # File path provided but file doesn't exist yet (new file)
    PROJECT_DIR=$(git -C "$(dirname "$FILE_PATH")" rev-parse --show-toplevel 2>/dev/null)
fi

# Fall back to cwd if no file path
if [ -z "$PROJECT_DIR" ]; then
    PROJECT_DIR="$CWD"
    # Check if cwd is a git repo
    if ! git -C "$PROJECT_DIR" rev-parse --show-toplevel > /dev/null 2>&1; then
        PROJECT_AMBIGUOUS="true"
    fi
fi

# Check if taxonomy exists, if not flag as ambiguous
TAXONOMY_FILE="$PROJECT_DIR/.claude/taxonomy.json"
if [ ! -f "$TAXONOMY_FILE" ] && [ -z "$PROJECT_AMBIGUOUS" ]; then
    PROJECT_AMBIGUOUS="no_taxonomy"
fi

# Find session log — checks AI memory first, then project-local
SESSION_LOG=""
if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then
    _find_session_log "$PROJECT_DIR"
fi

# Function to append to session log
append_to_log() {
    local message="$1"
    if [ -n "$SESSION_LOG" ] && [ -f "$SESSION_LOG" ]; then
        # Append to activity log section
        echo "| $TIMESTAMP | $message |" >> "$SESSION_LOG"
    fi
}

# Function to get current git state for correlation
get_git_state() {
    local dir="$1"
    cd "$dir" 2>/dev/null || return
    if git rev-parse --git-dir > /dev/null 2>&1; then
        local commit_short=$(git rev-parse --short HEAD 2>/dev/null || echo "none")
        local staged=$(git diff --cached --name-only 2>/dev/null | wc -l | tr -d ' ')
        local unstaged=$(git diff --name-only 2>/dev/null | wc -l | tr -d ' ')
        echo "@${commit_short} staged:${staged} unstaged:${unstaged}"
    fi
}

# Function to git add a file
git_add_file() {
    local file_path="$1"
    if [ -n "$file_path" ] && [ -f "$file_path" ]; then
        local file_dir=$(dirname "$file_path")
        local repo_root=$(git -C "$file_dir" rev-parse --show-toplevel 2>/dev/null)
        if [ -n "$repo_root" ]; then
            # Get line changes BEFORE staging (diff against HEAD)
            local changes=$(git -C "$repo_root" diff --numstat "$file_path" 2>/dev/null | awk '{print "+"$1"/-"$2}')
            if [ -z "$changes" ]; then
                # New file - count lines
                local lines=$(wc -l < "$file_path" 2>/dev/null | tr -d ' ')
                changes="+${lines}/-0 (new)"
            fi

            git -C "$repo_root" add "$file_path" 2>/dev/null
            local filename=$(basename "$file_path")
            # Update SESSION_LOG to point to correct repo (AI memory → project-local)
            _find_session_log "$repo_root"
            append_to_log "Staged: $filename ($changes) | ✓ |"
        fi
    fi
}

case "$TOOL_NAME" in
    "Edit"|"Write")
        # Auto-stage the file
        FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')
        if [ -n "$FILE_PATH" ]; then
            git_add_file "$FILE_PATH"
        fi
        ;;

    "Bash")
        COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty')
        EXIT_CODE=$(echo "$INPUT" | jq -r '.tool_response.exit_code // 0')

        # Check if this was a git commit
        if echo "$COMMAND" | grep -q "git commit"; then
            # Get the actual commit hash and message from git (more reliable than parsing command)
            cd "$PROJECT_DIR" 2>/dev/null
            COMMIT_HASH=$(git rev-parse --short HEAD 2>/dev/null)
            COMMIT_MSG=$(git log -1 --pretty=format:'%s' 2>/dev/null | cut -c1-60)
            if [ -z "$COMMIT_HASH" ]; then
                COMMIT_HASH="???"
            fi
            if [ -z "$COMMIT_MSG" ]; then
                COMMIT_MSG="(no message)"
            fi
            append_to_log "Git commit $COMMIT_HASH: $COMMIT_MSG | COMMITTED |"

            # ClickUp integration: add commit comment to matching task (background, non-blocking)
            # Gate: only fires when ENABLE_CLICKUP=1 is set (prevents errors for users without ClickUp)
            if [ "${ENABLE_CLICKUP:-0}" = "1" ]; then
            (
              CLICKUP_API_SH="$HOME/.claude/hooks/clickup-api.sh"
              if [ -f "$CLICKUP_API_SH" ]; then
                source "$CLICKUP_API_SH"
                REPO_ROOT=$(git -C "$PROJECT_DIR" rev-parse --show-toplevel 2>/dev/null)
                CU_PROJECT=$(clickup_repo_to_project "$REPO_ROOT")
                if [ -n "$CU_PROJECT" ]; then
                  CU_LIST_ID=$(clickup_get_list_id "$CU_PROJECT")
                  if [ -n "$CU_LIST_ID" ]; then
                    GIT_BRANCH=$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)
                    CU_TASK_ID=$(clickup_find_task "$CU_LIST_ID" "$GIT_BRANCH")
                    if [ -z "$CU_TASK_ID" ]; then
                      # No task matches the branch — create one
                      CU_TASK_ID=$(clickup_create_task "$CU_LIST_ID" "$GIT_BRANCH" "Auto-created from git activity on branch $GIT_BRANCH")
                      # Set custom fields
                      if [ -n "$CU_TASK_ID" ]; then
                        clickup_set_custom_field "$CU_TASK_ID" "$(clickup_get_field_id 'git_branch')" "$GIT_BRANCH"
                        clickup_set_custom_field "$CU_TASK_ID" "$(clickup_get_field_id 'auto_updated')" "true"
                        REMOTE_URL=$(git -C "$PROJECT_DIR" remote get-url origin 2>/dev/null)
                        if [ -n "$REMOTE_URL" ]; then
                          clickup_set_custom_field "$CU_TASK_ID" "$(clickup_get_field_id 'git_repo')" "$REMOTE_URL"
                        fi
                      fi
                    fi
                    # Add commit as comment
                    if [ -n "$CU_TASK_ID" ]; then
                      clickup_add_comment "$CU_TASK_ID" "Commit $COMMIT_HASH: $COMMIT_MSG ($(TZ='America/New_York' date '+%H:%M %Z'))"
                      # Auto-set status to "in progress" if still "to do"
                      clickup_update_task "$CU_TASK_ID" '{"status":"in progress"}'
                    fi
                  fi
                fi
              fi
            ) &
            fi

            # Remind Claude to update the session log with narrative
            jq -n --arg hash "$COMMIT_HASH" --arg wt "$WALL_TIME" '{
              "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": "🕐 " + $wt + "\n✏️ COMMIT MADE (" + $hash + "). Update the session log with:\n- What was accomplished?\n- Why did we do it?\n- What'"'"'s next?\n\nKeep the narrative current - don'"'"'t wait for compaction."
              }
            }'
            EMITTED="true"
        fi

        # Check if this was a test command
        if echo "$COMMAND" | grep -qiE "(pytest|npm test|yarn test|go test|cargo test|make test|jest|mocha|rspec)"; then
            if [ "$EXIT_CODE" = "0" ]; then
                # Tests passed - this is a commit point!
                cd "$PROJECT_DIR" 2>/dev/null || exit 0
                if git rev-parse --git-dir > /dev/null 2>&1; then
                    # Check if there are staged changes
                    if ! git diff --cached --quiet 2>/dev/null; then
                        # Get list of staged files
                        STAGED=$(git diff --cached --name-only | tr '\n' ', ' | sed 's/,$//')
                        git commit -m "Tests pass: auto-commit staged changes

Files: $STAGED

Co-Authored-By: Claude <noreply@anthropic.com>" 2>/dev/null

                        append_to_log "✅ Tests passed → Auto-committed: $STAGED | BLESSED |"

                        # Also stage+commit the session log itself
                        if [ -n "$SESSION_LOG" ]; then
                            git add "$SESSION_LOG" 2>/dev/null
                            git commit -m "Update session log after test pass" --no-verify 2>/dev/null
                        fi
                    fi
                fi
            else
                # Tests FAILED - log what didn't work WITH GIT STATE
                GIT_STATE=$(get_git_state "$CWD")
                ERROR_SNIPPET=$(echo "$INPUT" | jq -r '.tool_response.stdout // .tool_response.stderr // empty' | head -3 | tr '\n' ' ' | cut -c1-60)
                append_to_log "❌ Tests FAILED $GIT_STATE | exit $EXIT_CODE: $ERROR_SNIPPET | NEEDS FIX |"
            fi
        fi

        # Check for other failed commands (non-zero exit)
        if [ "$EXIT_CODE" != "0" ] && [ "$EXIT_CODE" != "" ]; then
            if ! echo "$COMMAND" | grep -qiE "(pytest|npm test|yarn test|go test|cargo test|make test|jest|mocha|rspec)"; then
                # Non-test command failed - log with git state for correlation
                GIT_STATE=$(get_git_state "$CWD")
                CMD_SHORT=$(echo "$COMMAND" | cut -c1-30)
                append_to_log "⚠️ Failed $GIT_STATE | $CMD_SHORT... (exit $EXIT_CODE) | ERROR |"
            fi
        fi
        ;;
esac

# Taxonomy warning for file-editing tools (Edit/Write), not Bash.
# NOTE: PostToolUse fires AFTER the write — we cannot undo it here.
# The real gate lives in pre-tool-use-artifact-guard.sh (PreToolUse).
# We emit an additionalContext warning so Claude knows to pause and fix it.
if [ "$TOOL_NAME" = "Edit" ] || [ "$TOOL_NAME" = "Write" ]; then
    if [ "$PROJECT_AMBIGUOUS" = "true" ]; then
        jq -n --arg wt "$WALL_TIME" '{
          "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": ("🕐 " + $wt + " — ⚠️ TAXONOMY WARNING: Project is ambiguous (not a git repo). The write completed, but you SHOULD ask the user: What project are we working on? What client/domain does this belong to? Create .claude/taxonomy.json before the next edit.")
          }
        }'
        EMITTED="true"
    elif [ "$PROJECT_AMBIGUOUS" = "no_taxonomy" ]; then
        # Get repo name from git remote or fallback to directory basename
        REPO_NAME=$(git -C "$PROJECT_DIR" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
        if [ -z "$REPO_NAME" ]; then
          REPO_NAME=$(basename "$PROJECT_DIR")
        fi
        jq -n --arg repo "$REPO_NAME" --arg dir "$PROJECT_DIR" --arg wt "$WALL_TIME" '{
          "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": ("🕐 " + $wt + " — ⚠️ TAXONOMY WARNING: Project " + $repo + " has no taxonomy. The write completed, but you SHOULD ask: What client and domain does " + $repo + " belong to? Then CREATE " + $dir + "/.claude/taxonomy.json before the next edit.")
          }
        }'
        EMITTED="true"
    fi
fi

# Emit wall time if nothing else was emitted
if [ -z "$EMITTED" ]; then
    jq -n --arg wt "$WALL_TIME" '{
      "hookSpecificOutput": {
        "hookEventName": "PostToolUse",
        "additionalContext": "🕐 " + $wt
      }
    }'
fi

exit 0
