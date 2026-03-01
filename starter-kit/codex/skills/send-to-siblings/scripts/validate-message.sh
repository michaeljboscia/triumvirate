#!/bin/bash
# validate-message.sh - Validate inter-agent protocol message has all required fields
# Used by send-to-claude skill

set -euo pipefail

# Required fields for inter-agent protocol
REQUIRED_FIELDS=(
    "from"
    "to"
    "timestamp"
    "repo"
    "branch"
    "session_log"
    "taxonomy"
    "cwd"
    "request_type"
)

# Validate that all required fields are present and non-empty
validate_fields() {
    local json="$1"
    local errors=()

    for field in "${REQUIRED_FIELDS[@]}"; do
        local value
        value=$(echo "$json" | jq -r ".$field" 2>/dev/null || echo "")

        if [[ -z "$value" ]] || [[ "$value" == "null" ]]; then
            errors+=("Missing or empty field: $field")
        fi
    done

    if [[ ${#errors[@]} -gt 0 ]]; then
        echo "❌ Validation failed:" >&2
        for error in "${errors[@]}"; do
            echo "  - $error" >&2
        done
        return 1
    fi

    echo "✅ All 9 required fields validated" >&2
    return 0
}

# Validate request type is one of the allowed values
validate_request_type() {
    local request_type="$1"
    local valid_types=("question" "review" "debug" "architecture" "other")

    for valid_type in "${valid_types[@]}"; do
        if [[ "$request_type" == "$valid_type" ]]; then
            return 0
        fi
    done

    echo "❌ Invalid request type: $request_type" >&2
    echo "   Valid types: ${valid_types[*]}" >&2
    return 1
}

# Validate session log exists or can be created
validate_session_log() {
    local session_log_path="$1"

    # If it exists, we're good
    if [[ -f "$session_log_path" ]]; then
        echo "✅ Session log exists: $session_log_path" >&2
        return 0
    fi

    # Check if parent directory exists
    local parent_dir
    parent_dir=$(dirname "$session_log_path")

    if [[ ! -d "$parent_dir" ]]; then
        echo "⚠️  Session log directory doesn't exist, will create: $parent_dir" >&2
        return 0
    fi

    echo "⚠️  Session log doesn't exist, will create: $session_log_path" >&2
    return 0
}

# Main validation function
main() {
    local json="$1"

    # Validate all required fields present
    if ! validate_fields "$json"; then
        return 1
    fi

    # Validate request type
    local request_type
    request_type=$(echo "$json" | jq -r '.request_type')
    if ! validate_request_type "$request_type"; then
        return 1
    fi

    # Validate session log
    local session_log
    session_log=$(echo "$json" | jq -r '.session_log')
    if ! validate_session_log "$session_log"; then
        return 1
    fi

    echo "✅ Message validation passed" >&2
    return 0
}

# Run if executed directly
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    if [[ $# -eq 0 ]]; then
        echo "Usage: $0 <json>" >&2
        exit 1
    fi
    main "$@"
fi
