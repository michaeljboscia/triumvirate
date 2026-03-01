#!/bin/bash
# detect-context.sh - Auto-detect inter-agent protocol context fields
# Used by send-to-claude skill

set -euo pipefail

# Detect REPO (git remote or current directory)
detect_repo() {
    git remote get-url origin 2>/dev/null || pwd
}

# Detect BRANCH (current git branch)
detect_branch() {
    git branch --show-current 2>/dev/null || echo "main"
}

# Detect CWD (absolute path)
detect_cwd() {
    pwd
}

# Detect TAXONOMY (from taxonomy.json, git remote, or directory)
detect_taxonomy() {
    local cwd="$1"

    # Try .claude/taxonomy.json first
    if [[ -f "$cwd/.claude/taxonomy.json" ]]; then
        jq -r '[.owner, .client, .domain, .repo] | join("/")' "$cwd/.claude/taxonomy.json" 2>/dev/null && return
    fi

    # Try git remote parsing
    local remote
    remote=$(git remote get-url origin 2>/dev/null || echo "")
    if [[ -n "$remote" ]]; then
        # Extract from github.com/owner/repo.git format
        if [[ "$remote" =~ github\.com[:/]([^/]+)/([^/\.]+) ]]; then
            local owner="${BASH_REMATCH[1]}"
            local repo="${BASH_REMATCH[2]}"
            echo "$owner/project/general/$repo"
            return
        fi
    fi

    # Fallback: use directory structure
    local dirname
    dirname=$(basename "$cwd")
    echo "unknown/project/general/$dirname"
}

# Detect or create SESSION_LOG
detect_session_log() {
    local cwd="$1"
    local session_log_dir="$cwd/session-logs"

    # Check if session-logs directory exists
    if [[ -d "$session_log_dir" ]]; then
        # Find most recent session log for claude
        local latest
        latest=$(find "$session_log_dir" -name "*_claude.md" -type f 2>/dev/null | sort -r | head -1)
        if [[ -n "$latest" ]]; then
            # Return relative path from repo root
            echo "${latest#$cwd/}"
            return
        fi
    fi

    # No existing log found - generate new filename
    mkdir -p "$session_log_dir"

    local taxonomy
    taxonomy=$(detect_taxonomy "$cwd")

    # Parse taxonomy for filename components
    IFS='/' read -r owner client domain repo <<< "$taxonomy"

    local date
    date=$(TZ='America/New_York' date +%Y%m%d)

    local feature="inter-agent-request"

    # Generate filename
    echo "session-logs/${owner}--${client}_${domain}_${repo}_${feature}_${date}_v1_claude.md"
}

# Generate TIMESTAMP
generate_timestamp() {
    # macOS date doesn't support %:z, so we format it manually
    local raw_timestamp
    raw_timestamp=$(TZ='America/New_York' date '+%Y-%m-%dT%H:%M:%S%z')
    # Insert colon in timezone: -0500 -> -05:00
    echo "${raw_timestamp:0:22}:${raw_timestamp:22}"
}

# Main function - outputs all fields as JSON
main() {
    local cwd
    cwd=$(detect_cwd)

    local repo
    repo=$(detect_repo)

    local branch
    branch=$(detect_branch)

    local taxonomy
    taxonomy=$(detect_taxonomy "$cwd")

    local session_log
    session_log=$(detect_session_log "$cwd")

    local timestamp
    timestamp=$(generate_timestamp)

    # Output as JSON
    cat <<EOF
{
  "from": "claude",
  "to": "gemini",
  "timestamp": "$timestamp",
  "repo": "$repo",
  "branch": "$branch",
  "session_log": "$session_log",
  "taxonomy": "$taxonomy",
  "cwd": "$cwd"
}
EOF
}

# Run if executed directly
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
