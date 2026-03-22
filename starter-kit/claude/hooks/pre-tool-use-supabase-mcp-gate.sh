#!/bin/bash
# ============================================================================
# SUPABASE MCP GATE — PreToolUse Enforcement
# Purpose: Require a fresh backup before any mcp__supabase__apply_migration
#          or destructive mcp__supabase__execute_sql / mcp__supabase-local__execute_sql call.
#
# WHY THIS EXISTS:
#   pre-tool-use-artifact-guard.sh (the Airlock) only fires on Edit|Write tool
#   calls. MCP tool calls bypass it entirely. This gate closes that gap for the
#   two Supabase tools capable of irreversible schema/data changes.
#
# Covered tools:
#   mcp__supabase__apply_migration       — always destructive (DDL by definition)
#   mcp__supabase__execute_sql           — destructive if SQL contains DDL or DML
#   mcp__supabase-local__execute_sql     — same as above for local instance
#
# Destructive SQL patterns:
#   DDL: CREATE, ALTER, DROP, TRUNCATE
#   DML: INSERT, UPDATE, DELETE
#   Read-only (SELECT, EXPLAIN, SHOW, etc.) → pass through silently
#
# Backup check:
#   Looks for any .sql file modified within TTL in known backup directories.
#   "A recent backup file exists" = backup protocol was run this session.
#   TTL default: 60 minutes (override: $SUPABASE_MCP_GATE_TTL_MINS)
#
# Backup directories checked (in order):
#   1. <git_root>/supabase-backups/     (project-local)
#   2. <cwd>/supabase-backups/           (fallback)
#   3. $SUPABASE_GDRIVE_BACKUP_PATH      (Google Drive, env-configurable)
#   4. ~/supabase-backups/               (home-level fallback)
#   5. $SUPABASE_BACKUP_DIRS             (colon-separated extra dirs)
#
# Emergency bypass: export SUPABASE_MCP_GATE_BYPASS=1  (always logged)
#
# macOS notes:
#   - Bash 3.2 compatible: no associative arrays, no mapfile
#   - Uses shasum (not sha256sum), stat -f %m (not stat -c %Y)
#
# Created: 2026-03-05
# Companion to: pre-tool-use-artifact-guard.sh
# ============================================================================

set -uo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# PARSE INPUT
# ─────────────────────────────────────────────────────────────────────────────
INPUT="$(cat)" || INPUT=""
TOOL_NAME="$(echo "$INPUT" | jq -r '.tool_name // empty' 2>/dev/null)"           || TOOL_NAME=""
SQL="$(echo "$INPUT" | jq -r '.tool_input.query // empty' 2>/dev/null)"          || SQL=""
MIGRATION_NAME="$(echo "$INPUT" | jq -r '.tool_input.name // empty' 2>/dev/null)" || MIGRATION_NAME=""
CWD_FROM_INPUT="$(echo "$INPUT" | jq -r '.cwd // empty' 2>/dev/null)"            || CWD_FROM_INPUT=""
CWD="${CWD_FROM_INPUT:-${PWD:-$HOME}}"

# ─────────────────────────────────────────────────────────────────────────────
# EARLY EXIT: only act on covered tools
# ─────────────────────────────────────────────────────────────────────────────
case "$TOOL_NAME" in
  mcp__supabase__apply_migration|\
  mcp__supabase__execute_sql|\
  mcp__supabase-local__execute_sql)
    ;;  # continue to gate logic
  *)
    exit 0 ;;
esac

# ─────────────────────────────────────────────────────────────────────────────
# EMERGENCY BYPASS (always logged)
# ─────────────────────────────────────────────────────────────────────────────
if [[ "${SUPABASE_MCP_GATE_BYPASS:-0}" == "1" ]]; then
  LOG_FILE="$HOME/.claude/artifact-guard-logs/supabase-mcp-gate.log"
  mkdir -p "$(dirname "$LOG_FILE")" 2>/dev/null
  printf '%s BYPASS tool=%s migration=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$TOOL_NAME" "${MIGRATION_NAME:-n/a}" \
    >> "$LOG_FILE" 2>/dev/null
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# CLASSIFY: is this operation destructive?
# ─────────────────────────────────────────────────────────────────────────────
IS_DESTRUCTIVE=0
OPERATION_LABEL=""

if [[ "$TOOL_NAME" == "mcp__supabase__apply_migration" ]]; then
  # apply_migration is always DDL — always requires backup
  IS_DESTRUCTIVE=1
  OPERATION_LABEL="apply_migration${MIGRATION_NAME:+ '${MIGRATION_NAME}'}"

else
  # execute_sql: scan SQL body for destructive keywords
  if [[ -n "$SQL" ]]; then
    SQL_UPPER="$(printf '%s' "$SQL" | tr '[:lower:]' '[:upper:]' | tr -s '[:space:]' ' ')"
    case "$SQL_UPPER" in
      *"CREATE "*|*"ALTER "*|*"DROP "*|*"TRUNCATE "*|\
      *"INSERT "*|*"UPDATE "*|*"DELETE "*)
        IS_DESTRUCTIVE=1
        OPERATION_LABEL="execute_sql [destructive]" ;;
    esac
  fi
