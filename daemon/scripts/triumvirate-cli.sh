#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${TRIUMVIRATE_URL:-http://127.0.0.1:8080}"

usage() {
  cat <<'EOF'
Triumvirate v2 CLI helper

Usage:
  triumvirate-cli.sh health
  triumvirate-cli.sh ask "<message>"
  triumvirate-cli.sh twins "<message>"
  triumvirate-cli.sh debate "<topic>"
  triumvirate-cli.sh fleet "<spec>"

Examples:
  triumvirate-cli.sh ask "what changed in auth?"
  triumvirate-cli.sh twins "review this migration plan"
  triumvirate-cli.sh debate "Redis vs Postgres for caching"
  triumvirate-cli.sh fleet "1 codex: build e2e test harness"
EOF
}

need_arg() {
  if [[ $# -lt 1 ]]; then
    echo "error: missing argument" >&2
    usage
    exit 1
  fi
}

post_json() {
  local path="$1"
  local body="$2"
  curl -fsS -X POST "${BASE_URL}${path}" \
    -H 'Content-Type: application/json' \
    -d "${body}"
  echo
}

cmd="${1:-}"
if [[ -z "${cmd}" ]]; then
  usage
  exit 1
fi
shift || true

case "${cmd}" in
  health)
    curl -fsS "${BASE_URL}/api/health"
    echo
    ;;
  ask)
    need_arg "$@"
    msg="$*"
    post_json "/api/message" "{\"content\":\"${msg//\"/\\\"}\"}"
    ;;
  twins)
    need_arg "$@"
    msg="$*"
    post_json "/api/message" "{\"content\":\"@claude ${msg//\"/\\\"}\"}" &
    p1=$!
    post_json "/api/message" "{\"content\":\"@gemini ${msg//\"/\\\"}\"}" &
    p2=$!
    wait "${p1}" "${p2}"
    ;;
  debate)
    need_arg "$@"
    topic="$*"
    post_json "/api/debate/start" "{\"topic\":\"${topic//\"/\\\"}\"}"
    ;;
  fleet)
    need_arg "$@"
    spec="$*"
    post_json "/api/fleet/spawn" "{\"spec\":\"${spec//\"/\\\"}\"}"
    ;;
  *)
    echo "error: unknown command '${cmd}'" >&2
    usage
    exit 1
    ;;
esac
