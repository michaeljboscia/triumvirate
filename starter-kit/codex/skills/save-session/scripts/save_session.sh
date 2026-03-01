#!/usr/bin/env bash
set -euo pipefail

MODEL="gpt-5.3-codex"
AGENT="codex"
CODEX_HOME="${HOME}/.codex"
STYLE_MODE="plain"

trigger="manual"
project_root="${PWD}"
transcript_path=""
feature_override=""
do_commit="1"

usage() {
  cat <<'USAGE'
Usage: save_session.sh [options]

Options:
  --trigger <manual|session-end>   Trigger value in output (default: manual)
  --project-root <path>            Project root to write session log from (default: current directory)
  --transcript <path>              Native Codex transcript JSONL path (default: latest)
  --feature <name>                 Override feature value
  --no-commit                      Skip git commit step
  -h, --help                       Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --trigger) trigger="${2:-manual}"; shift 2 ;;
    --project-root) project_root="${2:-$PWD}"; shift 2 ;;
    --transcript) transcript_path="${2:-}"; shift 2 ;;
    --feature) feature_override="${2:-}"; shift 2 ;;
    --no-commit) do_commit="0"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage; exit 1 ;;
  esac
done

sanitize() {
  local v="${1:-unknown}"
  v="$(printf '%s' "$v" | tr '[:upper:]' '[:lower:]')"
  v="$(printf '%s' "$v" | sed -E 's/[[:space:]]+/_/g; s/[^a-z0-9._-]+/-/g; s/-+/-/g; s/^[-_.]+//; s/[-_.]+$//')"
  [[ -n "$v" ]] || v="unknown"
  printf '%s' "$v"
}

latest_transcript() {
  local today="${CODEX_HOME}/sessions/$(date '+%Y/%m/%d')"
  local f=""
  if [[ -d "$today" ]]; then
    f="$(ls -1t "$today"/*.jsonl 2>/dev/null | sed -n '1p' || true)"
  fi
  if [[ -z "$f" ]]; then
    f="$(find "${CODEX_HOME}/sessions" -type f -name '*.jsonl' 2>/dev/null | sort | tail -n 1 || true)"
  fi
  printf '%s' "$f"
}

human_size() {
  [[ -f "$1" ]] && du -h "$1" | awk '{print $1}' || printf 'n/a'
}

extract_owner_repo_from_remote() {
  local remote="$1"
  local parsed owner repo
  parsed="$(printf '%s' "$remote" | sed -E 's#^git@[^:]+:##; s#^https?://[^/]+/##; s#\.git$##')"
  owner="$(printf '%s' "$parsed" | awk -F/ '{print $1}')"
  repo="$(printf '%s' "$parsed" | awk -F/ '{print $2}')"
  printf '%s\t%s' "$owner" "$repo"
}

[[ -d "$project_root" ]] || { echo "Project root does not exist: $project_root" >&2; exit 1; }

if [[ -z "$transcript_path" ]]; then
  transcript_path="$(latest_transcript)"
fi
[[ -n "$transcript_path" && -f "$transcript_path" ]] || { echo "No transcript found. Pass --transcript." >&2; exit 1; }

owner="unknown"
client="unknown"
domain="unknown"
repo="unknown"
feature="unknown"

taxonomy_file=""
if [[ -f "$project_root/.codex/taxonomy.json" ]]; then
  taxonomy_file="$project_root/.codex/taxonomy.json"
elif [[ -f "$project_root/.claude/taxonomy.json" ]]; then
  taxonomy_file="$project_root/.claude/taxonomy.json"
fi

if [[ -n "$taxonomy_file" ]]; then
  owner="$(jq -r '.owner // "unknown"' "$taxonomy_file")"
  client="$(jq -r '.client // "unknown"' "$taxonomy_file")"
  domain="$(jq -r '.domain // "unknown"' "$taxonomy_file")"
  repo="$(jq -r '.repo // "unknown"' "$taxonomy_file")"
  feature="$(jq -r '.feature // "unknown"' "$taxonomy_file")"
fi

