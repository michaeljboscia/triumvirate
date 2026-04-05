#!/usr/bin/env bash
set -euo pipefail

MODE="mock"
SKIP_GATES="0"
PORT="${TRIUMVIRATE_TEST_PORT:-8099}"

for arg in "$@"; do
  case "$arg" in
    --live) MODE="live" ;;
    --mock) MODE="mock" ;;
    --skip-gates) SKIP_GATES="1" ;;
    *)
      echo "Unknown arg: $arg"
      echo "Usage: $0 [--mock|--live] [--skip-gates]"
      exit 1
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DAEMON_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
FRONTEND_DIR="$DAEMON_DIR/frontend"
ROOT_DIR="$(cd "$DAEMON_DIR/.." && pwd)"
BIN="$DAEMON_DIR/target/release/triumvirate-agentd"
MOCK_CLAUDE="$DAEMON_DIR/target/release/mock-claude"
MOCK_GEMINI="$DAEMON_DIR/target/release/mock-gemini"
MOCK_CODEX="$DAEMON_DIR/target/release/mock-codex"

fail() {
  echo "[FAIL] $1" >&2
  exit 1
}

wait_for_health() {
  local i
  for i in {1..80}; do
    if curl -sf "http://127.0.0.1:${PORT}/api/health" >/tmp/rc1-health.json 2>/dev/null; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

echo "==> RC1 test start (mode: ${MODE}, port: ${PORT})"

if [[ "$SKIP_GATES" != "1" ]]; then
  echo "==> Running frontend + Rust gates"
  (
    cd "$FRONTEND_DIR"
    npm install
    npm run build
    npm audit
  )
  (
    cd "$DAEMON_DIR"
    cargo check
    cargo test
    cargo clippy -- -D warnings
    cargo build
    cargo build --release
  )
fi

[[ -x "$BIN" ]] || fail "release binary missing: $BIN"

RUN_ID="rc1_$(date +%Y%m%d_%H%M%S)"
TMPHOME="$(mktemp -d)"
DB_PATH="$TMPHOME/.triumvirate/memory.db"
mkdir -p "$TMPHOME/.triumvirate"

cat > "$TMPHOME/.triumvirate/config.toml" <<CFG
web_port = ${PORT}
db_path = "${DB_PATH}"
[agents]
claude_enabled = true
gemini_enabled = true
codex_enabled = true
CFG

if [[ "$MODE" == "mock" ]]; then
  [[ -x "$MOCK_CLAUDE" ]] || fail "missing mock binary: $MOCK_CLAUDE"
  [[ -x "$MOCK_GEMINI" ]] || fail "missing mock binary: $MOCK_GEMINI"
  [[ -x "$MOCK_CODEX" ]] || fail "missing mock binary: $MOCK_CODEX"
  export TRIUMVIRATE_CLAUDE_BIN="$MOCK_CLAUDE"
  export TRIUMVIRATE_GEMINI_BIN="$MOCK_GEMINI"
  export TRIUMVIRATE_CODEX_BIN="$MOCK_CODEX"
  echo "==> Using mock connectors"
else
  unset TRIUMVIRATE_CLAUDE_BIN || true
  unset TRIUMVIRATE_GEMINI_BIN || true
  unset TRIUMVIRATE_CODEX_BIN || true
  echo "==> Using live connectors from PATH"
fi

export HOME="$TMPHOME"
LOG_FILE="$TMPHOME/daemon.log"

echo "==> Starting daemon"
(
  cd "$ROOT_DIR"
  "$BIN" >"$LOG_FILE" 2>&1 &
)
PID="$(pgrep -f "target/release/triumvirate-agentd" | tail -n 1)"
[[ -n "$PID" ]] || fail "could not find daemon pid"

cleanup() {
  kill "$PID" >/dev/null 2>&1 || true
  wait "$PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

wait_for_health || {
  tail -n 120 "$LOG_FILE" || true
  fail "health endpoint did not become ready"
}

echo "==> Scenario A: health/surface"
grep -q '"status":"ok"' /tmp/rc1-health.json || fail "health status not ok"
curl -sf "http://127.0.0.1:${PORT}/metrics" >/tmp/rc1-metrics.txt
grep -q "# HELP" /tmp/rc1-metrics.txt || fail "metrics not in Prometheus format"
curl -sf "http://127.0.0.1:${PORT}/api/costs" >/tmp/rc1-costs.json
grep -q "estimated_total_cost_usd" /tmp/rc1-costs.json || fail "costs payload missing summary"
curl -sf "http://127.0.0.1:${PORT}/api/lessons" >/tmp/rc1-lessons-empty.json

echo "==> Scenario B: routing"
curl -sf -X POST "http://127.0.0.1:${PORT}/api/message" -H 'Content-Type: application/json' -d '{"content":"hello from rc1"}' >/dev/null
curl -sf -X POST "http://127.0.0.1:${PORT}/api/message" -H 'Content-Type: application/json' -d '{"content":"@claude architecture check"}' >/dev/null
curl -sf -X POST "http://127.0.0.1:${PORT}/api/message" -H 'Content-Type: application/json' -d '{"content":"@codex implementation check"}' >/dev/null
sleep 1
ROUTES="$(sqlite3 "$DB_PATH" "select count(*) from routing_log;")"
[[ "${ROUTES}" -ge 2 ]] || fail "routing_log did not capture expected rows"

echo "==> Scenario C: debate lifecycle"
DEBATE_START="$(curl -sf -X POST "http://127.0.0.1:${PORT}/api/debate/start" -H 'Content-Type: application/json' -d '{"topic":"redis vs postgres"}')"
WID="$(echo "$DEBATE_START" | sed -n 's/.*"workflow_id":"\([^"]*\)".*/\1/p')"
[[ -n "$WID" ]] || fail "debate start did not return workflow_id"
curl -sf -X POST "http://127.0.0.1:${PORT}/api/debate/challenge" -H 'Content-Type: application/json' -d "{\"workflow_id\":\"${WID}\",\"challenger\":\"claude\",\"argument\":\"prefer postgres\"}" >/dev/null
curl -sf -X POST "http://127.0.0.1:${PORT}/api/debate/vote" -H 'Content-Type: application/json' -d "{\"workflow_id\":\"${WID}\",\"voter\":\"gemini\",\"vote\":\"postgres\"}" >/dev/null
curl -sf -X POST "http://127.0.0.1:${PORT}/api/debate/vote" -H 'Content-Type: application/json' -d "{\"workflow_id\":\"${WID}\",\"voter\":\"codex\",\"vote\":\"postgres\"}" >/dev/null
curl -sf -X POST "http://127.0.0.1:${PORT}/api/debate/complete" -H 'Content-Type: application/json' -d "{\"workflow_id\":\"${WID}\",\"decision\":\"postgres\"}" >/dev/null

echo "==> Scenario D: fleet dependency lifecycle"
FLEET_START="$(curl -sf -X POST "http://127.0.0.1:${PORT}/api/fleet/spawn" -H 'Content-Type: application/json' -d '{"spec":"1 codex: e2e task"}')"
FID="$(echo "$FLEET_START" | sed -n 's/.*"fleet_id":"\([^"]*\)".*/\1/p')"
[[ -n "$FID" ]] || fail "fleet spawn did not return fleet_id"
curl -sf "http://127.0.0.1:${PORT}/api/fleet/tasks" >/tmp/rc1-tasks.json
grep -q '"task_key":"contracts"' /tmp/rc1-tasks.json || fail "missing contracts task"
grep -q '"task_key":"implementation"' /tmp/rc1-tasks.json || fail "missing implementation task"

BLOCK_CODE="$(curl -s -o /tmp/rc1-claim-blocked.json -w "%{http_code}" -X POST "http://127.0.0.1:${PORT}/api/fleet/tasks/claim" -H 'Content-Type: application/json' -d "{\"fleet_id\":\"${FID}\",\"task_key\":\"implementation\",\"agent\":\"codex-1\"}")"
[[ "$BLOCK_CODE" != "200" ]] || fail "implementation should be blocked before contracts completion"

curl -sf -X POST "http://127.0.0.1:${PORT}/api/fleet/tasks/claim" -H 'Content-Type: application/json' -d "{\"fleet_id\":\"${FID}\",\"task_key\":\"contracts\",\"agent\":\"codex-1\"}" >/dev/null
curl -sf -X POST "http://127.0.0.1:${PORT}/api/fleet/tasks/complete" -H 'Content-Type: application/json' -d "{\"fleet_id\":\"${FID}\",\"task_key\":\"contracts\"}" >/dev/null
curl -sf -X POST "http://127.0.0.1:${PORT}/api/fleet/tasks/claim" -H 'Content-Type: application/json' -d "{\"fleet_id\":\"${FID}\",\"task_key\":\"implementation\",\"agent\":\"codex-1\"}" >/dev/null
curl -sf "http://127.0.0.1:${PORT}/api/fleet/status/${FID}" >/tmp/rc1-fleet-status.json

echo "==> Scenario E: governance gate"
DENY_CODE="$(curl -s -o /tmp/rc1-merge-deny.json -w "%{http_code}" -X POST "http://127.0.0.1:${PORT}/api/fleet/merge" -H 'Content-Type: application/json' -d "{\"fleet_id\":\"${FID}\",\"human_approved\":false}")"
[[ "$DENY_CODE" == "403" ]] || fail "expected governance deny 403, got ${DENY_CODE}"
ALLOW_CODE="$(curl -s -o /tmp/rc1-merge-allow.json -w "%{http_code}" -X POST "http://127.0.0.1:${PORT}/api/fleet/merge" -H 'Content-Type: application/json' -d "{\"fleet_id\":\"${FID}\",\"human_approved\":true}")"
echo "$ALLOW_CODE" | grep -Eq '^(200|409)$' || fail "expected approved merge status 200 or 409, got ${ALLOW_CODE}"

echo "==> Scenario FEAT-031: lessons create/filter"
curl -sf -X POST "http://127.0.0.1:${PORT}/api/lessons" -H 'Content-Type: application/json' -d '{"decision":"Avoid cwd assumptions","rationale":"Fleet spawn can run outside git cwd","outcome":"failure","confidence_score":0.9,"pattern":"fleet_worktree","agent_source":"system"}' >/tmp/rc1-lesson-create.json
curl -sf "http://127.0.0.1:${PORT}/api/lessons?outcome=failure&pattern=fleet_worktree&min_confidence=0.5" >/tmp/rc1-lesson-filter.json
grep -q "fleet_worktree" /tmp/rc1-lesson-filter.json || fail "lesson filtering did not return expected row"

echo "==> Scenario F: restart recovery"
OPEN_DEBATE="$(curl -sf -X POST "http://127.0.0.1:${PORT}/api/debate/start" -H 'Content-Type: application/json' -d '{"topic":"restart recovery"}')"
OPEN_WID="$(echo "$OPEN_DEBATE" | sed -n 's/.*"workflow_id":"\([^"]*\)".*/\1/p')"
[[ -n "$OPEN_WID" ]] || fail "recovery setup did not return workflow_id"
kill "$PID" >/dev/null 2>&1 || true
wait "$PID" >/dev/null 2>&1 || true
(
  cd "$ROOT_DIR"
  "$BIN" >"$LOG_FILE" 2>&1 &
)
PID="$(pgrep -f "target/release/triumvirate-agentd" | tail -n 1)"
wait_for_health || fail "daemon did not return after restart"
curl -sf "http://127.0.0.1:${PORT}/api/workflows" >/tmp/rc1-workflows.json
grep -q "$OPEN_WID" /tmp/rc1-workflows.json || fail "resumable workflow missing after restart"

echo "==> RC1 test PASS"
echo "RUN_ID=$RUN_ID"
echo "MODE=$MODE"
echo "TMPHOME=$TMPHOME"
