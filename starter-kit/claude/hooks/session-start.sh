#!/bin/bash
# IRON LAW: Project Picker + Session Recovery
#
# When in HOME: Scan for projects, present picker with recent sessions
# When in a project: Read latest session log (existing behavior)

INPUT=$(cat)
PROJECT_DIR=$(echo "$INPUT" | jq -r '.cwd // empty')
# Claude Code sends source:"compact" in the JSON for compact events — use it directly.
SOURCE=$(echo "$INPUT" | jq -r '.reason // .source // .trigger // "startup"')
HOME_DIR="$HOME"
WALL_TIME=$(TZ='America/New_York' date '+%Y-%m-%d %H:%M %Z')

# Source credentials vault (auto-export all vars to child processes)
if [ -f "$HOME/.claude/.env" ]; then
  set -a
  source "$HOME/.claude/.env"
  set +a
fi

# Source shared session log finder
source "$HOME/.claude/hooks/_find-session-log.sh" 2>/dev/null

# --- Skip picker on resume/compact (already have conversation context) ---
if [ "$SOURCE" = "resume" ] || [ "$SOURCE" = "compact" ]; then
  LESSONS_MAX_CHARS=6000
  ADDED_LESSON_PATHS="|"
  LESSONS_CTX=""

  # For compact starts, avoid duplicating large lessons payload here.
  # post-compact-recovery.sh injects the recovery narrative + lessons.
  if [ "$SOURCE" != "compact" ]; then
    append_lessons() {
      local label="$1"
      local path="$2"
      [ -z "$path" ] && return
      [ ! -f "$path" ] && return

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

      LESSONS_CTX="$LESSONS_CTX

---
$label ($path):
$body"
    }

    append_lessons "📚 GLOBAL LESSONS" "$HOME_DIR/.claude/lessons.md"
    if [ -n "$PROJECT_DIR" ]; then
      append_lessons "📁 PROJECT LESSONS" "$PROJECT_DIR/lessons.md"
    fi
  fi

  if [ "$SOURCE" = "compact" ]; then
    # On compact: emit NO JSON at all. post-compact-recovery.sh owns context injection.
    # Even emitting hookSpecificOutput without additionalContext may clobber recovery's
    # context under strict last-writer-wins (Codex review finding). Silent exit is safest.
    exit 0
  else
    # resume/startup: inject wall time + lessons as before
    jq -n --arg wt "$WALL_TIME" --arg src "$SOURCE" --arg lessons "$LESSONS_CTX" '{
      "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": ("🕐 Wall time: " + $wt + "\nSession " + $src + "d — continuing where you left off." + $lessons)
      }
    }'
  fi
  exit 0
fi

