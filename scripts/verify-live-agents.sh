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
#   bash scripts/verify-live-agents.sh shutdown # the daemon exits on SIGTERM, even mid-request
#   bash scripts/verify-live-agents.sh kimi     # one new-seat live guard
#   bash scripts/verify-live-agents.sh mistral
#   bash scripts/verify-live-agents.sh qwen
#   bash scripts/verify-live-agents.sh --launch # all new seats and write live evidence
#   bash scripts/verify-live-agents.sh --self-test # T-301 stub-server cases only
#
# Exit non-zero if any guard fails. Safe to wire into a scheduled job.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ "${TRIUMVIRATE_SELF_TEST_CHILD:-0}" = 1 ] && [ -n "${TRIUMVIRATE_TEST_REPO_ROOT:-}" ]; then
    REPO_ROOT="$TRIUMVIRATE_TEST_REPO_ROOT"
fi
WHICH="all"
LAUNCH=0
for arg in "$@"; do
    case "$arg" in
        agy|grok|codex|review|strict|guards|shutdown|kimi|mistral|qwen|all) WHICH="$arg" ;;
        --launch) LAUNCH=1 ;;
        --self-test) WHICH="self-test" ;;
        *) echo "invalid argument: $arg" >&2; exit 2 ;;
    esac
done
if [ "$LAUNCH" -eq 1 ]; then
    WHICH="launch"
fi
# shellcheck disable=SC2164
cd "$REPO_ROOT/daemon"

FAILED=0

NEW_SEATS=(kimi mistral qwen)
LIVE_SUMMARY=""
LIVE_COUNT=""
EVIDENCE_ROWS='{}'

daemon_url() {
    printf '%s' "${TRIUMVIRATE_DAEMON_URL:-http://127.0.0.1:8080}"
}

daemon_token() {
    local token_file="${TRIUMVIRATE_HOME:-${HOME}/.triumvirate}/daemon.token"
    [ -r "$token_file" ] || return 1
    IFS= read -r REPLY < "$token_file"
    printf '%s' "$REPLY"
}

status_json() {
    local token
    token="$(daemon_token)" || return 1
    curl -fsS --max-time 5 -H "Authorization: Bearer $token" "$(daemon_url)/status"
}

json_field() {
    python3 -I -c 'import json, sys
value = json.load(sys.stdin)
for part in sys.argv[1].split("."):
    value = value.get(part) if isinstance(value, dict) else None
print(str(value).lower() if isinstance(value, bool) else "" if value is None else value)' "$1"
}

refusal_code() {
    local seat="$1" token body response
    token="$(daemon_token)" || return 1
    body="$(python3 -I -c 'import json, sys; print(json.dumps({"agent": sys.argv[1], "message": "Reply OK", "cwd": sys.argv[2]}))' "$seat" "$REPO_ROOT")"
    response="$(curl -sS --max-time 5 -H "Authorization: Bearer $token" -H 'Content-Type: application/json' -d "$body" "$(daemon_url)/ask-agent" 2>/dev/null)" || return 1
    printf '%s' "$response" | python3 -I -c 'import json, sys
value = json.load(sys.stdin)
print(value.get("error", value.get("code", "")))'
}

triumvirate_cli() {
    if [ -n "${TRIUMVIRATE_CLI:-}" ]; then
        printf '%s' "$TRIUMVIRATE_CLI"
    elif command -v triumvirate >/dev/null 2>&1; then
        command -v triumvirate
    elif [ -x "$REPO_ROOT/daemon/target/debug/triumvirate" ]; then
        printf '%s' "$REPO_ROOT/daemon/target/debug/triumvirate"
    elif [ -x "$REPO_ROOT/daemon/target/release/triumvirate" ]; then
        printf '%s' "$REPO_ROOT/daemon/target/release/triumvirate"
    else
        return 1
    fi
}

