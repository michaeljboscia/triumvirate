#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${TRIUMVIRATE_URL:-http://127.0.0.1:8080}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVICE_SCRIPT="${SCRIPT_DIR}/triumvirate-service.sh"
PROJECT_DAEMON_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
LOCAL_BIN="${PROJECT_DAEMON_DIR}/target/release/triumvirate-agentd"

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

daemon_is_up() {
  curl -fsS "${BASE_URL}/api/health" >/dev/null 2>&1
}

ensure_daemon() {
  if daemon_is_up; then
    return 0
  fi

  # First choice: launchd service if installed.
  if [[ -x "${SERVICE_SCRIPT}" ]] && [[ -f "${HOME}/Library/LaunchAgents/com.triumvirate.agentd.plist" ]]; then
    "${SERVICE_SCRIPT}" start >/dev/null 2>&1 || true
  fi

  # Fallback: local detached process from release binary.
  if ! daemon_is_up; then
    if [[ ! -x "${LOCAL_BIN}" ]]; then
      (cd "${PROJECT_DAEMON_DIR}" && cargo build --release -p triumvirate-agentd --bin triumvirate-agentd >/dev/null)
    fi
    nohup "${LOCAL_BIN}" >/tmp/triumvirate_agentd.out.log 2>/tmp/triumvirate_agentd.err.log &
    disown || true
  fi

  for _ in {1..30}; do
    if daemon_is_up; then
      return 0
    fi
    sleep 1
  done

  echo "error: daemon is not reachable at ${BASE_URL}" >&2
  exit 1
}

cmd="${1:-}"
if [[ -z "${cmd}" ]]; then
  usage
  exit 1
fi
shift || true

case "${cmd}" in
  health)
    ensure_daemon
    curl -fsS "${BASE_URL}/api/health"
    echo
    ;;
  ask)
    ensure_daemon
    need_arg "$@"
    msg="$*"
    post_json "/api/message" "{\"content\":\"${msg//\"/\\\"}\"}"
    ;;
  twins)
    ensure_daemon
    need_arg "$@"
    msg="$*"
    post_json "/api/message" "{\"content\":\"@claude ${msg//\"/\\\"}\"}" &
    p1=$!
    post_json "/api/message" "{\"content\":\"@gemini ${msg//\"/\\\"}\"}" &
    p2=$!
    wait "${p1}" "${p2}"
    ;;
  debate)
    ensure_daemon
    need_arg "$@"
    topic="$*"
    post_json "/api/debate/start" "{\"topic\":\"${topic//\"/\\\"}\"}"
    ;;
  fleet)
    ensure_daemon
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