# ===================================================================
# PHASE 1: HOME DIRECTORY → PROJECT PICKER
# ===================================================================
if [ "$PROJECT_DIR" = "$HOME_DIR" ] || [ "$PROJECT_DIR" = "$HOME_DIR/" ]; then

  # --- Load project registry (maps repo names → local paths) ---
  REGISTRY_FILE="$HOME_DIR/.claude/project-registry.json"
  REGISTRY_REPO_NAMES=""  # pipe-delimited GitHub repo names that have local paths

  # --- Scan for projects ---
  PROJECTS=""
  PROJECT_COUNT=0
  SCANNED_DIRS=""  # For dedup

  scan_dir() {
    local dir="$1"
    local display="$2"

    # Dedup check
    case "$SCANNED_DIRS" in *"|${dir}|"*) return ;; esac
    SCANNED_DIRS="${SCANNED_DIRS}|${dir}|"

    # Must have .claude/taxonomy.json OR .git to count as a project
    local has_tax=false
    local has_git=false
    [ -f "$dir/.claude/taxonomy.json" ] && has_tax=true
    [ -d "$dir/.git" ] && has_git=true

    if ! $has_tax && ! $has_git; then
      return
    fi

    PROJECT_COUNT=$((PROJECT_COUNT + 1))

    # Read taxonomy if available
    local client="" domain="" repo="" feature=""
    if $has_tax; then
      client=$(jq -r '.client // ""' "$dir/.claude/taxonomy.json" 2>/dev/null)
      domain=$(jq -r '.domain // ""' "$dir/.claude/taxonomy.json" 2>/dev/null)
      repo=$(jq -r '.repo // ""' "$dir/.claude/taxonomy.json" 2>/dev/null)
      feature=$(jq -r '.feature // ""' "$dir/.claude/taxonomy.json" 2>/dev/null)
      # Clean up null strings from jq
      [ "$client" = "null" ] && client=""
      [ "$domain" = "null" ] && domain=""
      [ "$repo" = "null" ] && repo=""
      [ "$feature" = "null" ] && feature=""
    fi

    # Fallback repo name from git remote or directory name
    if [ -z "$repo" ]; then
      repo=$(git -C "$dir" remote get-url origin 2>/dev/null | sed 's/.*\///' | sed 's/\.git$//')
    fi
    if [ -z "$repo" ]; then
      repo=$(basename "$dir")
    fi

    # Last session log time (check AI memory first, then project-local)
    local last_time="no sessions"
    local last_log=""
    local _ai_mem="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
    if [ -d "$_ai_mem/$repo" ]; then
      last_log=$(ls -t "$_ai_mem/$repo/"*_v*.md 2>/dev/null | head -1)
    fi
    if [ -z "$last_log" ]; then
      last_log=$(ls -t "$dir/session-logs/"*_v*.md 2>/dev/null | head -1)
    fi
    if [ -n "$last_log" ] && [ -f "$last_log" ]; then
      last_time=$(stat -f '%Sm' -t '%b %d %H:%M' "$last_log" 2>/dev/null || echo "?")
    fi

    # Build display line
    local label="$repo"
    if [ -n "$client" ] && [ -n "$domain" ]; then
      label="${repo} (${client}/${domain})"
    fi

    local config_status=""
    if ! $has_tax; then
      config_status=" ⚠️ no taxonomy"
    fi

    PROJECTS="${PROJECTS}  ${PROJECT_COUNT}. **${label}**${config_status} → \`${display}\`\n     Last: ${last_time}"
    if [ -n "$feature" ]; then
      PROJECTS="${PROJECTS} | Feature: ${feature}"
    fi
    PROJECTS="${PROJECTS}\n"
  }

  # Scan ~/projects/*/ first (canonical project location)
  for dir in "$HOME_DIR"/projects/*/; do
    dir="${dir%/}"
    [ ! -d "$dir" ] && continue
    scan_dir "$dir" "~/projects/$(basename "$dir")"
  done

  # Scan ~/*/ (top-level dirs, skip known non-project dirs)
  for dir in "$HOME_DIR"/*/; do
    dir="${dir%/}"
    [ ! -d "$dir" ] && continue
    case "$(basename "$dir")" in
      projects|Downloads|Documents|Desktop|Library|Pictures|Music|Movies|Public|Applications|node_modules|session-logs) continue ;;
      "My Drive"|"Google Drive") continue ;;
    esac
    # Skip hidden dirs (except we handle .claude separately)
    case "$(basename "$dir")" in
      .*) continue ;;
    esac
    scan_dir "$dir" "~/$(basename "$dir")"
  done

  # Include ~/.claude as the config repo
  if [ -d "$HOME_DIR/.claude/.git" ]; then
    scan_dir "$HOME_DIR/.claude" "~/.claude"
  fi

  # --- Scan registry entries (catches mismatched names + Google Drive) ---
  if [ -f "$REGISTRY_FILE" ]; then
    # Read all key-value pairs (skip keys starting with _)
    while IFS=$'\t' read -r repo_name local_path; do
      [ -z "$repo_name" ] && continue
      # Expand ~ to $HOME
      local_path="${local_path/#\~/$HOME_DIR}"
      if [ -d "$local_path" ]; then
        # Re-collapse for display
        display_path=$(echo "$local_path" | sed "s|$HOME_DIR|~|")
        scan_dir "$local_path" "$display_path"
        # Track this repo name so GitHub dedup knows it's local
        REGISTRY_REPO_NAMES="${REGISTRY_REPO_NAMES}|${repo_name}|"
      fi
    done < <(jq -r 'to_entries[] | select(.key | startswith("_") | not) | "\(.key)\t\(.value)"' "$REGISTRY_FILE" 2>/dev/null)
  fi

  # --- Scan GitHub repos (catches remote-only projects like tellus) ---
  REMOTE_PROJECTS=""
  REMOTE_COUNT=0
  if command -v gh &>/dev/null; then
    # macOS has no `timeout` command — use background + sleep + kill pattern
    GH_REPOS=""
    GH_TMPFILE="$(mktemp 2>/dev/null || echo "/tmp/_gh_repos_$$")"
    GH_OWNER="$(gh api user -q .login 2>/dev/null || git config --global user.name 2>/dev/null | tr ' ' '-' | tr '[:upper:]' '[:lower:]' || echo "")"
    gh repo list "$GH_OWNER" --limit 50 --json name,description,updatedAt \
      --jq '.[] | "\(.name)\t\(.description // "")\t\(.updatedAt)"' > "$GH_TMPFILE" 2>/dev/null &
    GH_PID=$!
    # Wait up to 5 seconds
    for i in 1 2 3 4 5; do
      if ! kill -0 "$GH_PID" 2>/dev/null; then break; fi
      sleep 1
    done
    # Kill if still running (network timeout)
    kill "$GH_PID" 2>/dev/null; wait "$GH_PID" 2>/dev/null
    GH_REPOS=$(cat "$GH_TMPFILE" 2>/dev/null || true)
    rm -f "$GH_TMPFILE" 2>/dev/null

    if [ -n "$GH_REPOS" ]; then
      while IFS=$'\t' read -r repo_name repo_desc repo_updated; do
        [ -z "$repo_name" ] && continue

        # Skip if already found locally (dedup: check SCANNED_DIRS, registry, common paths)
        FOUND_LOCAL=false
        # Check scanned dirs list (pipe-delimited paths)
        case "$SCANNED_DIRS" in
          *"|${HOME_DIR}/${repo_name}|"*) FOUND_LOCAL=true ;;
          *"|${HOME_DIR}/projects/${repo_name}|"*) FOUND_LOCAL=true ;;
          *"|${HOME_DIR}/.${repo_name}|"*) FOUND_LOCAL=true ;;
        esac
        # Check registry (handles name mismatches like claude-config → ~/.claude)
        if ! $FOUND_LOCAL; then
          case "$REGISTRY_REPO_NAMES" in
            *"|${repo_name}|"*) FOUND_LOCAL=true ;;
          esac
        fi
        # Also check directory existence directly (catches name variations)
        if ! $FOUND_LOCAL; then
          for local_dir in "$HOME_DIR/$repo_name" "$HOME_DIR/projects/$repo_name" \
                           "$HOME_DIR/.$repo_name" "$HOME_DIR/projects/$repo_name"*; do
            if [ -d "$local_dir" ]; then
              FOUND_LOCAL=true
              break
            fi
          done
        fi

        if ! $FOUND_LOCAL; then
          REMOTE_COUNT=$((REMOTE_COUNT + 1))
          # Format updated time (strip time portion)
          updated_short=$(echo "$repo_updated" | cut -dT -f1)
          line="  R${REMOTE_COUNT}. **${repo_name}** (GitHub — not cloned)"
          # Only show description if it's actual text, not a timestamp
          if [ -n "$repo_desc" ] && ! echo "$repo_desc" | grep -qE '^[0-9]{4}-[0-9]{2}-[0-9]{2}'; then
            line="${line}\n     _${repo_desc}_"
          fi
          line="${line}\n     Updated: ${updated_short} | Clone: \`gh repo clone ${GH_OWNER}/${repo_name} ~/projects/${repo_name}\`"
          REMOTE_PROJECTS="${REMOTE_PROJECTS}${line}\n"
        fi
      done <<< "$GH_REPOS"
    fi
  fi

  # --- Collect recent sessions across ALL projects ---
  RECENT=""
  ALL_SESSION_DIRS=()

  # Gather all session-logs directories from scanned projects + known locations
  for dir in "$HOME_DIR"/projects/*/ "$HOME_DIR"/*/; do
    dir="${dir%/}"
    [ -d "$dir/session-logs" ] && ALL_SESSION_DIRS+=("$dir/session-logs")
  done
  [ -d "$HOME_DIR/.claude/session-logs" ] && ALL_SESSION_DIRS+=("$HOME_DIR/.claude/session-logs")
  # Include orphan ~/session-logs/ for migration visibility
  [ -d "$HOME_DIR/session-logs" ] && ALL_SESSION_DIRS+=("$HOME_DIR/session-logs")
  # Include AI memory central store (all repo subdirs)
  _AI_MEM="${AI_MEMORY_DIR:-$HOME_DIR/.ai-memory}"
  if [ -d "$_AI_MEM" ]; then
    for _memdir in "$_AI_MEM"/*/; do
      [ -d "$_memdir" ] && ALL_SESSION_DIRS+=("${_memdir%/}")
    done
  fi
  # Include registry-resolved paths (Google Drive, mismatched names)
  # Dedup: only add if not already in the array (check via SCANNED_DIRS)
  if [ -f "$REGISTRY_FILE" ]; then
    while IFS=$'\t' read -r _rname rpath; do
      [ -z "$_rname" ] && continue
      rpath="${rpath/#\~/$HOME_DIR}"
      if [ -d "$rpath/session-logs" ]; then
        # Skip if this dir was already scanned by the glob loops above
        case "$SCANNED_DIRS" in *"|${rpath}|"*) continue ;; esac
        ALL_SESSION_DIRS+=("$rpath/session-logs")
      fi
    done < <(jq -r 'to_entries[] | select(.key | startswith("_") | not) | "\(.key)\t\(.value)"' "$REGISTRY_FILE" 2>/dev/null)
  fi

  if [ ${#ALL_SESSION_DIRS[@]} -gt 0 ]; then
    # Build file list safely using find per directory, sort by mtime
    RECENT_LOGS=""
    for sdir in "${ALL_SESSION_DIRS[@]}"; do
      RECENT_LOGS="${RECENT_LOGS}$(find "$sdir" -maxdepth 1 -name '*_v*_*.md' -type f 2>/dev/null)"$'\n'
    done
    # Sort by mtime (newest first) and take top 8
    RECENT_LOGS=$(echo "$RECENT_LOGS" | xargs ls -t 2>/dev/null | head -8)
    if [ -n "$RECENT_LOGS" ]; then
      RECENT="\n📋 **Recent sessions (pick up where you left off):**\n"
      while IFS= read -r log; do
        [ -z "$log" ] && continue
        LOG_NAME=$(basename "$log")
        LOG_TIME=$(stat -f '%Sm' -t '%b %d %H:%M' "$log" 2>/dev/null || echo "?")
        LOG_PROJECT_DIR=$(dirname "$(dirname "$log")")
        DISPLAY_DIR=$(echo "$LOG_PROJECT_DIR" | sed "s|$HOME_DIR|~|")

        # Parse taxonomy from filename: owner--client_domain_repo_feature_date_vN_agent.md
        REPO_NAME=$(echo "$LOG_NAME" | sed 's/[^-]*--[^_]*_[^_]*_//' | sed 's/_[^_]*_[0-9].*$//')
        FEATURE_NAME=$(echo "$LOG_NAME" | sed 's/[^-]*--[^_]*_[^_]*_[^_]*_//' | sed 's/_[0-9].*$//')

        RECENT="${RECENT}  - **${REPO_NAME}** / ${FEATURE_NAME} — ${LOG_TIME} → \`${DISPLAY_DIR}\`\n"
      done <<< "$RECENT_LOGS"
    fi
  fi

  # --- Build picker output ---
  REMOTE_SECTION=""
  if [ -n "$REMOTE_PROJECTS" ]; then
    REMOTE_SECTION="\n**GitHub repos (not cloned locally):**\n$(printf '%b' "$REMOTE_PROJECTS")"
  fi

  PICKER_OUTPUT="🏠 **PROJECT PICKER** — You started from the home directory.
🕐 Wall time: ${WALL_TIME}

**Local projects:**
$(printf '%b' "$PROJECTS")${REMOTE_SECTION}$(printf '%b' "$RECENT")
🆕 **Or start a NEW project** — just tell me what you're working on.

---
**INSTRUCTIONS FOR CLAUDE:**
1. Present the FULL project list above as a NUMBERED LIST directly in chat text.
   Do NOT use AskUserQuestion — it only supports 4 options and truncates the list.
   Just print the list and ask \"What are we working on?\" as plain text.
   The user will type a number, name, or describe what they want.
2. When user picks a LOCAL project: run \`cd <path>\` then read the latest session log in \`<path>/session-logs/\`
3. When user picks a GITHUB-ONLY project: clone it first (\`gh repo clone <username>/<name> ~/projects/<name>\`), set up taxonomy, then \`cd\` there
4. When user wants a NEW project:
   a. Ask: project name, what it's for
   b. Create: \`mkdir -p ~/projects/<name>/.claude\` and \`mkdir -p ~/projects/<name>/session-logs\`
   c. Create: \`.claude/taxonomy.json\` with owner/client/domain/repo/feature
   d. Run: \`cd ~/projects/<name>\` and \`git init\`
5. Do NOT start real work or create session logs until you are in a project directory
6. Once in a project, read the latest session log to restore context"

  jq -n --arg ctx "$PICKER_OUTPUT" '{
    "hookSpecificOutput": {
      "hookEventName": "SessionStart",
      "additionalContext": $ctx
    }
  }'
  exit 0
fi

# ===================================================================
# PHASE 2: IN A PROJECT DIRECTORY — NORMAL SESSION LOG RECOVERY
# ===================================================================
if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then
  # Get taxonomy from .claude/taxonomy.json or use defaults
  TAXONOMY_FILE="$PROJECT_DIR/.claude/taxonomy.json"

  if [ -f "$TAXONOMY_FILE" ]; then
    OWNER=$(jq -r '.owner // "unknown"' "$TAXONOMY_FILE")
    CLIENT=$(jq -r '.client // "unknown"' "$TAXONOMY_FILE")
    DOMAIN=$(jq -r '.domain // "unknown"' "$TAXONOMY_FILE")
  else
    # Defaults
    OWNER=$(git -C "$PROJECT_DIR" config user.name 2>/dev/null | tr ' ' '-' | tr '[:upper:]' '[:lower:]')
    if [ -z "$OWNER" ]; then
      OWNER="unknown"
    fi
    CLIENT="unknown"
    DOMAIN="unknown"
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

  # Find most recent session log (AI memory → project-local → legacy)
  _find_session_log "$PROJECT_DIR"
  LATEST_LOG="$SESSION_LOG"

  if [ -n "$LATEST_LOG" ] && [ -f "$LATEST_LOG" ]; then
    CONTENT=$(cat "$LATEST_LOG")
    # Inject global and project lessons
    if [ -f "$HOME_DIR/.claude/lessons.md" ]; then
      CONTENT="$CONTENT

---
📚 GLOBAL LESSONS ($HOME_DIR/.claude/lessons.md):
$(cat "$HOME_DIR/.claude/lessons.md")"
    fi
    if [ -f "$PROJECT_DIR/lessons.md" ]; then
      CONTENT="$CONTENT

---
📁 PROJECT LESSONS ($PROJECT_DIR/lessons.md):
$(cat "$PROJECT_DIR/lessons.md")"
    fi

    CONTEXT="🔔 IRON LAW: Session log found!

🕐 Wall time: $WALL_TIME
File: $LATEST_LOG
Taxonomy: ${OWNER}/${CLIENT}/${DOMAIN}/${REPO}

$CONTENT"
    jq -n --arg ctx "$CONTEXT" '{
      "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": $ctx
      }
    }'
  else
    # No session log exists - list known sessions for context
    EXISTING_SESSIONS=""
    # Check AI memory first
    _SCAN_AI_MEM="${AI_MEMORY_DIR:-$HOME/.ai-memory}"
    if [ -d "$_SCAN_AI_MEM" ]; then
      for _mdir in "$_SCAN_AI_MEM"/*/; do
        [ -d "$_mdir" ] || continue
        LOGS=$(ls -t "$_mdir"*--*_v*.md 2>/dev/null | head -3)
        if [ -n "$LOGS" ]; then
          EXISTING_SESSIONS="${EXISTING_SESSIONS}${LOGS}\n"
        fi
      done
    fi
    # Then check project-local
    for dir in ~/projects/* ~/.claude "$HOME_DIR"/*/; do
      dir="${dir%/}"
      if [ -d "$dir" ]; then
        LOGS=$(ls -t "$dir"/session-logs/*--*_v*.md "$dir"/*--*_v*.md 2>/dev/null | head -3)
        if [ -n "$LOGS" ]; then
          EXISTING_SESSIONS="${EXISTING_SESSIONS}${LOGS}\n"
        fi
      fi
    done

    if [ -n "$EXISTING_SESSIONS" ]; then
      jq -n --arg sessions "$EXISTING_SESSIONS" --arg wt "$WALL_TIME" '{
        "hookSpecificOutput": {
          "hookEventName": "SessionStart",
          "additionalContext": "🕐 Wall time: " + $wt + "\n\n🛑 NO SESSION LOG FOR THIS DIRECTORY.\n\nBefore doing ANY work, ask the user:\n\n**Are we starting NEW work or CONTINUING existing?**\n\nIf NEW:\n- What client? (gtm-machine, personal, core)\n- What domain? (infrastructure, intelligence, outreach)\n- What feature/task?\n\nIf CONTINUING, recent sessions:\n" + $sessions + "\n\nThen either create new taxonomy + session log, or cd to the existing project."
        }
      }'
    else
      jq -n --arg wt "$WALL_TIME" '{
        "hookSpecificOutput": {
          "hookEventName": "SessionStart",
          "additionalContext": "🕐 Wall time: " + $wt + "\n\n🛑 NO SESSION LOG.\n\nBefore doing ANY work, ask the user:\n\n1. What are we working on? (feature/task)\n2. What client? (gtm-machine, personal, core)\n3. What domain? (infrastructure, intelligence, outreach)\n\nThen CREATE .claude/taxonomy.json and session log with naming:\n<owner>--<client>_<domain>_<repo>_<feature>_<date>_v1.md"
        }
      }'
    fi
  fi
else
  echo '{}'
fi

exit 0
