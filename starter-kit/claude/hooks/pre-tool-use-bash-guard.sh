#!/bin/bash
# ============================================================================
# BASH GUARD HOOK — PreToolUse protection for destructive database operations
#
# Fires before every Bash tool call. Blocks commands that DELETE, TRUNCATE,
# DROP, or ETL-reload database data unless a recent backup exists.
#
# Destructive patterns intercepted:
#   SQL:    DELETE FROM, TRUNCATE [TABLE], DROP TABLE/SCHEMA/DATABASE/VIEW/
#           INDEX/FUNCTION/TYPE, ALTER TABLE ... DROP COLUMN, UPDATE ... SET
#   ETL:    Python/Node invocation + ETL keyword (load/reload/etl/ingest/seed)
#           AND a destructive flag (--load, --reload, --force, etc.)
#   PG:     pg_restore, psql -f <file>, psql with inline destructive SQL
#
# Preprocessing: strips heredoc blocks and -m commit message bodies before
# pattern matching to prevent false positives in git commit messages.
#
# Backup check: scans known backup dirs for .dump/.sql/.sql.gz/.backup files
#               modified within BASH_GUARD_BACKUP_TTL_MINS (default: 30)
#
# Backup dirs checked (in order):
#   1. <git_root>/supabase-backups/
#   2. <git_root>/backups/
#   3. <cwd>/backups/
#   4. <cwd>/../backups/          ← catches etl/backups/ from parent
#   5. ~/supabase-backups/
#   6. Custom path via SUPABASE_BACKUP_DIRS env var (colon-separated)
#   7. $BASH_GUARD_BACKUP_DIRS (colon-separated override)
#
# Bypass options:
#   File:   Write ~/.claude/bash-guard-bypass (single-use, 5min TTL)
#   Env:    Set BASH_GUARD_BYPASS=1 in the shell BEFORE starting Claude Code
#           (inline export inside blocked command does NOT work — hook runs first)
#
# macOS compatible: stat -f %m, printf not echo, no mapfile, bash 3.2+
# Reviewed by: Gemini + Codex, 2026-02-24
# Created: 2026-02-24
# ============================================================================

set -uo pipefail

# ─── Parse input ─────────────────────────────────────────────────────────────
INPUT="$(cat)" || INPUT=""
TOOL_NAME="$(printf '%s' "$INPUT" | jq -r '.tool_name // empty' 2>/dev/null)" || TOOL_NAME=""
COMMAND="$(printf '%s' "$INPUT" | jq -r '.tool_input.command // empty' 2>/dev/null)" || COMMAND=""
CWD="$(printf '%s' "$INPUT" | jq -r '.cwd // empty' 2>/dev/null)" || CWD=""

[[ "$TOOL_NAME" != "Bash" ]] && exit 0
[[ -z "$COMMAND" ]]           && exit 0

BACKUP_TTL_MINS="${BASH_GUARD_BACKUP_TTL_MINS:-30}"
LOG_DIR="$HOME/.claude/artifact-guard-logs"
mkdir -p "$LOG_DIR" 2>/dev/null

# ─── Bypass mechanisms ───────────────────────────────────────────────────────
# File bypass: create ~/.claude/bash-guard-bypass using the Write tool.
# Single-use: hook deletes it immediately. Expires after 5 minutes regardless.
# NOTE: inline `export BASH_GUARD_BYPASS=1 && command` does NOT work —
#       the hook runs BEFORE the command subprocess starts.

BYPASS_FILE="$HOME/.claude/bash-guard-bypass"

_do_bypass() {
  printf '%s BYPASS[%s] cmd=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "${COMMAND:0:120}" \
    >> "$LOG_DIR/bash-guard-bypass.log" 2>/dev/null
  exit 0
}

if [[ "${BASH_GUARD_BYPASS:-0}" == "1" ]]; then
  _do_bypass "env"
fi

