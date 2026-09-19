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
#   bash scripts/verify-live-agents.sh codex
#   bash scripts/verify-live-agents.sh review   # mandatory peer review, mock reviewer, no network
#   bash scripts/verify-live-agents.sh strict   # strict_agent never substitutes, mock binaries, no network
#   bash scripts/verify-live-agents.sh guards   # git hooks are armed, not merely present (D-009)
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

if [ "$WHICH" = "all" ] || [ "$WHICH" = "review" ]; then
    # Mandatory peer review, end to end, with a MOCK reviewer. No network, no API key.
    #
    # Single-threaded and opt-in because these set TRIUMVIRATE_REQUIRE_PEER_REVIEW and a mock
    # connector binary, both of which change every dispatch in that test binary. Under the
    # default parallel harness they failed about one run in three, and the failure surfaced in
    # unrelated tests as "REJECTED by peer review".
    #
    # These are the tests that prove the gate is a gate: that a reviewer is really spawned,
    # that REJECT really blocks, that an unreadable answer blocks, and that a review is not
    # itself reviewed.
    echo "RUN   mandatory review end to end"
    if cargo test -p triumvirate --bin triumvirate mandatory_review_tests \
        -- --ignored --test-threads=1 2>&1 | tail -12; then
        echo "PASS  mandatory review end to end"
    else
        echo "FAIL  mandatory review end to end"
        FAILED=1
    fi
fi

if [ "$WHICH" = "all" ] || [ "$WHICH" = "guards" ]; then
    # D-009: every git hook is ARMED (git itself will run it), and the checker can fail. Run from
    # here and from the installer, never from a hook: a hook cannot report that hooks are dead.
    echo "RUN   guards are armed, and the checker can fail"
    if bash "$REPO_ROOT/scripts/verify-guards.sh" --self-test && bash "$REPO_ROOT/scripts/verify-guards.sh"; then
        echo "PASS  guards are armed, and the checker can fail"
    else
        echo "FAIL  guards are armed, and the checker can fail"
        FAILED=1
    fi
fi

if [ "$WHICH" = "all" ] || [ "$WHICH" = "strict" ]; then
    # strict_agent, end to end, with MOCK agy and codex binaries. No network, no quota.
    #
    # Single-threaded and opt-in for the same reason as the review guards: the mocks replace
    # the agy and codex binaries for every dispatch in that test binary.
    #
    # strict_01 is the negative control. It proves the fixture really does substitute codex
    # when strict is off, so strict_02 cannot be green merely because nothing degraded.
    echo "RUN   strict_agent never substitutes"
    if cargo test -p triumvirate --bin triumvirate strict_agent_tests \
        -- --ignored --test-threads=1 2>&1 | tail -12; then
        echo "PASS  strict_agent never substitutes"
    else
        echo "FAIL  strict_agent never substitutes"
        FAILED=1
    fi

    # breaker_probe against the REAL process-global breaker: a healthy probe closes an open
    # breaker, and repeated failed probes never extend its cooldown.
    echo "RUN   breaker_probe closes on health, never extends on failure"
    if cargo test -p triumvirate --bin triumvirate breaker_probe_tests \
        -- --ignored --test-threads=1 2>&1 | tail -12; then
        echo "PASS  breaker_probe closes on health, never extends on failure"
    else
        echo "FAIL  breaker_probe closes on health, never extends on failure"
        FAILED=1
    fi
fi

if [ "$WHICH" = "all" ] || [ "$WHICH" = "agy" ]; then
    # Proves: live agy emits tool events, the argv Triumvirate BUILDS produces a parseable
    # stream, and a real agy turn clears the real sight gate. The middle one is the guard that
    # would have caught the stream-json flag landing on only one of two invocation builders.
    run_guard "agy stream + gate" TRIUMVIRATE_LIVE_AGY agy \
        -p triumvirate --test integration_agy_sight
    run_guard "agy clears the real gate" TRIUMVIRATE_LIVE_AGY agy \
        -p triumvirate --bin triumvirate sight_25
    # THE CONTAINMENT PROOF. Grok pointed out this was missing from the runner: the one guard
    # that proves the sandbox actually DENIES a write was not among the guards that get run.
    run_guard "agy containment (denied write)" TRIUMVIRATE_LIVE_AGY agy \
        -p triumvirate --bin triumvirate sight_27
fi

if [ "$WHICH" = "all" ] || [ "$WHICH" = "codex" ]; then
    # Proves: a live codex turn reading a file produces a record the sight gate accepts.
    # This is the guard that caught codex wrapping every command in `/bin/zsh -lc '...'`,
    # which no offline test saw because they all used the shape I assumed.
    run_guard "codex source gating" TRIUMVIRATE_LIVE_CODEX codex \
        -p triumvirate --bin triumvirate sight_28
    # Codex containment. Triumvirate launches codex --dangerously-bypass-approvals-and-sandbox
    # by default, so a review MUST override that with --sandbox read-only. This guard carries a
    # control asserting codex actually ran, because the first version of it passed in 0.11s on
    # a startup failure.
    run_guard "codex containment (denied write)" TRIUMVIRATE_LIVE_CODEX codex \
        -p triumvirate --bin triumvirate sight_29
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