if git -C "$project_root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  remote_url="$(git -C "$project_root" remote get-url origin 2>/dev/null || true)"
  if [[ -n "$remote_url" ]]; then
    parsed="$(extract_owner_repo_from_remote "$remote_url")"
    git_owner="$(printf '%s' "$parsed" | awk -F'\t' '{print $1}')"
    git_repo="$(printf '%s' "$parsed" | awk -F'\t' '{print $2}')"
    [[ "$owner" == "unknown" && -n "$git_owner" ]] && owner="$git_owner"
    [[ "$repo" == "unknown" && -n "$git_repo" ]] && repo="$git_repo"
  fi
fi

[[ "$repo" != "unknown" ]] || repo="$(basename "$project_root")"
[[ -z "$feature_override" ]] || feature="$feature_override"

owner="$(sanitize "$owner")"
client="$(sanitize "$client")"
domain="$(sanitize "$domain")"
repo="$(sanitize "$repo")"
feature="$(sanitize "$feature")"

if [[ -d "$project_root/.git" || -n "$taxonomy_file" ]]; then
  output_dir="$project_root/session-logs"
else
  output_dir="$CODEX_HOME/session-logs"
fi
mkdir -p "$output_dir"

date_compact="$(TZ='America/New_York' date '+%Y%m%d')"
generated="$(TZ='America/New_York' date '+%Y-%m-%d %H:%M:%S %Z')"
time_hm="$(TZ='America/New_York' date '+%H:%M')"

prefix="${owner}--${client}_${domain}_${repo}_${feature}_${date_compact}"
max_v=0
for f in "$output_dir"/${prefix}_v*_codex.md; do
  [[ -e "$f" ]] || continue
  n="$(basename "$f" | sed -E 's/.*_v([0-9]+)_codex\.md/\1/')"
  if [[ "$n" =~ ^[0-9]+$ ]] && (( n > max_v )); then max_v="$n"; fi
done
next_v=$((max_v + 1))

prev_log="none"
prev="$(ls -1t "$output_dir"/${owner}--${client}_${domain}_${repo}_${feature}_*_codex.md 2>/dev/null | sed -n '1p' || true)"
[[ -n "$prev" ]] && prev_log="$(basename "$prev")"

filename="${prefix}_v${next_v}_codex.md"
log_path="$output_dir/$filename"

transcript_ref="$transcript_path"
if [[ "$transcript_path" == "$CODEX_HOME/"* ]]; then
  transcript_ref="${transcript_path#${CODEX_HOME}/}"
fi
transcript_size="$(human_size "$transcript_path")"

first_user="$(jq -r 'select(.type=="response_item" and .payload.type=="message" and .payload.role=="user") | ([.payload.content[]? | select(.type=="input_text") | .text] | join(" "))' "$transcript_path" | sed '/^$/d' | sed -n '1p')"
last_user="$(jq -r 'select(.type=="response_item" and .payload.type=="message" and .payload.role=="user") | ([.payload.content[]? | select(.type=="input_text") | .text] | join(" "))' "$transcript_path" | sed '/^$/d' | tail -n 1)"
[[ -n "$first_user" ]] || first_user="No explicit user prompt found in transcript."
[[ -n "$last_user" ]] || last_user="$first_user"

goal_text="$(printf '%s' "$last_user" | sed -E 's/[`]/\x27/g')"

changed_files="No tracked file delta captured by this script run."
if git -C "$project_root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  cf="$(git -C "$project_root" status --short 2>/dev/null | awk 'NR<=8 {print $2}' | paste -sd ', ' -)"
  [[ -n "$cf" ]] && changed_files="$cf"
fi