static_checks_pass() {
    local seat="$1" cli output checks
    cli="$(triumvirate_cli)" || { echo "FAIL  $seat static checks: triumvirate CLI unavailable"; return 1; }
    output="$("$cli" doctor 2>&1)" || true
    checks="$(printf '%s\n' "$output" | grep -E "^[[:space:]]*$seat\.[A-Za-z0-9_]+: (PASS|FAIL)" || true)"
    if [ -z "$checks" ] || printf '%s\n' "$checks" | grep -q ': FAIL'; then
        printf '%s\n' "$checks"
        echo "FAIL  $seat static checks"
        return 1
    fi
}

verified_field() {
    local seat="$1" field="$2" file
    file="${TRIUMVIRATE_HOME:-${HOME}/.triumvirate}/seats/$seat/verified.json"
    [ -r "$file" ] || return 1
    python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1], encoding="utf-8")).get(sys.argv[2], ""))' "$file" "$field"
}

run_new_seat_live_test() {
    local seat="$1" upper output status
    upper="$(printf '%s' "$seat" | tr '[:lower:]' '[:upper:]')"
    echo "RUN   $seat live E2E"
    output="$(env "TRIUMVIRATE_LIVE_${upper}=1" cargo test -p triumvirate --test integration_seven_primes_live -- --ignored "$seat" 2>&1)"
    status=$?
    printf '%s\n' "$output" | tail -20
    LIVE_SUMMARY="$(printf '%s\n' "$output" | grep -E 'test result: ok\. [0-9]+ passed' | tail -1 || true)"
    LIVE_COUNT="$(printf '%s' "$LIVE_SUMMARY" | sed -E 's/.*ok\. ([0-9]+) passed.*/\1/')"
    if [ "$status" -ne 0 ] || ! [[ "$LIVE_COUNT" =~ ^[1-9][0-9]*$ ]] || printf '%s\n' "$output" | grep -q 'running 0 tests'; then
        echo "FAIL  $seat live E2E"
        return 1
    fi
    echo "PASS  $seat live E2E"
}

launch_tree_is_clean() {
    local dirty disallowed
    dirty="$(git -C "$REPO_ROOT" status --porcelain --ignored -- daemon scripts .github)"
    disallowed="$(printf '%s\n' "$dirty" | grep -Ev '^[!? MARCUD][!? MARCUD] daemon/target(/|$)' || true)"
    if [ -n "$disallowed" ]; then
        echo "FAIL  launch requires a committed code tree"
        printf '%s\n' "$disallowed"
        return 1
    fi
}

add_evidence_row() {
    EVIDENCE_ROWS="$(python3 -I -c 'import json, sys
rows = json.loads(sys.argv[1]); rows[sys.argv[2]] = json.loads(sys.argv[3]); print(json.dumps(rows))' "$EVIDENCE_ROWS" "$1" "$2")"
}