if [[ -f "$BYPASS_FILE" ]]; then
  file_mtime="$(stat -f %m "$BYPASS_FILE" 2>/dev/null)" || file_mtime=0
  file_age="$(( $(date +%s) - file_mtime ))"
  rm -f "$BYPASS_FILE" 2>/dev/null   # single-use: consume regardless of age
  if [[ "$file_age" -le 300 ]]; then
    _do_bypass "file"
  fi
  # expired — fall through to enforcement
fi

# ─── Preprocess: strip heredocs and commit message bodies ────────────────────
# Prevents false positives when commit messages mention DELETE/pg_restore/etc.
# Uses env var to pass command safely to Python (avoids shell escaping issues).
CMD_FOR_MATCHING="$COMMAND"
if command -v python3 &>/dev/null; then
  CMD_FOR_MATCHING="$(CMD_IN="$COMMAND" python3 -c "
import os, re
s = os.environ.get('CMD_IN', '')
# FIRST: strip entire git commit commands — they can never be DB-destructive,
# but their message bodies often mention DELETE/DROP/UPDATE etc. (as this very
# commit proved). Replace the whole commit up to the next shell separator so
# downstream patterns do not fire on commit message text.
# Non-greedy .*? with DOTALL consumes heredoc bodies (real newlines included).
# Lookahead (?=...) stops at shell separators (||, &&, ;, |) or string end (\Z).
s = re.sub(
    r'\bgit\s+commit\b.*?(?=\|\||&&|;|\Z)',
    'git commit [stripped]',
    s, flags=re.DOTALL|re.IGNORECASE)
# Strip heredoc blocks: $(cat <<'WORD'\n...\nWORD) and bare <<WORD forms
s = re.sub(
    r'\\\$\(cat\s*<<-?\s*(?:[\"\'']?[A-Za-z_]\w*[\"\'']?)[^\n]*\n.*?\n[A-Za-z_]\w*\s*\)',
    '[heredoc]', s, flags=re.DOTALL)
s = re.sub(
    r'<<-?\s*([\"\'']?)([A-Za-z_]\w*)\1[^\n]*\n.*?\n\2\b',
    '[heredoc]', s, flags=re.DOTALL)
