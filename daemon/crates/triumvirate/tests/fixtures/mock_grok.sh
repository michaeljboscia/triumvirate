#!/usr/bin/env bash
# Mock grok CLI for offline integration tests. Replays a captured NDJSON turn.
#
# Honors the flags Triumvirate owns so the tests exercise real argv handling:
#   -p / --resume / --session-id / --output-format / --sandbox
#
# Behavior is scripted through env vars so one script covers every case:
#   MOCK_GROK_EXIT           exit code (default 0)
#   MOCK_GROK_MODE           normal | max_turns | error | no_end | sandbox_fail | auth_fail
#   MOCK_GROK_ARGS_OUT       if set, the full argv is written there for assertions
set -u

[ -n "${MOCK_GROK_ARGS_OUT:-}" ] && printf '%s\n' "$@" > "$MOCK_GROK_ARGS_OUT"

SESSION=""
PROMPT=""
prev=""
for a in "$@"; do
  case "$prev" in
    --session-id|-s) SESSION="$a" ;;
    --resume|-r)     SESSION="$a" ;;
    -p|--single)     PROMPT="$a" ;;
  esac
  prev="$a"
done
[ -z "$SESSION" ] && SESSION="mock-session-0001"

MODE="${MOCK_GROK_MODE:-normal}"

# An unknown sandbox profile warns on stderr and runs UNCONTAINED. The runner must
# treat this as fatal, so the mock can reproduce it.
if [ "$MODE" = "sandbox_fail" ]; then
  echo "warning: sandbox could not be applied: Custom sandbox profile 'nope' not found." >&2
fi

if [ "$MODE" = "auth_fail" ]; then
  echo "error: 401 unauthorized: run grok login" >&2
  exit 1
fi

echo '{"type":"available_commands","tools":["read_file","grep"],"commands":[]}'
echo '{"type":"thought","data":"CHAIN OF THOUGHT MUST NOT LEAK"}'
echo '{"type":"tool_call","toolCallId":"c1","toolName":"read_file","kind":"read","status":"in_progress","rawInput":{"path":"src/main.rs"}}'
echo '{"type":"tool_call_update","toolCallId":"c1","status":"completed","rawOutput":{"lines":42}}'

if [ "$MODE" = "error" ]; then
  echo '{"type":"error","message":"mock grok failure"}'
  exit "${MOCK_GROK_EXIT:-1}"
fi

echo "{\"type\":\"text\",\"data\":\"pong from ${PROMPT:-nothing}\"}"

if [ "$MODE" = "max_turns" ]; then
  echo '{"type":"max_turns_reached"}'
  exit "${MOCK_GROK_EXIT:-0}"
fi

if [ "$MODE" = "no_end" ]; then
  exit "${MOCK_GROK_EXIT:-0}"
fi

echo '{"type":"usage","messageId":"m1","stopReason":"end_turn","usage":{"input_tokens":14386,"output_tokens":31,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"reasoning_tokens":4}}'
echo "{\"type\":\"end\",\"stopReason\":\"end_turn\",\"sessionId\":\"${SESSION}\",\"requestId\":\"r1\",\"usage\":{\"input_tokens\":14386,\"output_tokens\":31,\"cache_read_input_tokens\":0,\"reasoning_tokens\":4,\"total_tokens\":14421},\"num_turns\":1,\"modelUsage\":{\"grok-4.6-build\":{\"inputTokens\":14386,\"outputTokens\":31,\"costUSD\":0.00493}},\"total_cost_usd\":0.00493}"
exit "${MOCK_GROK_EXIT:-0}"
