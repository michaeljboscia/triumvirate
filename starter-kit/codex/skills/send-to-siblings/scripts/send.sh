#!/bin/bash
# send.sh - Main script for send-to-siblings skill
# Fires send-to-gemini AND send-to-codex in parallel, waits for both responses.
# This is the dual-escalation pattern — use when you've failed 3 times on a problem.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILLS_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"

# Forward all args to both siblings
GEMINI_SCRIPT="$SKILLS_DIR/send-to-gemini/scripts/send.sh"
CODEX_SCRIPT="$SKILLS_DIR/send-to-codex/scripts/send.sh"

if [[ ! -x "$GEMINI_SCRIPT" ]]; then
    echo "❌ send-to-gemini skill not found at: $GEMINI_SCRIPT" >&2
    exit 1
fi
if [[ ! -x "$CODEX_SCRIPT" ]]; then
    echo "❌ send-to-codex skill not found at: $CODEX_SCRIPT" >&2
    exit 1
fi

# Temp files for capturing parallel output
GEMINI_OUT="$(mktemp)"
CODEX_OUT="$(mktemp)"
trap "rm -f '$GEMINI_OUT' '$CODEX_OUT'" EXIT

cat >&2 <<EOF
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📨 SENDING TO BOTH SIBLINGS IN PARALLEL
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
This is the dual-escalation pattern.
Both Gemini and Codex will receive your question simultaneously.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
EOF

# Fire both in parallel
"$GEMINI_SCRIPT" "$@" > "$GEMINI_OUT" 2>&1 &
GEMINI_PID=$!

"$CODEX_SCRIPT" "$@" > "$CODEX_OUT" 2>&1 &
CODEX_PID=$!

echo "⏳ Waiting for Gemini and Codex..." >&2

# Wait for both and capture exit codes
GEMINI_EXIT=0
CODEX_EXIT=0
wait "$GEMINI_PID" || GEMINI_EXIT=$?
wait "$CODEX_PID"  || CODEX_EXIT=$?

# Print responses
echo ""
cat <<EOF
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📥 GEMINI RESPONSE $([ "$GEMINI_EXIT" -ne 0 ] && echo "(exit $GEMINI_EXIT)" || echo "")
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
EOF
cat "$GEMINI_OUT"

echo ""
cat <<EOF
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📥 CODEX RESPONSE $([ "$CODEX_EXIT" -ne 0 ] && echo "(exit $CODEX_EXIT)" || echo "")
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
EOF
cat "$CODEX_OUT"

# Exit non-zero if both failed
if [[ "$GEMINI_EXIT" -ne 0 && "$CODEX_EXIT" -ne 0 ]]; then
    exit 1
fi
