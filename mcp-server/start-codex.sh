#!/bin/bash
# Start the Codex inter-agent MCP server.
# Uses dirname to resolve the dist path relative to this script — no hardcoded paths.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec node "$SCRIPT_DIR/dist/codex/server.js"
