#!/usr/bin/env bash
# Run the LIVE agent tests: the ones that catch a vendor changing its wire format.
#
# WHY THIS EXISTS. The offline suite runs against captured fixtures. Fixtures go stale. If xAI
# or Google changes a stream shape tomorrow, every offline test stays green while production
# breaks, because a parser that records nothing looks exactly like an agent that did nothing.
#
# That is not hypothetical here. agy shipped for months emitting no tool calls at all, which
# made Antigravity structurally unable to satisfy the sight gate, and nothing failed.
#
# These tests are #[ignore] by default because they spend subscription quota and need the real
# binaries. Nothing runs them automatically. This script is the deliberate one-command version,
# so "run the live guards" is not a research project.
#
# Usage:
#   bash scripts/verify-live-agents.sh          # all live guards
#   bash scripts/verify-live-agents.sh agy      # just the agy ones
#   bash scripts/verify-live-agents.sh grok
#
# Exit non-zero if any guard fails. Safe to wire into a scheduled job.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHICH="${1:-all}"
cd "$REPO_ROOT/daemon"

FAILED=0

run_guard() {
    local label="$1" env_var="$2" bin="$3"
    shift 3
    if ! command -v "$bin" >/dev/null 2>&1; then
        echo "SKIP  $label: '$bin' is not on PATH"
        return 0
    fi
    echo "RUN   $label"
    if env "$env_var=1" cargo test "$@" -- --ignored --nocapture 2>&1 | tail -20; then
        echo "PASS  $label"
    else
        echo "FAIL  $label"
        FAILED=1
    fi
}

if [ "$WHICH" = "all" ] || [ "$WHICH" = "agy" ]; then
    # Proves: live agy emits tool events, the argv Triumvirate BUILDS produces a parseable
    # stream, and a real agy turn clears the real sight gate. The middle one is the guard that
    # would have caught the stream-json flag landing on only one of two invocation builders.
    run_guard "agy stream + gate" TRIUMVIRATE_LIVE_AGY agy \
        -p triumvirate --test integration_agy_sight
    run_guard "agy clears the real gate" TRIUMVIRATE_LIVE_AGY agy \
        -p triumvirate --bin triumvirate sight_25
fi

if [ "$WHICH" = "all" ] || [ "$WHICH" = "grok" ]; then
    # Proves: live grok records the file it opened, and lands on a parser mode the gate trusts.
    run_guard "grok stream + sight" TRIUMVIRATE_LIVE_GROK grok \
        -p triumvirate --test integration_grok
fi

if [ "$FAILED" -eq 0 ]; then
    echo ""
    echo "verify-live-agents: all guards passed"
else
    echo ""
    echo "verify-live-agents: A GUARD FAILED."
    echo "A live failure with a green offline suite means a vendor changed its wire format."
    echo "Recapture the fixtures before editing the parser to match: the fixtures are the"
    echo "evidence, and a parser tuned to a belief about the format is how this broke before."
    exit 1
fi