fi

# Read-only or unclassified SQL — allow silently
[[ "$IS_DESTRUCTIVE" -eq 0 ]] && exit 0

# ─────────────────────────────────────────────────────────────────────────────
# RESOLVE BACKUP DIRECTORIES
# ─────────────────────────────────────────────────────────────────────────────
BACKUP_DIRS=()

# 1. Project-local: git root supabase-backups/
GIT_ROOT="$(cd "$CWD" 2>/dev/null && git rev-parse --show-toplevel 2>/dev/null || echo "")"
if [[ -n "$GIT_ROOT" && -d "$GIT_ROOT/supabase-backups" ]]; then
  BACKUP_DIRS+=("$GIT_ROOT/supabase-backups")
fi

# 2. CWD-local (if different from git root)
if [[ -d "$CWD/supabase-backups" ]]; then
  _cwdpath="$CWD/supabase-backups"
  _already=0
  for _d in "${BACKUP_DIRS[@]+"${BACKUP_DIRS[@]}"}"; do
    [[ "$_d" == "$_cwdpath" ]] && _already=1 && break
  done
  [[ "$_already" -eq 0 ]] && BACKUP_DIRS+=("$_cwdpath")
fi

# 3. Google Drive backup path (env-configurable)
GDRIVE_BACKUP="${SUPABASE_GDRIVE_BACKUP_PATH:-}"
[[ -n "$GDRIVE_BACKUP" && -d "$GDRIVE_BACKUP" ]] && BACKUP_DIRS+=("$GDRIVE_BACKUP")

# 4. Home-level fallback
[[ -d "$HOME/supabase-backups" ]] && BACKUP_DIRS+=("$HOME/supabase-backups")

# 5. Extra dirs via env (colon-separated)
if [[ -n "${SUPABASE_BACKUP_DIRS:-}" ]]; then
  IFS=':' read -r -a _extra <<< "$SUPABASE_BACKUP_DIRS"
  for _d in "${_extra[@]}"; do
    [[ -d "$_d" ]] && BACKUP_DIRS+=("$_d")
  done
fi

# ─────────────────────────────────────────────────────────────────────────────
# BACKUP FRESHNESS CHECK
# Any .sql file modified within TTL = backup protocol was run this session
# ─────────────────────────────────────────────────────────────────────────────
TTL_MINS="${SUPABASE_MCP_GATE_TTL_MINS:-60}"
if ! [[ "$TTL_MINS" =~ ^[0-9]+$ ]] || [[ "$TTL_MINS" -eq 0 ]]; then TTL_MINS=60; fi

FOUND_BACKUP=""
for _dir in "${BACKUP_DIRS[@]+"${BACKUP_DIRS[@]}"}"; do
  _cand="$(find "$_dir" -maxdepth 1 -type f -name "*.sql" -mmin "-${TTL_MINS}" -print0 2>/dev/null \
    | xargs -0 ls -t 2>/dev/null | head -n 1 || true)"
  if [[ -n "$_cand" ]]; then
    FOUND_BACKUP="$_cand"
    break
  fi
done

# ─────────────────────────────────────────────────────────────────────────────
# GATE DECISION
# ─────────────────────────────────────────────────────────────────────────────
if [[ -z "$FOUND_BACKUP" ]]; then
  DIRS_LISTED=""
  for _dir in "${BACKUP_DIRS[@]+"${BACKUP_DIRS[@]}"}"; do
    DIRS_LISTED="${DIRS_LISTED}  - ${_dir}\n"
  done
  [[ -z "$DIRS_LISTED" ]] && DIRS_LISTED="  (no backup directories found — is supabase-backups/ missing?)\n"

  _REASON="🔒 SUPABASE MCP GATE: No fresh backup before ${OPERATION_LABEL}."
  _CONTEXT="Destructive Supabase MCP call blocked.\nNo .sql backup found within ${TTL_MINS} minutes in:\n${DIRS_LISTED}\nRequired action:\n  1. Run /backup-supabase  (or invoke the sub-agent backup protocol manually)\n  2. Confirm .sql files appear in supabase-backups/\n  3. Retry this operation\n\nAlternatives:\n  export SUPABASE_MCP_GATE_BYPASS=1  (always logged to artifact-guard-logs/supabase-mcp-gate.log)\n  export SUPABASE_MCP_GATE_TTL_MINS=120  (extend the TTL window)\n\nNOTE: This gate exists because mcp__supabase tools bypass the Airlock\n(pre-tool-use-artifact-guard.sh only covers Edit/Write tool calls).\nThis hook is the enforcement equivalent for MCP-path schema changes."

  jq -n --arg r "$_REASON" --arg c "$_CONTEXT" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r,additionalContext:$c}}'
  exit 0
fi

# Fresh backup found — log and allow
LOG_FILE="$HOME/.claude/artifact-guard-logs/supabase-mcp-gate.log"
mkdir -p "$(dirname "$LOG_FILE")" 2>/dev/null
printf '%s ALLOW tool=%s op=%s backup=%s\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$TOOL_NAME" "$OPERATION_LABEL" "$FOUND_BACKUP" \
  >> "$LOG_FILE" 2>/dev/null

exit 0
