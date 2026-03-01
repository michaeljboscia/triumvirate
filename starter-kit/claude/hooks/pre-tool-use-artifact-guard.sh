#!/bin/bash
# ============================================================================
# THE AIRLOCK — PreToolUse Enforcement
# Purpose: Snapshot every edited file before modification; enforce backup
#          requirements for Supabase DB objects (remote_strict path).
#
# Routes (determined by file type and path):
#   remote_strict      — Supabase DB SQL with DDL (deny if no fresh backup)
#   remote_best_effort — Edge functions, n8n workflows (snapshot, never deny)
#   local_copy         — All other source files (snapshot, never deny)
#   pass               — node_modules, .git, logs, tmp (no action)
#
# Backup layout:
#   ~/.claude/artifact-backups/<repo>/<class>/<YYYY>/<MM>/<DD>/
#       <timestamp>_<pid>__<filename>.gz       (compressed content)
#       <timestamp>_<pid>__<filename>.meta.json (sidecar, uncompressed)
#
# Emergency bypass: export ARTIFACT_GUARD_BYPASS=1  (always logged)
#
# macOS notes:
#   - Uses shasum (not sha256sum), stat -f %m (not stat -c %Y)
#   - Bash 3.2 compatible: no associative arrays, no mapfile
#   - No 'timeout' command on macOS; psql timeout handled via perl alarm
#
# Created: 2026-02-18
# Replaces: pre-tool-use-supabase-gate.sh
# ============================================================================

# -e intentionally omitted — handle errors explicitly.
# Non-zero hook exit crashes the hook framework.
set -uo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# PARSE INPUT
# ─────────────────────────────────────────────────────────────────────────────
INPUT="$(cat)" || INPUT=""
TOOL_NAME="$(echo "$INPUT" | jq -r '.tool_name // empty' 2>/dev/null)" || TOOL_NAME=""
FILE_PATH="$(echo "$INPUT" | jq -r '.tool_input.file_path // empty' 2>/dev/null)" || FILE_PATH=""
# Edit uses new_string; Write uses content — capture both
WRITE_TEXT="$(echo "$INPUT" | jq -r '.tool_input.new_string // .tool_input.content // empty' 2>/dev/null)" || WRITE_TEXT=""

# ─────────────────────────────────────────────────────────────────────────────
# EARLY EXITS
# ─────────────────────────────────────────────────────────────────────────────
[[ "$TOOL_NAME" != "Edit" && "$TOOL_NAME" != "Write" ]] && exit 0
[[ -z "$FILE_PATH" ]] && exit 0

# ─────────────────────────────────────────────────────────────────────────────
# EMERGENCY BYPASS
# ─────────────────────────────────────────────────────────────────────────────
if [[ "${ARTIFACT_GUARD_BYPASS:-0}" == "1" ]]; then
  LOG_FILE="$HOME/.claude/artifact-guard-logs/bypass.log"
  mkdir -p "$(dirname "$LOG_FILE")" 2>/dev/null
  printf '%s BYPASS tool=%s file=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$TOOL_NAME" "$FILE_PATH" \
    >> "$LOG_FILE" 2>/dev/null
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# SECRET SCAN — deterministic, zero-LLM, deny-before-write
# Fires on every Edit/Write unless file path is in the allowlist.
# Uses labeled matcher pattern for clear error messages.
# Patterns: high-confidence prefix signatures + generic assignment catch-all.
# ─────────────────────────────────────────────────────────────────────────────

_is_secret_allow_path() {
  local p="$1"
  case "$p" in
    # Env files — secrets live here by design
    *.env|*/.env|*/.env.*) return 0 ;;
    # Agent config docs that explicitly document credentials
    */CLAUDE.md|*/AGENTS.md|*/GEMINI.md|*/infrastructure.md) return 0 ;;
    # Named credential/secret stores
    */credentials|*/credentials.*|*/credentials/*)  return 0 ;;
    */credential|*/credential.*|*/credential/*)     return 0 ;;
    */secrets|*/secrets.*|*/secrets/*)              return 0 ;;
    # Package manager credential files
    */.npmrc|*/.pypirc) return 0 ;;
    # Cert and key files
    *.pem|*.key|*.p12|*.pfx) return 0 ;;
  esac
  return 1
}

