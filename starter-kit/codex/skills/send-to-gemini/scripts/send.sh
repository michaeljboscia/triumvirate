#!/bin/bash
# send.sh - Main script for send-to-gemini skill
# Sends properly formatted inter-agent request to Gemini

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

REQUEST_TYPE="question"
CONTEXT="Full context in session log"
QUESTION=""
SESSION_LOG_OVERRIDE=""
NO_COMMIT=false
HEADLESS=false

CURRENT_AGENT="unknown"
if [[ -f ~/.gemini/GEMINI.md ]]; then
    CURRENT_AGENT="gemini"
elif [[ -f ~/.codex/AGENTS.md ]]; then
    CURRENT_AGENT="codex"
fi

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --type)       REQUEST_TYPE="$2"; shift 2 ;;
            --context)    CONTEXT="$2"; shift 2 ;;
            --session-log) SESSION_LOG_OVERRIDE="$2"; shift 2 ;;
            --no-commit)  NO_COMMIT=true; shift ;;
            --headless)   HEADLESS=true; shift ;;
            --help)       show_help; exit 0 ;;
            *)            QUESTION="$1"; shift ;;
        esac
    done
}

show_help() {
    cat <<EOF
Usage: send-to-gemini [OPTIONS] "QUESTION"

Send an inter-agent request to Gemini.

OPTIONS:
    --type TYPE           Request type: question, review, debug, architecture, other
    --context TEXT        Brief context summary
    --session-log PATH    Override session log path
    --no-commit           Skip git commit of session log
    --headless            Disable interactive prompts
    --help                Show this help

EXAMPLES:
    send-to-gemini "Research the best approach for X"
    send-to-gemini --type review "Does this architecture make sense?"
EOF
}

is_headless_mode() {
    [[ "$HEADLESS" == true ]] || [[ "${CI:-}" == "true" ]] || [[ ! -t 0 ]] || [[ ! -t 1 ]]
}

create_session_log() {
    local session_log_path="$1" taxonomy="$2"
    IFS='/' read -r owner client domain repo <<< "$taxonomy"
    mkdir -p "$(dirname "$session_log_path")"
    cat > "$session_log_path" <<EOF
# Session Log: $taxonomy

**Agent:** $CURRENT_AGENT
**Feature:** inter-agent-request
**Generated:** $(TZ='America/New_York' date '+%Y-%m-%d %H:%M:%S %Z')

---

## CONTEXT SUMMARY

Sending inter-agent request to Gemini.

---

## SESSION ACTIVITY LOG

| Time | Action | Outcome |
|------|--------|---------|
| $(TZ='America/New_York' date '+%H:%M') | Created session log | ✓ |
EOF
}

main() {
    parse_args "$@"

    if [[ -z "$QUESTION" ]]; then
        if is_headless_mode; then
            QUESTION="Please review the linked session log and provide actionable guidance."
        else
            echo "📝 Interactive mode:" >&2
            echo -n "Request type [question]: " >&2; read -r t; [[ -n "$t" ]] && REQUEST_TYPE="$t"
            echo -n "Question: " >&2; read -r QUESTION
            [[ -z "$QUESTION" ]] && { echo "❌ Question required" >&2; exit 1; }
            echo -n "Context (optional): " >&2; read -r c; [[ -n "$c" ]] && CONTEXT="$c"
        fi
    fi

    echo "🔍 Detecting context..." >&2
    local context_json
    context_json=$("$SCRIPT_DIR/detect-context.sh")
    context_json=$(echo "$context_json" | jq --arg from "$CURRENT_AGENT" --arg to "gemini" '.from = $from | .to = $to')
    context_json=$(echo "$context_json" | jq --arg rt "$REQUEST_TYPE" '. + {request_type: $rt}')
    [[ -n "$SESSION_LOG_OVERRIDE" ]] && context_json=$(echo "$context_json" | jq --arg sl "$SESSION_LOG_OVERRIDE" '.session_log = $sl')

    echo "✅ Validating..." >&2
    "$SCRIPT_DIR/validate-message.sh" "$context_json" || { echo "❌ Validation failed" >&2; exit 1; }

    local from to timestamp repo branch session_log taxonomy cwd
    from=$(echo "$context_json" | jq -r '.from')
    to=$(echo "$context_json" | jq -r '.to')
    timestamp=$(echo "$context_json" | jq -r '.timestamp')
    repo=$(echo "$context_json" | jq -r '.repo')
    branch=$(echo "$context_json" | jq -r '.branch')
    session_log=$(echo "$context_json" | jq -r '.session_log')
    taxonomy=$(echo "$context_json" | jq -r '.taxonomy')
    cwd=$(echo "$context_json" | jq -r '.cwd')

    [[ ! -f "$session_log" ]] && create_session_log "$session_log" "$taxonomy"

    if [[ "$NO_COMMIT" == false ]] && git rev-parse --git-dir > /dev/null 2>&1; then
        git add "$session_log" 2>/dev/null || true
        if ! git diff --cached --quiet 2>/dev/null; then
            git commit -m "session($CURRENT_AGENT): escalating to gemini" >/dev/null 2>&1 || true
        fi
    fi

    local message
    message=$(cat <<EOF
===INTER_AGENT_REQUEST===
FROM: $from
TO: $to
TIMESTAMP: $timestamp
REPO: $repo
BRANCH: $branch
SESSION_LOG: $session_log
TAXONOMY: $taxonomy
CWD: $cwd
REQUEST_TYPE: $REQUEST_TYPE
===END_HEADERS===

QUESTION:
$QUESTION

CONTEXT:
$CONTEXT

===END_REQUEST===
EOF
)

    cat >&2 <<EOF
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📨 SENDING TO GEMINI
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Request Type: $REQUEST_TYPE
Session Log:  $session_log
Question:     $QUESTION
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
EOF

    local gemini_bin="${GEMINI_CLI_PATH:-gemini}"
    if ! command -v "$gemini_bin" &>/dev/null; then
        echo "❌ Error: 'gemini' command not found" >&2
        echo "   Install from: github.com/google-gemini/gemini-cli" >&2
        exit 1
    fi

    echo "🚀 Sending to Gemini..." >&2
    # --output-format text disables glamour markdown rendering (ANSI escape codes)
    # which corrupts output when parsed programmatically. Always use for inter-agent calls.
    local response
    response=$(printf '%s' "$message" | "$gemini_bin" --output-format text --approval-mode yolo -p "$message" 2>&1) || \
    response=$(printf '%s' "$message" | "$gemini_bin" --output-format text -p "$message" 2>&1)

    cat <<EOF
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📥 GEMINI RESPONSE
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

$response

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
EOF
}

main "$@"