# Strip -m / -am commit message bodies (single and double quoted)
s = re.sub(r\"-[a-zA-Z]*m\s+'(?:[^'\\\\]|\\\\.)*'\", \"-m '[stripped]'\", s, flags=re.DOTALL)
s = re.sub(r'-[a-zA-Z]*m\s+\"(?:[^\"\\\\]|\\\\.)*\"', '-m \"[stripped]\"', s, flags=re.DOTALL)
# Strip --message=... long form (git commit --message=\"...\")
s = re.sub(r'--message=[\"\''](?:[^\"\'']|\\\\.)*[\"\'']', '--message=[stripped]', s, flags=re.DOTALL)
s = re.sub(r'--message=(?![\"\\']).+?(?=\s--|$)', '--message=[stripped]', s, flags=re.DOTALL)
# Strip --body / --body= (gh CLI, curl, etc.) — prevents false positives on gh issue create
s = re.sub(r'--body[= ][\"\''](?:[^\"\'']|\\\\.)*[\"\'']', '--body [stripped]', s, flags=re.DOTALL)
s = re.sub(r'--body[= ](?![\"\\']).+?(?=\s--|$)', '--body [stripped]', s, flags=re.DOTALL)
print(s, end='')
" 2>/dev/null)" || CMD_FOR_MATCHING="$COMMAND"
fi

# Uppercase for case-insensitive matching and flatten newlines so multiline SQL doesn't evade grep
CMD_UPPER="$(printf '%s\n' "$CMD_FOR_MATCHING" | tr '[:lower:]' '[:upper:]' | tr '\n' ' ')"

# ─── Destructive pattern detection ───────────────────────────────────────────
DESTRUCTIVE_REASON=""

# 1. Direct destructive SQL keywords
#    Requires something after FROM/TABLE/etc (not just the keyword alone in a comment)
#    UPDATE...SET included: any SQL mutation requires a backup, even row updates.
if printf '%s\n' "$CMD_UPPER" | grep -qE \
  '(DELETE[[:space:]]+FROM[[:space:]]|TRUNCATE[[:space:]]+(TABLE[[:space:]]+)?[^[:space:],]|DROP[[:space:]]+(TABLE|SCHEMA|DATABASE|VIEW|INDEX|FUNCTION|TYPE)[[:space:]]|ALTER[[:space:]]+TABLE[[:space:]]+[^[:space:]]+[[:space:]]+DROP[[:space:]]+COLUMN|UPDATE[[:space:]]+[^[:space:]]+[[:space:]]+SET[[:space:]])'; then
  DESTRUCTIVE_REASON="Destructive SQL (DELETE/TRUNCATE/DROP/ALTER DROP COLUMN/UPDATE SET)"
fi

# 2. ETL invocation: interpreter + ETL keyword + destructive flag
#    Uses semantic keyword scan instead of brittle awk/basename parsing.
#    All checks run against preprocessed CMD_FOR_MATCHING / CMD_UPPER.
if [[ -z "$DESTRUCTIVE_REASON" ]]; then
  HAS_INTERPRETER=0
  printf '%s\n' "$CMD_UPPER" | grep -qE '\b(PYTHON3?|NODE|NPX|NPM|BUN|DENO|TSX)\b' \
    && HAS_INTERPRETER=1

  HAS_ETL_KEYWORD=0
  printf '%s\n' "$CMD_UPPER" | grep -qE '\b(LOAD|RELOAD|ETL|INGEST|SEED|IMPORT|MIGRATE)\b' \
    && HAS_ETL_KEYWORD=1

  HAS_DESTRUCTIVE_FLAG=0
  printf '%s\n' "$CMD_UPPER" | grep -qE \
    '\-\-(LOAD|RELOAD|FORCE|TRUNCATE|DROP|RESET|CLEAN|WIPE|OVERWRITE|FULL)\b' \
    && HAS_DESTRUCTIVE_FLAG=1

  if [[ "$HAS_INTERPRETER" -eq 1 && "$HAS_ETL_KEYWORD" -eq 1 && "$HAS_DESTRUCTIVE_FLAG" -eq 1 ]]; then
    DESTRUCTIVE_REASON="ETL script with destructive flag"
  fi
fi

# 3. pg_restore — actual invocation (has a following argument, not just mentioned)
#    Checked against preprocessed string to avoid commit message false positives.
if [[ -z "$DESTRUCTIVE_REASON" ]] \
  && printf '%s\n' "$CMD_FOR_MATCHING" | grep -qE '\bpg_restore[[:space:]]+[^[:space:]]'; then
  DESTRUCTIVE_REASON="pg_restore invocation"
fi

# 4. psql with inline destructive SQL OR -f <sql-file>
if [[ -z "$DESTRUCTIVE_REASON" ]] \
  && printf '%s\n' "$CMD_FOR_MATCHING" | grep -qE '\bpsql\b'; then
  # Inline SQL via -c
  if printf '%s\n' "$CMD_UPPER" | grep -qE \
    '(DELETE[[:space:]]+FROM|TRUNCATE|DROP[[:space:]]+(TABLE|SCHEMA|DATABASE))'; then
    DESTRUCTIVE_REASON="psql with inline destructive SQL"
  fi
  # File-driven SQL via -f (conservative: any -f is potentially destructive)
  if [[ -z "$DESTRUCTIVE_REASON" ]] \
    && printf '%s\n' "$CMD_FOR_MATCHING" | grep -qE '\bpsql\b.*-f[[:space:]]+[^[:space:]]'; then
    DESTRUCTIVE_REASON="psql -f <file> (SQL file may contain destructive statements)"
  fi
fi

# Not destructive — allow
[[ -z "$DESTRUCTIVE_REASON" ]] && exit 0

# ─── Target object extraction ────────────────────────────────────────────────
# We already have the command that matched — extract the target object name from
# the token immediately after the SQL keyword. No generic SQL parsing needed;
# each extraction is targeted at the specific keyword position we know fired.
# CMD_UPPER is already preprocessed (heredoc/commit-body stripped) and uppercase.
#
# Result: TARGET_OBJECT = lowercase table/object name (schema-qualifier stripped)
# Fallback: empty string → backup check accepts any recent backup (old behavior)
TARGET_OBJECT=""
_raw=""

if [[ -z "$_raw" ]]; then
  _raw="$(printf '%s\n' "$CMD_UPPER" \
    | grep -oE 'DELETE[[:space:]]+FROM[[:space:]]+[A-Z0-9_"\.-]+'  \
    | grep -oE '[A-Z0-9_"\.-]+$' | head -1)"
fi
if [[ -z "$_raw" ]]; then
  _raw="$(printf '%s\n' "$CMD_UPPER" \
    | grep -oE 'TRUNCATE[[:space:]]+(TABLE[[:space:]]+)?(ONLY[[:space:]]+)?[A-Z0-9_"\.-]+'  \
    | grep -oE '[A-Z0-9_"\.-]+$' | head -1)"
fi
if [[ -z "$_raw" ]]; then
  _raw="$(printf '%s\n' "$CMD_UPPER" \
    | grep -oE 'DROP[[:space:]]+(TABLE|SCHEMA|DATABASE|VIEW|INDEX|FUNCTION|TYPE)[[:space:]]+(IF[[:space:]]+(NOT[[:space:]]+)?EXISTS[[:space:]]+)?[A-Z0-9_"\.-]+'  \
    | grep -oE '[A-Z0-9_"\.-]+$' | head -1)"
fi
if [[ -z "$_raw" ]]; then
  _raw="$(printf '%s\n' "$CMD_UPPER" \
    | grep -oE 'UPDATE[[:space:]]+[A-Z0-9_"\.-]+' \
    | grep -oE '[A-Z0-9_"\.-]+$' | head -1)"
fi
if [[ -z "$_raw" ]]; then
  _raw="$(printf '%s\n' "$CMD_UPPER" \
    | grep -oE 'ALTER[[:space:]]+TABLE[[:space:]]+[A-Z0-9_"\.-]+'  \
    | grep -oE '[A-Z0-9_"\.-]+$' | head -1)"
fi

if [[ -n "$_raw" ]]; then
  _raw="${_raw##*.}"   # strip schema qualifier: SCHEMA.TABLE → TABLE
  TARGET_OBJECT="$(printf '%s' "$_raw" | tr '[:upper:]' '[:lower:]' | tr -d '"')"
fi

# ─── Backup freshness check ───────────────────────────────────────────────────
GIT_ROOT=""
if [[ -n "$CWD" ]]; then
  GIT_ROOT="$(git -C "$CWD" rev-parse --show-toplevel 2>/dev/null)" || GIT_ROOT=""
fi

declare -a BACKUP_DIRS=()
[[ -n "$GIT_ROOT" ]] && BACKUP_DIRS+=("$GIT_ROOT/supabase-backups")
[[ -n "$GIT_ROOT" ]] && BACKUP_DIRS+=("$GIT_ROOT/backups")
if [[ -n "$CWD" ]]; then
  BACKUP_DIRS+=("$CWD/backups")
  BACKUP_DIRS+=("$CWD/../backups")
fi
BACKUP_DIRS+=("$HOME/supabase-backups")
# Add your own Google Drive / cloud backup path via SUPABASE_BACKUP_DIRS env var

if [[ -n "${BASH_GUARD_BACKUP_DIRS:-}" ]]; then
  IFS=':' read -ra EXTRA <<< "$BASH_GUARD_BACKUP_DIRS"
  BACKUP_DIRS+=("${EXTRA[@]}")
fi

NOW="$(date +%s)"
TTL_SECS="$((BACKUP_TTL_MINS * 60))"
CUTOFF="$((NOW - TTL_SECS))"
RECENT_BACKUP=""

for dir in "${BACKUP_DIRS[@]}"; do
  [[ -d "$dir" ]] || continue
  while IFS= read -r -d '' f; do
    mtime="$(stat -f %m "$f" 2>/dev/null)" || continue
    if [[ "$mtime" -ge "$CUTOFF" ]]; then
      if [[ -z "$TARGET_OBJECT" ]]; then
        # Target unknown (ETL/psql-file/pg_restore) — any recent backup satisfies
        RECENT_BACKUP="$f"
        break 2
      else
        # Target known — require the backup filename to contain the object name.
        # Backup naming convention: *_<type>_<object>.sql or *<object>*.dump etc.
        _fname="$(basename "$f" | tr '[:upper:]' '[:lower:]')"
        # "full" / "all" must appear at a word boundary (start, end, or surrounded by [-_.])
        # to prevent "wallet.sql" (contains "all") from satisfying any target's check.
        if [[ "$_fname" == *"$TARGET_OBJECT"* ]] \
           || [[ "$_fname" =~ (^|[-_.])(full|all)([-_.]|$) ]]; then
          RECENT_BACKUP="$f"
          break 2
        fi
      fi
    fi
  done < <(find "$dir" -maxdepth 3 -type f \( \
    -name "*.dump" -o -name "*.sql" -o -name "*.sql.gz" \
    -o -name "*.backup" -o -name "*.pgdump" \) -print0 2>/dev/null)
done

# Recent backup found — allow
if [[ -n "$RECENT_BACKUP" ]]; then
  printf '%s ALLOW reason="%s" target="%s" backup="%s" cmd="%s"\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    "$DESTRUCTIVE_REASON" "${TARGET_OBJECT:-any}" "$RECENT_BACKUP" "${COMMAND:0:100}" \
    >> "$LOG_DIR/bash-guard.log" 2>/dev/null
  exit 0
fi

# ─── Deny — no backup found ───────────────────────────────────────────────────
printf '%s DENY reason="%s" cmd="%s"\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$DESTRUCTIVE_REASON" "${COMMAND:0:100}" \
  >> "$LOG_DIR/bash-guard.log" 2>/dev/null

DIRS_LIST=""
for dir in "${BACKUP_DIRS[@]}"; do
  DIRS_LIST="${DIRS_LIST}  - ${dir}"$'\n'
done

jq -cn \
  --arg reason "$DESTRUCTIVE_REASON" \
  --arg cmd    "${COMMAND:0:300}" \
  --arg dirs   "$DIRS_LIST" \
  --arg ttl    "$BACKUP_TTL_MINS" \
  --arg target "$TARGET_OBJECT" \
  '{
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: (
        "🛑 BASH GUARD — BACKUP REQUIRED\n\n" +
        "Trigger:  " + $reason + "\n" +
        "Command:  " + $cmd + "\n" +
        (if $target != "" then "Target:   " + $target + "\n" else "" end) +
        "\n" +
        (if $target != ""
         then "No backup file containing \"" + $target + "\" found (mtime <= " + $ttl + "m) in:\n"
         else "No .dump/.sql/.sql.gz found in the last " + $ttl + " minutes in:\n"
         end) +
        $dirs + "\n" +
        "Run first:\n" +
        (if $target != ""
         then "  pg_dump \"<connection>\" --table=<schema." + $target + "> -F c -f <YYYYMMDD_" + $target + ".dump>\n"
         else "  pg_dump \"<connection>\" --table=<schema.table> -F c -f <backup.dump>\n"
         end) +
        "  # OR full DB:\n" +
        "  pg_dump \"<connection>\" -F c -f <full_backup.dump>\n\n" +
        "Then re-run.\n\n" +
        "Bypass (one-time): Use the Write tool to create ~/.claude/bash-guard-bypass\n" +
        "Bypass (session):  Set BASH_GUARD_BYPASS=1 in shell BEFORE starting Claude Code"
      )
    }
  }'
exit 0