_secret_match_label=""
_match_secret() {
  local label="$1" pattern="$2"
  if printf '%s' "$WRITE_TEXT" | grep -Eiq -- "$pattern" 2>/dev/null; then
    _secret_match_label="$label"
    return 0
  fi
  return 1
}

if [[ -n "$WRITE_TEXT" ]] && ! _is_secret_allow_path "$FILE_PATH"; then
  # High-confidence provider prefix signatures (ordered: most specific first)
  _match_secret "private_key_block"         '-----BEGIN[[:space:]]+([A-Z ]+)?PRIVATE KEY-----'          ||
  _match_secret "aws_access_key"            '(AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}'                        ||
  _match_secret "github_token"              'gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}' ||
  _match_secret "slack_token"               'xox[baprs]-[A-Za-z0-9-]{10,}'                             ||
  _match_secret "stripe_key"               '(sk|pk|rk)_(live|test)_[A-Za-z0-9]{16,}'                   ||
  _match_secret "anthropic_key"             'sk-ant-[A-Za-z0-9_-]{20,}'                                ||
  _match_secret "openai_key"                'sk-proj-[A-Za-z0-9_-]{20,}|sk-[A-Za-z0-9]{40,}'          ||
  _match_secret "google_api_key"            'AIza[0-9A-Za-z_-]{35}'                                    ||
  _match_secret "supabase_service_token"    'sbp_[A-Za-z0-9]{40,}'                                     ||
  _match_secret "jwt_bearer_token"          'eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}' ||
  # Generic assignment catch-all: api_key/secret/token/password = <16+ char value>
  _match_secret "generic_secret_assignment" '(api[_-]?key|client[_-]?secret|secret[_-]?key|access[_-]?token|auth[_-]?token|password|passwd)[[:space:]]*[:=][[:space:]]*['"'"'"]?[A-Za-z0-9._~+/=!@#$%-]{16,}'

  if [[ -n "$_secret_match_label" ]]; then
    _REASON="🔒 AIRLOCK [secret_scan]: Potential secret detected in write content."
    _CONTEXT="Denied write — matched pattern: ${_secret_match_label}\nTarget file: ${FILE_PATH}\n\nTo write secrets intentionally:\n  1. Use an allowlisted path (.env, CLAUDE.md, infrastructure.md, credentials/, secrets/)\n  2. Or: export ARTIFACT_GUARD_BYPASS=1 (always logged)\n\nThis check is deterministic (regex, no LLM). If you hit a false positive on\ndocumentation examples, use clearly fake values (e.g. sk-placeholder-xxx)\nor use the bypass."
    jq -n --arg r "$_REASON" --arg c "$_CONTEXT" \
      '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r,additionalContext:$c}}'
    exit 0
  fi
fi

# ─────────────────────────────────────────────────────────────────────────────
# CLASSIFY: determine route and file class
# ─────────────────────────────────────────────────────────────────────────────
ROUTE="local_copy"
FILE_CLASS="source"

# --- PASS: never-backup paths ---
# Segment-anchored: match /segment/ or /segment at end
_is_pass_path() {
  local p="$1"
  case "$p" in
    *"/node_modules/"*|*"/node_modules") return 0 ;;
  esac
  case "$p" in
    *"/.git/"*|*"/.git") return 0 ;;
  esac
  case "$p" in
    /tmp/*|/var/tmp/*|/private/tmp/*|/var/*) return 0 ;;
  esac
  case "$p" in
    *.log|*.cache|*/.DS_Store) return 0 ;;
  esac
  return 1
}

if _is_pass_path "$FILE_PATH"; then
  exit 0
fi

