#!/bin/bash
# Install repo hooks into the local .git/hooks directory.
# Run once after cloning. Idempotent — safe to re-run.
set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)
HOOK_SRC="$REPO_ROOT/scripts/version-drift-check.sh"
HOOK_DEST="$REPO_ROOT/.git/hooks/pre-commit"

if [ ! -f "$HOOK_SRC" ]; then
    echo "missing: $HOOK_SRC"; exit 1
fi

mkdir -p "$(dirname "$HOOK_DEST")"
ln -sf "$HOOK_SRC" "$HOOK_DEST"
chmod +x "$HOOK_SRC"
echo "installed pre-commit hook → $HOOK_DEST"