run_new_seat_guards() {
    local status seat configured available suspended reason state expected actual available_count=0
    local cli_version served_model timestamp code_tree evidence_file
    EVIDENCE_ROWS='{}'
    evidence_file="$REPO_ROOT/docs/seven-primes/live-evidence.json"
    if [ "$LAUNCH" -eq 1 ]; then
        rm -f "$evidence_file"
        launch_tree_is_clean || return 1
    fi
    status="$(status_json)" || { echo "FAIL  daemon unreachable"; return 1; }
    for seat in "${NEW_SEATS[@]}"; do
        for field in configured verified suspended available reason; do
            [ -n "$(printf '%s' "$status" | json_field "seats.$seat.$field")" ] || { echo "FAIL  /status schema missing seats.$seat.$field"; return 1; }
        done
    done
    for seat in "${NEW_SEATS[@]}"; do
        if [ "$LAUNCH" -eq 0 ] && [ "$WHICH" != "all" ] && [ "$WHICH" != "$seat" ]; then continue; fi
        configured="$(printf '%s' "$status" | json_field "seats.$seat.configured")"
        available="$(printf '%s' "$status" | json_field "seats.$seat.available")"
        reason="$(printf '%s' "$status" | json_field "seats.$seat.reason")"
        if [ "$configured" != true ]; then
            echo "NOT CONFIGURED $seat"
            actual="$(refusal_code "$seat" || true)"
            if [[ "$actual" != seat_not_configured* ]]; then echo "FAIL  $seat rejection: ${actual:-<none>}"; FAILED=1; fi
            add_evidence_row "$seat" "$(python3 -I -c 'import json, sys; print(json.dumps({"status":"NOT LAUNCHED","reason":sys.argv[1]}))' "$reason")"
            continue
        fi
        if ! static_checks_pass "$seat"; then FAILED=1; continue; fi
        if [ "$available" != true ]; then
            suspended="$(printf '%s' "$status" | json_field "seats.$seat.suspended")"
            if [ "$suspended" = true ]; then state="SUSPENDED"; expected="seat_suspended"; else state="UNVERIFIED"; expected="seat_unverified"; fi
            echo "UNAVAILABLE $seat: $state $reason"
            actual="$(refusal_code "$seat" || true)"
            if [[ "$actual" != "$expected"* ]]; then echo "FAIL  $seat rejection: expected $expected, got ${actual:-<none>}"; FAILED=1; fi
            if [ "$LAUNCH" -eq 1 ]; then echo "FAIL  configured $seat is not AVAILABLE under --launch"; FAILED=1; fi
            add_evidence_row "$seat" "$(python3 -I -c 'import json, sys; print(json.dumps({"status":"NOT LAUNCHED","reason":sys.argv[1]}))' "$reason")"
            continue
        fi
        available_count=$((available_count + 1))
        if run_new_seat_live_test "$seat"; then
            if [ "$LAUNCH" -eq 1 ]; then
                cli_version="$(verified_field "$seat" cli_version || true)"
                served_model="$(verified_field "$seat" served_model || true)"
                timestamp="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
                if [ -z "$cli_version" ] || [ -z "$served_model" ]; then echo "FAIL  $seat verified.json metadata unavailable"; FAILED=1; continue; fi
                add_evidence_row "$seat" "$(python3 -I -c 'import json, sys; print(json.dumps({"status":"PASS","tests_run":int(sys.argv[1]),"cargo_summary":sys.argv[2],"cli_version":sys.argv[3],"served_model":sys.argv[4],"timestamp":sys.argv[5]}))' "$LIVE_COUNT" "$LIVE_SUMMARY" "$cli_version" "$served_model" "$timestamp")"
            fi
        else
            FAILED=1
        fi
    done
    if [ "$LAUNCH" -eq 1 ] && [ "$available_count" -eq 0 ]; then echo "FAIL  --launch requires at least one AVAILABLE new seat"; FAILED=1; fi
    if [ "$LAUNCH" -eq 1 ] && [ "$FAILED" -eq 0 ]; then
        code_tree="$(git -C "$REPO_ROOT" ls-tree -r HEAD -- daemon scripts .github | git hash-object --stdin)"
        mkdir -p "$(dirname "$evidence_file")"
        python3 -I -c 'import json, sys
with open(sys.argv[1], "w", encoding="utf-8") as out:
    json.dump({"code_tree":sys.argv[2], "seats":json.loads(sys.argv[3])}, out, indent=2, sort_keys=True); out.write("\n")' "$evidence_file" "$code_tree" "$EVIDENCE_ROWS"
        echo "PASS  launch evidence written"
    fi
    [ "$FAILED" -eq 0 ]
}