# --- REMOTE_STRICT: Supabase DB SQL with DDL ---
_maybe_supabase=0
case "$FILE_PATH" in
  */supabase/*.sql|*/supabase/functions/*.sql|\
  */supabase/triggers/*.sql|*/supabase/views/*.sql|\
  */supabase/sql/*.sql)
    # Exclude non-live-object paths
    case "$FILE_PATH" in
      */supabase/migrations/*|*/supabase/seeds/*|\
      */supabase/test-functions/*|*/supabase/schema-audit/*)
        _maybe_supabase=0 ;;
      *)
        _maybe_supabase=1 ;;
    esac ;;
esac

if [[ "$_maybe_supabase" == "1" ]]; then
  if [[ -f "$FILE_PATH" ]]; then
    # Existing file: content-sniff to confirm it contains DDL
    _sniff="$(head -c 8192 "$FILE_PATH" 2>/dev/null | tr '[:lower:]' '[:upper:]')"
    case "$_sniff" in
      *"CREATE OR REPLACE FUNCTION"*|*"CREATE OR REPLACE TRIGGER"*|\
      *"CREATE OR REPLACE VIEW"*|*"CREATE FUNCTION"*|\
      *"CREATE TRIGGER"*|*"CREATE VIEW"*)
        ROUTE="remote_strict"
        FILE_CLASS="supabase_sql" ;;
    esac
  else
    # New file: cannot content-sniff, but path alone is sufficient to require a
    # backup — if you're creating a new Supabase SQL file, live state must be
    # backed up first. Backup freshness check (CHECK 1) still runs; hash check
    # (CHECK 2) is skipped since there is no existing disk file to compare.
    ROUTE="remote_strict"
    FILE_CLASS="supabase_sql"
  fi
fi

# --- REMOTE_BEST_EFFORT: edge functions, n8n workflows ---
if [[ "$ROUTE" == "local_copy" ]]; then
  case "$FILE_PATH" in
    */supabase/functions/*/index.ts|*/supabase/functions/*/*.ts)
      ROUTE="remote_best_effort"
      FILE_CLASS="edge_function" ;;
    *.json)
      if [[ -f "$FILE_PATH" ]]; then
        _sniff="$(head -c 512 "$FILE_PATH" 2>/dev/null)"
        case "$_sniff" in
          *'"nodes"'*|*'"connections"'*)
            ROUTE="remote_best_effort"
            FILE_CLASS="n8n_workflow" ;;
        esac
      fi ;;
  esac
fi

# --- Refine FILE_CLASS for local_copy ---
if [[ "$ROUTE" == "local_copy" ]]; then
  case "$FILE_PATH" in
    *.py)                        FILE_CLASS="python" ;;
    *.ts|*.tsx)                  FILE_CLASS="typescript" ;;
    *.js|*.jsx|*.mjs)            FILE_CLASS="javascript" ;;
    *.sh|*.bash)                 FILE_CLASS="shell" ;;
    *.md)                        FILE_CLASS="markdown" ;;
    *.txt)                       FILE_CLASS="text" ;;
    *.json)                      FILE_CLASS="json" ;;
    *Dockerfile|*.dockerfile)    FILE_CLASS="dockerfile" ;;
    *.tf|*.tfvars)               FILE_CLASS="terraform" ;;
    *.yaml|*.yml)                FILE_CLASS="yaml" ;;
    *.sql)                       FILE_CLASS="sql" ;;
    *)                           FILE_CLASS="source" ;;
  esac
fi

# ─────────────────────────────────────────────────────────────────────────────
# NEW FILE CHECK
# ─────────────────────────────────────────────────────────────────────────────
SOURCE_EXISTS="true"
if [[ ! -f "$FILE_PATH" ]]; then
  SOURCE_EXISTS="false"
fi

# ─────────────────────────────────────────────────────────────────────────────
# RESOLVE REPO (cached to avoid git subprocess overhead per-edit)
# Cache: /tmp/claude-artifact-guard/<sha>.gitroot, TTL invalidated by
# .git/index or HEAD mtime.
# ─────────────────────────────────────────────────────────────────────────────
REPO_NAME="no-git"
GIT_ROOT=""
FILE_DIR="$(dirname "$FILE_PATH")"
CACHE_BASE="/tmp/claude-artifact-guard"
mkdir -p "$CACHE_BASE" 2>/dev/null
CACHE_KEY="$(printf '%s' "$FILE_DIR" | shasum | awk '{print $1}')"
CACHE_FILE="$CACHE_BASE/${CACHE_KEY}.gitroot"

_load_cached_root() {
  [[ ! -f "$CACHE_FILE" ]] && return 1
  local _root
  _root="$(cat "$CACHE_FILE" 2>/dev/null)" || return 1
  [[ -z "$_root" || ! -d "$_root/.git" ]] && return 1
  # Invalidate if .git/index OR HEAD is newer than the cache file
  local _cache_mt _idx_mt _head_mt _newer
  _cache_mt="$(stat -f %m "$CACHE_FILE" 2>/dev/null || echo 0)"
  _idx_mt="$(stat -f %m "$_root/.git/index" 2>/dev/null || echo 0)"
  _head_mt="$(stat -f %m "$_root/.git/HEAD"  2>/dev/null || echo 0)"
  _newer="$_idx_mt"
  [[ "$_head_mt" -gt "$_newer" ]] && _newer="$_head_mt"
  [[ "$_cache_mt" -le "$_newer" ]] && return 1   # stale
  GIT_ROOT="$_root"
  return 0
}

if ! _load_cached_root; then
  GIT_ROOT="$(cd "$FILE_DIR" 2>/dev/null && git rev-parse --show-toplevel 2>/dev/null || echo "")"
  printf '%s' "$GIT_ROOT" > "$CACHE_FILE" 2>/dev/null
fi

# Agent config dirs are always local_copy even if not in a git repo
if [[ -z "$GIT_ROOT" ]]; then
  case "$FILE_PATH" in
    "$HOME/.claude/"*) GIT_ROOT="$HOME/.claude"; REPO_NAME="claude-config"  ;;
    "$HOME/.gemini/"*) GIT_ROOT="$HOME/.gemini"; REPO_NAME="gemini-config"  ;;
    "$HOME/.codex/"*)  GIT_ROOT="$HOME/.codex";  REPO_NAME="codex-config"   ;;
  esac
fi

if [[ -n "$GIT_ROOT" && "$REPO_NAME" == "no-git" ]]; then
  REPO_NAME="$(basename "$GIT_ROOT")"
fi

# ─────────────────────────────────────────────────────────────────────────────
# BUILD BACKUP PATH
# ─────────────────────────────────────────────────────────────────────────────
BACKUP_ROOT="$HOME/.claude/artifact-backups"
NOW_DATE="$(date +%Y/%m/%d)"
TIMESTAMP="$(date +%Y%m%dT%H%M%S)_$$"
FILE_BASENAME="$(basename "$FILE_PATH")"
BACKUP_DIR="$BACKUP_ROOT/$REPO_NAME/$FILE_CLASS/$NOW_DATE"
mkdir -p "$BACKUP_DIR" 2>/dev/null
BACKUP_BASE="$BACKUP_DIR/${TIMESTAMP}__${FILE_BASENAME}"
BACKUP_META="${BACKUP_BASE}.meta.json"

# ─────────────────────────────────────────────────────────────────────────────
# CAPTURE GIT STATE
# ─────────────────────────────────────────────────────────────────────────────
GIT_STATE="no-git"
if [[ -n "$GIT_ROOT" && -d "$GIT_ROOT/.git" ]]; then
  _gs="$(cd "$GIT_ROOT" 2>/dev/null && git status --porcelain "$FILE_PATH" 2>/dev/null | head -1 || echo "")"
  if [[ -z "$_gs" ]]; then
    GIT_STATE="clean"
  else
    case "$_gs" in
      "??"*) GIT_STATE="untracked" ;;
      *)     GIT_STATE="dirty"     ;;
    esac
  fi
fi

# ─────────────────────────────────────────────────────────────────────────────
# HASH SOURCE FILE
# ─────────────────────────────────────────────────────────────────────────────
SRC_HASH="none"
if [[ "$SOURCE_EXISTS" == "true" ]]; then
  SRC_HASH="$(shasum -a 256 "$FILE_PATH" 2>/dev/null | awk '{print $1}' || echo "hash-error")"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TAKE SNAPSHOT (atomic: gz to .tmp → mv; plain copy fallback)
# ─────────────────────────────────────────────────────────────────────────────
BACKUP_STATUS="skipped"
BACKUP_PATH=""

_take_snapshot() {
  local _src="$1" _base="$2"
  local _gz="${_base}.gz" _tmp="${_base}.gz.tmp" _plain="${_base}.bak"

  # Attempt gzip compression (atomic write via tmp file)
  if gzip -c "$_src" > "$_tmp" 2>/dev/null; then
    if mv "$_tmp" "$_gz" 2>/dev/null; then
      BACKUP_STATUS="ok_gz"
      BACKUP_PATH="$_gz"
      return 0
    fi
    rm -f "$_tmp" 2>/dev/null
  else
    rm -f "$_tmp" 2>/dev/null
  fi

  # Fallback: plain copy (for already-compressed or binary files)
  if cp -p "$_src" "$_plain" 2>/dev/null; then
    BACKUP_STATUS="ok_plain"
    BACKUP_PATH="$_plain"
    return 0
  fi

  BACKUP_STATUS="copy-failed"
  return 1
}

if [[ "$SOURCE_EXISTS" == "true" ]]; then
  _take_snapshot "$FILE_PATH" "$BACKUP_BASE"
fi

# ─────────────────────────────────────────────────────────────────────────────
# WRITE METADATA SIDECAR (always uncompressed for easy inspection)
# ─────────────────────────────────────────────────────────────────────────────
NOW_ISO="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
jq -n \
  --arg  class       "$FILE_CLASS" \
  --arg  route       "$ROUTE" \
  --arg  sha256      "$SRC_HASH" \
  --arg  git_state   "$GIT_STATE" \
  --arg  git_root    "$GIT_ROOT" \
  --arg  tool        "$TOOL_NAME" \
  --arg  ts          "$NOW_ISO" \
  --arg  src         "$FILE_PATH" \
  --arg  bpath       "$BACKUP_PATH" \
  --arg  bstatus     "$BACKUP_STATUS" \
  --argjson src_exists "$SOURCE_EXISTS" \
  '{
    class:         $class,
    route:         $route,
    sha256:        $sha256,
    git_state:     $git_state,
    git_root:      $git_root,
    tool:          $tool,
    timestamp:     $ts,
    source_file:   $src,
    source_exists: $src_exists,
    backup_path:   $bpath,
    backup_status: $bstatus
  }' > "$BACKUP_META" 2>/dev/null

# ─────────────────────────────────────────────────────────────────────────────
# REMOTE_STRICT DENIAL CHECK (Supabase DB SQL only)
# Requires: fresh backup (agent-created via mcp__supabase__execute_sql) AND
# disk file hash matching that backup. Same logic as pre-tool-use-supabase-gate.sh.
# ─────────────────────────────────────────────────────────────────────────────

# Normalize SQL text: extract from first CREATE statement, collapse whitespace
_normalize_sql() {
  awk '
    BEGIN { printing = 0 }
    {
      line = toupper($0)
      if (match(line, /^[[:space:]]*(CREATE[[:space:]]+OR[[:space:]]+REPLACE|CREATE[[:space:]]+(FUNCTION|TRIGGER|VIEW))/)) {
        printing = 1
      }
      if (printing) print
    }
  ' "$1" | tr -s '[:space:]' ' ' | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//'
}

_sql_hash() {
  _normalize_sql "$1" | shasum -a 256 | awk '{print $1}'
}

if [[ "$ROUTE" == "remote_strict" ]]; then
  # SOURCE_EXISTS may be false (new file) — backup freshness check (CHECK 1) always
  # runs; hash staleness check (CHECK 2) is gated on SOURCE_EXISTS below.

  # Determine object type from path
  OBJ_TYPE="function"
  case "$FILE_PATH" in
    */triggers/*.sql) OBJ_TYPE="trigger" ;;
    */views/*.sql)    OBJ_TYPE="view"    ;;
  esac
  BASE_NAME="$(basename "$FILE_PATH")"
  OBJ_NAME="${BASE_NAME%.sql}"

  # Build backup directory list (priority order)
  # Use += throughout — safe for empty arrays in bash 3.2 with set -u
  BACKUP_DIRS=()
  REPO_ROOT="$(cd "$(dirname "$FILE_PATH")" 2>/dev/null && git rev-parse --show-toplevel 2>/dev/null || echo "")"
  [[ -n "$REPO_ROOT" && -d "$REPO_ROOT/supabase-backups" ]] && \
    BACKUP_DIRS+=("$REPO_ROOT/supabase-backups")
  GDRIVE_BACKUP="${SUPABASE_GDRIVE_BACKUP_PATH:-}"  # Set via env: export SUPABASE_GDRIVE_BACKUP_PATH="$HOME/My Drive/Your Folder/backups/supabase"
  [[ -d "$GDRIVE_BACKUP" ]] && BACKUP_DIRS+=("$GDRIVE_BACKUP")
  [[ -d "$HOME/supabase-backups" ]] && BACKUP_DIRS+=("$HOME/supabase-backups")
  if [[ -n "${SUPABASE_BACKUP_DIRS:-}" ]]; then
    IFS=':' read -r -a _extra <<< "$SUPABASE_BACKUP_DIRS"
    for _d in "${_extra[@]}"; do
      [[ -d "$_d" ]] && BACKUP_DIRS+=("$_d")
    done
  fi

  TTL_MINS="${SUPABASE_BACKUP_TTL_MINS:-30}"
  if ! [[ "$TTL_MINS" =~ ^[0-9]+$ ]] || [[ "$TTL_MINS" -eq 0 ]]; then TTL_MINS=30; fi

  PATTERN="*_${OBJ_TYPE}_${OBJ_NAME}.sql"
  LATEST_BACKUP=""

  for _dir in "${BACKUP_DIRS[@]}"; do
    _cand="$(find "$_dir" -maxdepth 1 -type f -name "$PATTERN" -mmin "-$TTL_MINS" -print0 2>/dev/null \
      | xargs -0 ls -t 2>/dev/null | head -n 1 || true)"
    if [[ -n "$_cand" ]]; then
      LATEST_BACKUP="$_cand"
      break
    fi
  done

  # --- CHECK 1: fresh backup must exist ---
  if [[ -z "$LATEST_BACKUP" ]]; then
    DIRS_LISTED=""
    for _dir in "${BACKUP_DIRS[@]}"; do DIRS_LISTED="${DIRS_LISTED}  - ${_dir}\n"; done
    [[ -z "$DIRS_LISTED" ]] && DIRS_LISTED="  (no backup directories configured)\n"

    _REASON="🔒 AIRLOCK [remote_strict]: No fresh backup for ${OBJ_TYPE} '${OBJ_NAME}'."
    _CONTEXT="Before editing Supabase SQL files you MUST pull a live backup first.\n\nRequired: '${PATTERN}' (mtime <= ${TTL_MINS}m) in:\n${DIRS_LISTED}\nRun backup protocol:\n  1. Use mcp__supabase__execute_sql to pull the live definition\n  2. Save to supabase-backups/ as: YYYYMMDD_HHMMSS_<ctx>_${OBJ_TYPE}_${OBJ_NAME}.sql\n  3. Retry this edit.\n\nPre-edit disk snapshot: ${BACKUP_PATH:-none}"

    jq -n --arg r "$_REASON" --arg c "$_CONTEXT" \
      '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r,additionalContext:$c}}'
    exit 0
  fi

  # --- CHECK 2: disk file must match backup (not stale) — existing files only ---
  # New files have no disk content to compare, so skip this check for them.
  if [[ "$SOURCE_EXISTS" == "false" ]]; then exit 0; fi

  HASH_FILE="$(_sql_hash "$FILE_PATH"   2>/dev/null || echo "HASH_ERROR_FILE")"
  HASH_BACK="$(_sql_hash "$LATEST_BACKUP" 2>/dev/null || echo "HASH_ERROR_BACKUP")"

  if [[ "$HASH_FILE" == "HASH_ERROR_FILE" || "$HASH_BACK" == "HASH_ERROR_BACKUP" ]]; then
    if [[ "${SUPABASE_GATE_ALLOW_HASH_ERROR:-0}" == "1" ]]; then exit 0; fi
    _REASON="🔒 AIRLOCK [remote_strict]: Hash verification failed for ${OBJ_TYPE} '${OBJ_NAME}'."
    _CONTEXT="Cannot compute normalized SQL hash.\nDisk: ${HASH_FILE}\nBackup: ${HASH_BACK}\n\nEmergency bypass: export SUPABASE_GATE_ALLOW_HASH_ERROR=1"
    jq -n --arg r "$_REASON" --arg c "$_CONTEXT" \
      '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r,additionalContext:$c}}'
    exit 0
  fi

  # Allow if no CREATE found in file (empty normalized hash = new/blank file)
  EMPTY_HASH="$(printf '' | shasum -a 256 | awk '{print $1}')"
  if [[ "$HASH_FILE" == "$EMPTY_HASH" ]]; then exit 0; fi

  if [[ "$HASH_FILE" != "$HASH_BACK" ]]; then
    _NORM_DISK="$(mktemp 2>/dev/null || echo "/tmp/_guard_disk_$$")"
    _NORM_BACK="$(mktemp 2>/dev/null || echo "/tmp/_guard_back_$$")"
    _normalize_sql "$LATEST_BACKUP"  > "$_NORM_BACK" 2>/dev/null
    _normalize_sql "$FILE_PATH"      > "$_NORM_DISK" 2>/dev/null
    _DIFF="$(diff -u \
      --label 'BACKUP (live Supabase)' \
      --label 'DISK (local — STALE)' \
      "$_NORM_BACK" "$_NORM_DISK" 2>/dev/null | head -30 || true)"
    rm -f "$_NORM_DISK" "$_NORM_BACK" 2>/dev/null

    _REASON="🔒 AIRLOCK [remote_strict]: Stale base for ${OBJ_TYPE} '${OBJ_NAME}'. Disk != live Supabase."
    _CONTEXT="DISK FILE IS OUT OF DATE.\n\nDisk:    ${FILE_PATH}\nBackup:  ${LATEST_BACKUP}\nDisk snapshot saved: ${BACKUP_PATH:-none}"
    [[ -n "$_DIFF" ]] && _CONTEXT="${_CONTEXT}\n\n--- DIFF ---\n${_DIFF}\n--- END ---"
    _CONTEXT="${_CONTEXT}\n\nFIX:\n  cp \"${LATEST_BACKUP}\" \"${FILE_PATH}\"\n  Then retry."

    jq -n --arg r "$_REASON" --arg c "$_CONTEXT" \
      '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r,additionalContext:$c}}'
    exit 0
  fi

fi  # end remote_strict

# ─────────────────────────────────────────────────────────────────────────────
# PROBABILISTIC RETENTION CLEANUP
# 1-in-100 chance per hook invocation: prune backups older than 180 days
# Runs in background subshell — never blocks the edit
# ─────────────────────────────────────────────────────────────────────────────
_RAND=$((RANDOM % 100))
if [[ "$_RAND" -eq 0 ]]; then
  (
    find "$BACKUP_ROOT" -type f \
      \( -name "*.gz" -o -name "*.meta.json" -o -name "*.bak" -o -name "*.sql" \) \
      -mtime +180 -delete 2>/dev/null
  ) &
fi

# All non-remote_strict routes: always allow (fail-open)
exit 0
