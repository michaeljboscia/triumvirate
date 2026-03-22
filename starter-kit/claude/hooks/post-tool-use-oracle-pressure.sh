#!/bin/bash
# Oracle Pressure Hook — FEAT-033
#
# Fires after every tool call. Every 5 calls, checks oracle pressure for
# any active oracle in the current project. Emits a warning if checkpoint
# is recommended.
#
# Active oracle discovery:
#   1. Check <project_root>/.pythia-active/ for oracle JSON files
#   2. Fallback: scan registry for longest project_root prefix match
#
# Skip conditions:
#   - No active oracles found
#   - Oracle status === "decommissioned"
#   - Pressure check fails (log warning, don't block)
#   - call_count % 5 != 0

INPUT=$(cat)
CWD=$(echo "$INPUT" | jq -r '.cwd // empty')

# State file for call count tracking
STATE_FILE="$HOME/.claude/oracle-pressure-state.json"

# Read current count
if [ -f "$STATE_FILE" ]; then
    CALL_COUNT=$(jq -r '.call_count // 0' "$STATE_FILE" 2>/dev/null || echo "0")
else
    CALL_COUNT=0
fi

# Increment count
CALL_COUNT=$((CALL_COUNT + 1))

# Write updated count (atomic)
TMPFILE=$(mktemp)
jq -n --argjson count "$CALL_COUNT" '{"call_count": $count}' > "$TMPFILE" && mv "$TMPFILE" "$STATE_FILE"

# Only fire every 5 calls
if [ $((CALL_COUNT % 5)) -ne 0 ]; then
    exit 0
fi

# Need a working directory
if [ -z "$CWD" ]; then
    exit 0
fi

# Find the project root (git root or cwd)
PROJECT_ROOT=$(git -C "$CWD" rev-parse --show-toplevel 2>/dev/null)
if [ -z "$PROJECT_ROOT" ]; then
    PROJECT_ROOT="$CWD"
fi

REGISTRY="$HOME/pythia/registry.json"

# Discover active oracles
ORACLE_NAMES=()

# Method 1: .pythia-active/ directory
ACTIVE_DIR="$PROJECT_ROOT/.pythia-active"
if [ -d "$ACTIVE_DIR" ]; then
    for f in "$ACTIVE_DIR"/*.json; do
        [ -f "$f" ] || continue
        NAME=$(jq -r '.oracle_name // empty' "$f" 2>/dev/null)
        [ -n "$NAME" ] && ORACLE_NAMES+=("$NAME")
    done
fi

# Method 2: Registry prefix match (fallback when no .pythia-active/)
if [ ${#ORACLE_NAMES[@]} -eq 0 ] && [ -f "$REGISTRY" ]; then
    # Find oracles where project_root is a prefix of PROJECT_ROOT
    MATCHED=$(jq -r --arg cwd "$PROJECT_ROOT" '
        .oracles // {} |
        to_entries[] |
        select(.value.decommissioned_at == null) |
        select($cwd | startswith(.value.project_root)) |
        .key
    ' "$REGISTRY" 2>/dev/null)
    while IFS= read -r name; do
        [ -n "$name" ] && ORACLE_NAMES+=("$name")
    done <<< "$MATCHED"
fi

# No active oracles — exit silently
if [ ${#ORACLE_NAMES[@]} -eq 0 ]; then
    exit 0
fi

# Run pressure check for each active oracle via MCP
# We use a simple jq-based approach: read state.json directly
# (avoids spawning a full MCP client just for a quick check)

WARNINGS=""

for ORACLE_NAME in "${ORACLE_NAMES[@]}"; do
    # Find oracle_dir from registry
    if [ ! -f "$REGISTRY" ]; then
        continue
    fi

    ORACLE_DIR=$(jq -r --arg name "$ORACLE_NAME" '
        .oracles[$name].oracle_dir // empty
    ' "$REGISTRY" 2>/dev/null)

    if [ -z "$ORACLE_DIR" ] || [ ! -d "$ORACLE_DIR" ]; then
        continue
    fi

    # Skip decommissioned
    STATUS=$(jq -r '.status // "idle"' "$ORACLE_DIR/state.json" 2>/dev/null)
    if [ "$STATUS" = "decommissioned" ]; then
        continue
    fi

    # Read pool state to find tokens_remaining
    # Use MAX across pool members (same logic as oracle_pressure_check)
    TOKENS_REMAINING=$(jq '
        .daemon_pool |
        if length == 0 then null
        else
            map(.chars_in // 0) |
            max as $max_chars_in |
            # Estimate: assume 2M token context (pro), subtract chars/4
            (2000000 - ($max_chars_in / 4 | floor))
        end
    ' "$ORACLE_DIR/state.json" 2>/dev/null)

    if [ -z "$TOKENS_REMAINING" ] || [ "$TOKENS_REMAINING" = "null" ]; then
        continue
    fi

    # Check headroom: < 10% of 2M = < 200K → checkpoint_now
    if [ "$TOKENS_REMAINING" -lt 200000 ] 2>/dev/null; then
        WARNINGS="$WARNINGS\n⚠️  Oracle '$ORACLE_NAME': low context headroom (~${TOKENS_REMAINING} tokens). Run /pythia checkpoint."
    fi
done

# Emit warning if needed
if [ -n "$WARNINGS" ]; then
    WALL_TIME=$(TZ='America/New_York' date '+%H:%M %Z')
    WARNING_TEXT=$(printf '%s' "$WARNINGS" | sed 's/\\n/\n/g')
    jq -n --arg wt "$WALL_TIME" --arg warn "$WARNING_TEXT" '{
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": ("🕐 " + $wt + " — PYTHIA PRESSURE WARNING:\n" + $warn + "\n\nCall oracle_pressure_check for precise metrics.")
        }
    }'
fi

exit 0
