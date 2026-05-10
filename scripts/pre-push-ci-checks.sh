#!/bin/bash
# pre-push: run the same fast checks as Rust CI to catch regressions
# before they reach GitHub. Mirrors `.github/workflows/rust.yml` Check & Lint.
#
# Bypass with: TRIUMVIRATE_SKIP_PRE_PUSH=1 git push
# (use rarely — bypass means the next push regression is on you)
set -euo pipefail

if [ "${TRIUMVIRATE_SKIP_PRE_PUSH:-0}" = "1" ]; then
    echo "pre-push: skipped via TRIUMVIRATE_SKIP_PRE_PUSH=1"
    exit 0
fi

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT/daemon"

echo "pre-push: cargo check --workspace --exclude pantheon..."
if ! cargo check --workspace --exclude pantheon --quiet; then
    echo ""
    echo "pre-push: FAILED on cargo check. Fix the build, or bypass with"
    echo "  TRIUMVIRATE_SKIP_PRE_PUSH=1 git push"
    exit 1
fi

echo "pre-push: cargo clippy --workspace --exclude pantheon -- -D warnings..."
if ! cargo clippy --workspace --exclude pantheon --quiet -- -D warnings; then
    echo ""
    echo "pre-push: FAILED on clippy. Fix the lints, or bypass with"
    echo "  TRIUMVIRATE_SKIP_PRE_PUSH=1 git push"
    exit 1
fi

echo "pre-push: ✓ check + clippy passed"
echo "pre-push: (note: tests still run in CI; not gated locally for push speed)"