history_rows=""
pattern="$output_dir/${owner}--${client}_${domain}_${repo}_${feature}_*.md"
for f in $pattern; do
  [[ -e "$f" ]] || continue
  row_date="$(grep -m1 '^\*\*Generated:\*\* ' "$f" | sed -E 's/^\*\*Generated:\*\* //')"
  row_session="$(basename "$f" | sed -E 's/.*_(v[0-9]+)_[^_]+\.md/\1/')"
  row_agent="$(basename "$f" | sed -E 's/.*_v[0-9]+_([^_]+)\.md/\1/')"
  row_transcript="$(grep -m1 '^| Transcript | ' "$f" | sed -E 's/^\| Transcript \| `(.*)` \|/\1/')"
  if [[ -z "$row_transcript" || "$row_transcript" == *'| Transcript |'* || "$row_transcript" == *'`'* || "$row_transcript" == *'|'* ]]; then
    row_transcript="unknown"
  fi
  row_size="n/a"
  if [[ "$row_transcript" == sessions/* && -f "$CODEX_HOME/$row_transcript" ]]; then
    row_size="$(human_size "$CODEX_HOME/$row_transcript")"
  elif [[ -f "$row_transcript" ]]; then
    row_size="$(human_size "$row_transcript")"
  fi
  [[ -n "$row_date" ]] || row_date="unknown"
  [[ -n "$row_session" ]] || row_session="unknown"
  [[ -n "$row_agent" ]] || row_agent="unknown"
  [[ -n "$row_transcript" ]] || row_transcript="unknown"
  history_rows+="| ${row_date} | ${row_session} | ${row_agent} | \`${row_transcript}\` | ${row_size} |\n"
done
history_rows+="| ${generated} | v${next_v} | ${AGENT} | \`${transcript_ref}\` | ${transcript_size} |\n"

cat > "$log_path" <<LOG
# Session Log: ${owner}/${client}/${domain}/${repo}

**Agent:** codex
**Model:** ${MODEL}
**Style Mode:** ${STYLE_MODE}
**Feature:** ${feature}
**Generated:** ${generated}
**Trigger:** ${trigger}
**Previous Log:** ${prev_log}

---

## TAXONOMY

| Level | Value |
|-------|-------|
| Owner | ${owner} |
| Client | ${client} |
| Domain | ${domain} |
| Repo | ${repo} |
| Feature | ${feature} |
| Agent | codex |
| Transcript | \`${transcript_ref}\` |

---

## TRANSCRIPT HISTORY

**Cumulative list of all transcripts for this feature.**

| Date | Session | Agent | Transcript Reference | Size |
|------|---------|-------|---------------------|------|
$(printf '%b' "$history_rows")

---

## CONTEXT SUMMARY

### Goal
${goal_text}

### Key Decisions
1. Use standardized cross-agent markdown format with \`_codex\` filename suffix.
2. Resolve taxonomy from \`.codex/taxonomy.json\`, then \`.claude/taxonomy.json\`, then git/directory fallback.
3. Persist native transcript references so future sessions can recover full details.

### What Was Built/Changed
- Generated session log: \`${log_path}\`
- Used native transcript: \`${transcript_ref}\`
- Captured working tree snapshot: ${changed_files}

### What Works
- Filename format matches \`<owner>--<client>_<domain>_<repo>_<feature>_<YYYYMMDD>_v<N>_codex.md\`.
- Version increments per day/feature/agent.
- Required markdown sections are emitted in fixed order.

### What Doesn't Work / Known Issues
- Context summary is deterministic transcript extraction, not semantic deep summarization.

### Current State
Session log generated successfully; next session can resume from this handoff.

### Critical Context
- Native transcript path: \`${transcript_path}\`
- Project root: \`${project_root}\`
- Taxonomy source: \`${taxonomy_file:-fallback}\`

---

## SESSION ACTIVITY LOG

| Time | Action | Outcome |
|------|--------|---------|
| ${time_hm} | Resolved taxonomy | owner=${owner}, client=${client}, domain=${domain}, repo=${repo}, feature=${feature} |
| ${time_hm} | Located transcript | \`${transcript_ref}\` (${transcript_size}) |
| ${time_hm} | Built transcript history | Included existing matching logs and this session row |
| ${time_hm} | Wrote markdown log | \`${filename}\` |

---

## INSTRUCTIONS FOR NEXT SESSION

1. Read the CONTEXT SUMMARY above
2. Check previous logs for full history
3. If details missing, consult native transcript at \`${transcript_ref}\`
4. Continue from where you left off
LOG

commit_result="skipped"
if [[ "$do_commit" == "1" ]] && git -C "$project_root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  git -C "$project_root" add "$log_path"
  msg="session(codex): ${feature} v${next_v} - session summary"
  if git -C "$project_root" commit -m "$msg" >/dev/null 2>&1; then
    commit_result="committed"
  else
    commit_result="no-op"
  fi
fi

printf 'Created: %s\n' "$log_path"
printf 'Trigger: %s\n' "$trigger"
printf 'Transcript: %s\n' "$transcript_ref"
printf 'Git commit: %s\n' "$commit_result"