run_self_test() (
    local fixture temp_dir port server_pid base_url state_file child_script checker
    local self_failed=0 self_output=""
    if [[ "${TRIUMVIRATE_DAEMON_URL:-}" =~ :8080(/|$) ]]; then
        echo "self-test must not target the live daemon"
        exit 2
    fi
    fixture="$REPO_ROOT/scripts/fixtures/verify-live-agents"
    temp_dir="$(mktemp -d)" || exit 1
    server_pid=""
    trap 'if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi; rm -rf "$temp_dir"' EXIT
    mkdir -p "$temp_dir/home/seats/kimi" "$temp_dir/bin" "$temp_dir/repo/daemon" "$temp_dir/repo/scripts" "$temp_dir/repo/.github" "$temp_dir/repo/docs/seven-primes"
    printf 'fixture-token\n' > "$temp_dir/home/daemon.token"
    cp "$fixture/cargo" "$temp_dir/bin/cargo"
    cp "$fixture/triumvirate" "$temp_dir/bin/triumvirate"
    chmod +x "$temp_dir/bin/cargo" "$temp_dir/bin/triumvirate"
    cp "$REPO_ROOT/scripts/verify-live-agents.sh" "$temp_dir/repo/scripts/verify-live-agents.sh"
    cp "$REPO_ROOT/scripts/check-live-evidence.sh" "$temp_dir/repo/scripts/check-live-evidence.sh"
    git -C "$temp_dir/repo" init -q
    git -C "$temp_dir/repo" config user.email fixture@example.invalid
    git -C "$temp_dir/repo" config user.name fixture
    git -C "$temp_dir/repo" add .
    git -C "$temp_dir/repo" commit -qm fixture
    state_file="$temp_dir/status.json"
    cp "$fixture/status-not-configured.json" "$state_file"
    if port="$(python3 -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()' 2>/dev/null)"; then
        python3 -I "$fixture/server.py" "$port" "$state_file" "$state_file" >/dev/null 2>&1 &
        server_pid=$!
        base_url="http://127.0.0.1:$port"
    else
        cp "$fixture/curl" "$temp_dir/bin/curl"
        chmod +x "$temp_dir/bin/curl"
        base_url="http://127.0.0.1:18081"
    fi
    child_script="$REPO_ROOT/scripts/verify-live-agents.sh"
    checker="$temp_dir/repo/scripts/check-live-evidence.sh"
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        curl -fsS --max-time 1 "$base_url/status" >/dev/null 2>&1 && break
        sleep 0.3
    done

    self_case() {
        local name="$1" expected="$2" status
        shift 2
        self_output="$(HOME="$temp_dir/home" TRIUMVIRATE_HOME="$temp_dir/home" TRIUMVIRATE_DAEMON_URL="$base_url" TRIUMVIRATE_CLI="$temp_dir/bin/triumvirate" TRIUMVIRATE_SELF_TEST_CHILD=1 TRIUMVIRATE_TEST_REPO_ROOT="$temp_dir/repo" STUB_STATUS_FILE="$state_file" PATH="$temp_dir/bin:/usr/bin:/bin" "$@" 2>&1)"
        status=$?
        if [ "$status" -eq "$expected" ]; then
            echo "PASS  self-test: $name"
        else
            echo "FAIL  self-test: $name (exit $status expected $expected)"
            printf '%s\n' "$self_output"
            self_failed=1
        fi
    }

    self_case "unreachable daemon" 1 env TRIUMVIRATE_DAEMON_URL=http://127.0.0.1:1 bash "$child_script" kimi
    self_case "all not configured launch" 1 bash "$child_script" all --launch
    cp "$fixture/status-kimi-configured.json" "$state_file"
    self_case "configured kimi missing binary" 1 env DOCTOR_FAIL_SEAT=kimi bash "$child_script" kimi
    cp "$fixture/status-mistral-unverified.json" "$state_file"
    self_case "static-clean unverified mistral" 0 bash "$child_script" mistral
    if [[ "$self_output" == *"UNAVAILABLE mistral: UNVERIFIED seat_unverified"* ]]; then echo "PASS  self-test: unavailable line"; else echo "FAIL  self-test: unavailable line"; self_failed=1; fi
    self_case "configured unavailable launch" 1 bash "$child_script" mistral --launch
    cp "$fixture/status-mistral-unverified-mismatch.json" "$state_file"
    self_case "unavailable refusal mismatch" 1 bash "$child_script" mistral
    cp "$fixture/status-kimi-available.json" "$state_file"
    : > "$temp_dir/cargo.log"
    self_case "available seat live test" 0 env CARGO_LOG="$temp_dir/cargo.log" bash "$child_script" kimi
    if grep -q 'TRIUMVIRATE_LIVE_KIMI=1.*argv=test -p triumvirate --test integration_seven_primes_live -- --ignored kimi' "$temp_dir/cargo.log"; then echo "PASS  self-test: exact live invocation"; else echo "FAIL  self-test: exact live invocation"; self_failed=1; fi
    self_case "cargo running zero tests" 1 env CARGO_MODE=zero bash "$child_script" kimi
    self_case "cargo exits one" 1 env CARGO_MODE=fail bash "$child_script" kimi
    printf '{"cli_version":"2.2.0","served_model":"kimi-test"}\n' > "$temp_dir/home/seats/kimi/verified.json"
    self_case "launch writes evidence" 0 bash "$child_script" all --launch
    if bash "$checker" "$temp_dir/repo/docs/seven-primes/live-evidence.json" >/dev/null; then echo "PASS  self-test: matching evidence"; else echo "FAIL  self-test: matching evidence"; self_failed=1; fi
    cp "$temp_dir/repo/docs/seven-primes/live-evidence.json" "$temp_dir/evidence.json"
    printf '# changed\n' >> "$temp_dir/repo/scripts/verify-live-agents.sh"
    git -C "$temp_dir/repo" add scripts/verify-live-agents.sh
    git -C "$temp_dir/repo" commit -qm changed
    if ! bash "$checker" "$temp_dir/evidence.json" >/dev/null; then echo "PASS  self-test: stale code tree"; else echo "FAIL  self-test: stale code tree"; self_failed=1; fi
    git -C "$temp_dir/repo" reset --hard -q HEAD~1
    python3 -I -c 'import json, sys; value=json.load(open(sys.argv[1])); del value["seats"]["qwen"]; json.dump(value,open(sys.argv[1],"w"))' "$temp_dir/evidence.json"
    if ! bash "$checker" "$temp_dir/evidence.json" >/dev/null; then echo "PASS  self-test: missing seat"; else echo "FAIL  self-test: missing seat"; self_failed=1; fi
    if ! bash "$checker" "$temp_dir/missing.json" >/dev/null; then echo "PASS  self-test: missing evidence file"; else echo "FAIL  self-test: missing evidence file"; self_failed=1; fi
    python3 -I -c 'import json, subprocess, sys
root=sys.argv[1]; tree=subprocess.check_output(["git","-C",root,"ls-tree","-r","HEAD","--","daemon","scripts",".github"]); code=subprocess.check_output(["git","hash-object","--stdin"],input=tree).decode().strip(); json.dump({"code_tree":code,"seats":{seat:{"status":"NOT LAUNCHED","reason":"fixture"} for seat in ("kimi","mistral","qwen")}},open(sys.argv[2],"w"))' "$temp_dir/repo" "$temp_dir/evidence.json"
    if ! bash "$checker" "$temp_dir/evidence.json" >/dev/null; then echo "PASS  self-test: zero PASS"; else echo "FAIL  self-test: zero PASS"; self_failed=1; fi
    printf 'dirty\n' > "$temp_dir/repo/daemon/dirty"
    self_case "dirty daemon blocks launch" 1 bash "$child_script" kimi --launch
    if [ ! -e "$temp_dir/repo/docs/seven-primes/live-evidence.json" ]; then echo "PASS  self-test: dirty launch writes nothing"; else echo "FAIL  self-test: dirty launch writes nothing"; self_failed=1; fi
    rm "$temp_dir/repo/daemon/dirty"
    printf 'daemon/ignored\n' > "$temp_dir/repo/.gitignore"
    git -C "$temp_dir/repo" add .gitignore
    git -C "$temp_dir/repo" commit -qm ignore
    printf 'ignored\n' > "$temp_dir/repo/daemon/ignored"
    self_case "ignored daemon file blocks launch" 1 bash "$child_script" kimi --launch
    exit "$self_failed"
)

