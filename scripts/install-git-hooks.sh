#!/bin/bash
# Install repo hooks into the local .git/hooks directory.
# Run once after cloning. Idempotent — safe to re-run.
set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)
HOOKS_DIR="$REPO_ROOT/.git/hooks"
mkdir -p "$HOOKS_DIR"

install_hook() {
    local src="$1"
    local dest="$2"
    if [ ! -f "$src" ]; then
        echo "missing: $src"; exit 1
    fi
    ln -sf "$src" "$dest"
    chmod +x "$src"
    echo "installed: $(basename "$dest") → $src"
}

# pre-commit: version drift check on Cargo.toml
install_hook "$REPO_ROOT/scripts/version-drift-check.sh" "$HOOKS_DIR/pre-commit"

# pre-push: cargo check + clippy gate (mirrors Rust CI Check & Lint)
install_hook "$REPO_ROOT/scripts/pre-push-ci-checks.sh" "$HOOKS_DIR/pre-push"

echo ""
echo "Hooks installed. Tip: use 'gh pr merge --auto --squash' so PRs only"
echo "merge once CI is green — branch protection isn't available on free-tier."