if [ "$WHICH" = "self-test" ]; then
    run_self_test
    exit $?
fi

# shellcheck disable=SC2329
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

if [ "$WHICH" = "all" ] || [ "$WHICH" = "shutdown" ]; then
    # The daemon EXITS after SIGTERM even with a connection stuck mid-request. Runs a throwaway
    # daemon on its own port and home, so a live daemon is never touched. Never escalates to
    # SIGKILL on its own: start-daemon.sh does, and that escalation is exactly what hid this hang.
    echo "RUN   daemon exits on SIGTERM with a stuck connection"
    if python3 "$REPO_ROOT/scripts/verify-shutdown.py"; then
        echo "PASS  daemon exits on SIGTERM with a stuck connection"
    else
        echo "FAIL  daemon exits on SIGTERM with a stuck connection"
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

# Run a guard and require that it actually RAN something.
#
# `cargo test <filter>` exits 0 when the filter matches NOTHING: "0 passed; 0 failed". Rename or
# delete a guard and this script printed PASS for a test that no longer exists (Grok, panel
# review of the D-027 fix). Absence reported as success is the exact shape these guards exist to
# catch, so the guard runner must not have it.
run_guard() {
    label="$1"
    shift
    echo "RUN   $label"
    out="$("$@" 2>&1)"
    status=$?
    echo "$out" | tail -12
    if [ "$status" -ne 0 ]; then
        echo "FAIL  $label"
        FAILED=1
        return
    fi
    if ! echo "$out" | grep -qE "test result: ok\. [1-9][0-9]* passed"; then
        echo "FAIL  $label (no test actually ran: filter matched nothing)"
        FAILED=1
        return
    fi
    echo "PASS  $label"
}

if [ "$WHICH" = "all" ] || [ "$WHICH" = "strict" ]; then
    # strict_agent, end to end, with MOCK agy and codex binaries. No network, no quota.
    #
    # Single-threaded and opt-in for the same reason as the review guards: the mocks replace
    # the agy and codex binaries for every dispatch in that test binary.
    #
    # strict_01 is the negative control. It proves the fixture really does substitute codex
    # when strict is off, so strict_02 cannot be green merely because nothing degraded.
    run_guard "strict_agent never substitutes" \
        cargo test -p triumvirate --bin triumvirate strict_agent_tests -- --ignored --test-threads=1

    # The fleet surface of the same rule: a breaker-open gemini task must launch NOBODY, and
    # must still drive its fleet to a terminal state. Opens the process-global breaker, so it
    # cannot run beside the parallel fleet tests.
    run_guard "fleet substitutes nobody and never strands a fleet" \
        cargo test -p fleet breaker_open_gemini -- --ignored --test-threads=1

    # breaker_probe against the REAL process-global breaker: a healthy probe closes an open
    # breaker, and repeated failed probes never extend its cooldown.
    run_guard "breaker_probe closes on health, never extends on failure" \
        cargo test -p triumvirate --bin triumvirate breaker_probe_tests -- --ignored --test-threads=1
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

if [ "$WHICH" = "all" ] || [ "$WHICH" = "launch" ] || [ "$WHICH" = "kimi" ] || [ "$WHICH" = "mistral" ] || [ "$WHICH" = "qwen" ]; then
    if ! run_new_seat_guards; then
        FAILED=1
    fi
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
