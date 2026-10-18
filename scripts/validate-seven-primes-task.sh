#!/usr/bin/env bash
# Seven Primes task validator (T-000, REQ-010). Blocks a task unless every gate really ran and really passed.
#
# Why this exists: ~/.claude/scripts/validate-task.sh runs `TEST_OUTPUT=$(eval "$TEST_CMD" 2>&1) || true` and then reads
# `TEST_EXIT=$?`, which is the exit status of `|| true`, so its test gate is 0 on every run. It also runs no format, lint,
# type or security check. This build uses this script instead (IMPLEMENTATION_PLAN.md T-000 and Definition of Done).
#
# Rules this script enforces, each with the failure it closes:
#   - Every command's exit status is captured on its own line, never through a pipe or `|| true`.
#   - EMPTY-SET RULE: every check prints how much it examined. A check that examined nothing is a FAIL unless the task is
#     on the fixed NON-RUST list, and then it prints the exact N/A line instead of passing silently.
#   - RUST SCOPE is daemon/**/*.rs only. scripts/fixtures/validate-seven-primes/ is deliberately bad and must never be
#     formatted, linted or scanned by the real floor.
#   - TASK RANGE: file scope and Rust scope are computed over every commit whose message names the task id (T-302 has two
#     commits), not over the last commit.
#   - SECURITY FLOOR runs after the commit, over origin/main..HEAD. `gitleaks protect --staged` is never used: after a
#     commit the index is empty, and it scans 0 bytes and exits 0 (reproduced 2026-10-18 with a planted key).
#   - semgrep gates at WARNING, not ERROR: p/rust has no ERROR rules, so an ERROR gate can never fire (amendment A-1,
#     docs/seven-primes/build/AMENDMENTS-2026-10-18.md on plan/seven-primes).
#   - Any change to a Cargo.lock or to a dependency table of any Cargo.toml fails: the build adds no crates.
#
# Usage:
#   bash scripts/validate-seven-primes-task.sh T-ID <files...>   # validate one task against its <files> list
#   bash scripts/validate-seven-primes-task.sh --floor-only       # security floor + Rust scope over the branch (CI job)
#   bash scripts/validate-seven-primes-task.sh --self-test        # prove every gate can FAIL, against scratch repos
#
# Exit: 0 all gates passed; 1 any gate failed; 2 no failure but a warning (a file outside the task's list).
#
# Overrides (the self-test uses these to point the real code path at a fixture workspace; defaults are the plan's exact
# commands): SEVEN_PRIMES_BASE, SEVEN_PRIMES_CHECK_CMD, SEVEN_PRIMES_LINT_CMD, SEVEN_PRIMES_TESTS_BASE_CMD,
# SEVEN_PRIMES_TESTS_FULL_CMD.

set -uo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
FIXTURE_DIR="$(dirname "$SCRIPT_PATH")/fixtures/validate-seven-primes"

BASE="${SEVEN_PRIMES_BASE:-origin/main}"
CHECK_CMD="${SEVEN_PRIMES_CHECK_CMD:-cargo check --workspace --exclude pantheon}"
LINT_CMD="${SEVEN_PRIMES_LINT_CMD:-cargo clippy --workspace --exclude pantheon --all-targets -- -D warnings}"
TESTS_BASE_CMD="${SEVEN_PRIMES_TESTS_BASE_CMD:-cargo test --workspace --exclude pantheon --lib && cargo test -p triumvirate --tests && cargo test -p fleet --tests && cargo test -p fleet-temporal --test replay}"
TESTS_FULL_CMD="${SEVEN_PRIMES_TESTS_FULL_CMD:-$TESTS_BASE_CMD && cargo test -p mcp-bridge --test containment}"

NON_RUST_TASKS=" T-000 T-301 T-303 "
NA_LINE="N/A (0 daemon .rs files; non-Rust task)"
STUB_PATTERN='todo!|unimplemented!|not implemented: T-'

FAILS=0
WARNS=0
SUMMARY=()

# record <PASS|FAIL|WARN|N/A> <check> <detail>: one summary line per check; FAIL and WARN set the exit code.
record() {
    SUMMARY+=("$(printf '%-5s %-14s %s' "$1" "$2" "$3")")
    case "$1" in
        FAIL) FAILS=$((FAILS + 1)) ;;
        WARN) WARNS=$((WARNS + 1)) ;;
    esac
}

# run_in_daemon <check> <command>: runs a cargo command in daemon/, prints its full output, records its own exit status.
run_in_daemon() {
    local name="$1" cmd="$2" out rc
    echo "=== $name: $cmd"
    out="$(cd "$REPO/daemon" && bash -c "$cmd" 2>&1)"
    rc=$?
    printf '%s\n' "$out"
    if [[ $rc -eq 0 ]]; then record PASS "$name" "exit 0"; else record FAIL "$name" "exit $rc"; fi
}

# need <tool>: a missing floor tool is a FAIL, never a skip.
need() {
    if command -v "$1" >/dev/null 2>&1; then return 0; fi
    echo "$1 not found on PATH: the security floor cannot run without it"
    record FAIL "$1" "$1 not found on PATH (missing tool is a FAIL)"
    return 1
}

is_non_rust() { [[ "$NON_RUST_TASKS" == *" $1 "* ]]; }

# Test suite by task id, never by whether the containment target exists (a deleted target must fail loudly).
suite_for() {
    case "$1" in
        T-104 | T-2?? | T-3??) echo FULL_TESTS ;;
        *) echo FULL_TESTS_BASE ;;
    esac
}

# Commits on the branch whose message names the task id as a whole token (T-30 must not match T-302).
task_commits() {
    git -C "$REPO" log --format=%H -E --grep="(^|[^A-Za-z0-9-])$1([^0-9]|$)" "$BASE..HEAD"
}

# files_in <commit...>: every path touched by those commits, one per line, unique.
files_in() {
    local c
    for c in "$@"; do git -C "$REPO" diff-tree --no-commit-id --name-only -r --root "$c"; done | sort -u
}

# daemon_rs <paths on stdin>: keep only Rust-scope paths that still exist (a deleted file cannot be formatted).
daemon_rs() {
    local f
    while IFS= read -r f; do
        [[ "$f" == daemon/*.rs && -f "$REPO/$f" ]] && printf '%s\n' "$f"
    done
}

# ---- security floor (shared by task mode and --floor-only) -------------------------------------------------------

floor_gitleaks() {
    local n out rc
    n="$(git -C "$REPO" rev-list --count "$BASE..HEAD")"
    if [[ "$n" -eq 0 ]]; then
        echo "gitleaks: 0 commits in $BASE..HEAD: nothing to scan"
        record FAIL gitleaks "0 commits in $BASE..HEAD: nothing to scan"
        return
    fi
    need gitleaks || return
    echo "=== gitleaks: $n commits in $BASE..HEAD"
    # --verbose prints each finding with its RuleID; --redact keeps the secret itself out of the log.
    out="$(cd "$REPO" && gitleaks git --log-opts="$BASE..HEAD" --redact --no-banner --verbose . 2>&1)"
    rc=$?
    printf '%s\n' "$out"
    if [[ $rc -eq 0 ]]; then record PASS gitleaks "$n commits scanned, no leaks"; else record FAIL gitleaks "$n commits scanned, exit $rc"; fi
}

# floor_semgrep <non_rust_task 0|1> <daemon .rs paths...>
floor_semgrep() {
    local nonrust="$1" out rc
    shift
    if [[ $# -eq 0 ]]; then
        if [[ "$nonrust" -eq 1 ]]; then echo "semgrep: $NA_LINE"; record N/A semgrep "$NA_LINE"; return; fi
        record FAIL semgrep "0 daemon .rs files to scan"
        return
    fi
    need semgrep || return
    echo "=== semgrep: $# daemon .rs files"
    out="$(cd "$REPO" && semgrep scan --config p/rust --severity WARNING --error --metrics=off "$@" 2>&1)"
    rc=$?
    printf '%s\n' "$out"
    if [[ $rc -eq 0 ]]; then record PASS semgrep "$# files scanned, 0 findings"; else record FAIL semgrep "$# files scanned, exit $rc"; fi
}

# floor_rustfmt <non_rust_task 0|1> <daemon .rs paths...>
floor_rustfmt() {
    local nonrust="$1" out rc
    shift
    if [[ $# -eq 0 ]]; then
        if [[ "$nonrust" -eq 1 ]]; then echo "rustfmt: $NA_LINE"; record N/A rustfmt "$NA_LINE"; return; fi
        record FAIL rustfmt "0 daemon .rs files to format"
        return
    fi
    echo "=== rustfmt: $# daemon .rs files"
    out="$(cd "$REPO" && rustfmt --edition 2024 --check "$@" 2>&1)"
    rc=$?
    printf '%s\n' "$out"
    if [[ $rc -eq 0 ]]; then record PASS rustfmt "$# files formatted"; else record FAIL rustfmt "$# files, exit $rc"; fi
}

# No new crates: any Cargo.lock change, or any change to a dependency table of any Cargo.toml, in BASE..HEAD fails.
floor_dependencies() {
    local changed locks tomls f old new verdict rc
    changed="$(git -C "$REPO" diff --name-only "$BASE..HEAD")"
    locks="$(printf '%s\n' "$changed" | grep -E '(^|/)Cargo\.lock$' || true)"
    tomls="$(printf '%s\n' "$changed" | grep -E '(^|/)Cargo\.toml$' || true)"
    if [[ -n "$locks" ]]; then
        echo "dependencies: Cargo.lock changed: $locks"
        record FAIL deps "Cargo.lock changed ($locks): the build adds no crates"
    fi
    local n=0 bad=0
    while IFS= read -r f; do
        [[ -z "$f" ]] && continue
        n=$((n + 1))
        old="$(git -C "$REPO" show "$BASE:$f" 2>/dev/null || true)"
        new="$(git -C "$REPO" show "HEAD:$f" 2>/dev/null || true)"
        verdict="$(OLD_TOML="$old" NEW_TOML="$new" python3 -I -c '
import os, tomllib
def tables(text):
    """Every dependency table, keyed by its dotted path, including [target.<cfg>.*] and [workspace.dependencies]."""
    doc = tomllib.loads(text) if text.strip() else {}
    names = ("dependencies", "dev-dependencies", "build-dependencies")
    out = {n: doc.get(n, {}) for n in names}
    out["workspace.dependencies"] = doc.get("workspace", {}).get("dependencies", {})
    for cfg, body in doc.get("target", {}).items():
        for n in names:
            out["target." + cfg + "." + n] = body.get(n, {})
    return out
a, b = tables(os.environ["OLD_TOML"]), tables(os.environ["NEW_TOML"])
print(" ".join(sorted(k for k in set(a) | set(b) if a.get(k, {}) != b.get(k, {}))))
')"
        rc=$?
        if [[ $rc -ne 0 ]]; then
            record FAIL deps "$f: could not parse as TOML (exit $rc)"
            bad=1
        elif [[ -n "$verdict" ]]; then
            echo "dependencies: $f changed table(s): $verdict"
            record FAIL deps "$f changed [$verdict]: the build adds no crates"
            bad=1
        fi
    done <<<"$tomls"
    if [[ -z "$locks" && $bad -eq 0 ]]; then record PASS deps "0 Cargo.lock changes, $n Cargo.toml examined, no dependency-table change"; fi
}

# ---- modes -----------------------------------------------------------------------------------------------------

summarize() {
    echo
    echo "=== SUMMARY"
    printf '%s\n' "${SUMMARY[@]}"
    if [[ $FAILS -gt 0 ]]; then echo "RESULT: FAIL ($FAILS failed, $WARNS warnings)"; exit 1; fi
    if [[ $WARNS -gt 0 ]]; then echo "RESULT: WARN ($WARNS warnings)"; exit 2; fi
    echo "RESULT: PASS"
    exit 0
}

floor_only() {
    local rs=()
    mapfile -t rs < <(git -C "$REPO" diff --name-only "$BASE..HEAD" | daemon_rs)
    echo "floor-only over $BASE..HEAD: ${#rs[@]} daemon .rs files changed"
    floor_gitleaks
    # In floor-only mode there is no task, so an empty Rust scope is reported as N/A with its own reason.
    if [[ ${#rs[@]} -eq 0 ]]; then
        record N/A rustfmt "0 daemon .rs files changed in $BASE..HEAD"
        record N/A semgrep "0 daemon .rs files changed in $BASE..HEAD"
    else
        floor_rustfmt 0 "${rs[@]}"
        floor_semgrep 0 "${rs[@]}"
    fi
    floor_dependencies
    summarize
}

validate_task() {
    local task="$1" nonrust=0 commits=() touched=() rs=() f a ok outside=() suite cmd
    shift
    [[ "$task" =~ ^T-[0-9]{3}$ ]] || { echo "usage: $0 T-ID <files...> | --floor-only | --self-test"; exit 1; }
    [[ $# -gt 0 ]] || { echo "T-ID given without its <files> list"; exit 1; }
    is_non_rust "$task" && nonrust=1
    suite="$(suite_for "$task")"
    echo "task $task, suite: $suite, base: $BASE, non-Rust task: $nonrust"

    mapfile -t commits < <(task_commits "$task")
    if [[ ${#commits[@]} -eq 0 ]]; then
        record FAIL commits "0 commits in $BASE..HEAD reference $task (validate after committing)"
    else
        record PASS commits "${#commits[@]} commits reference $task"
        mapfile -t touched < <(files_in "${commits[@]}")
    fi
    if [[ -n "$(git -C "$REPO" status --porcelain --untracked-files=no)" ]]; then
        record WARN worktree "uncommitted tracked changes present; only committed $task changes were scoped"
    fi

    mapfile -t rs < <(printf '%s\n' ${touched[@]+"${touched[@]}"} | daemon_rs)
    echo "rust scope: ${#rs[@]} daemon .rs files touched by $task"
    if [[ ${#rs[@]} -eq 0 && $nonrust -eq 0 ]]; then
        record FAIL rust-scope "0 daemon .rs files touched by $task, which is not a non-Rust task"
    fi

    run_in_daemon check "$CHECK_CMD"
    run_in_daemon lint "$LINT_CMD"
    if [[ "$suite" == FULL_TESTS ]]; then cmd="$TESTS_FULL_CMD"; else cmd="$TESTS_BASE_CMD"; fi
    run_in_daemon "$suite" "$cmd"
    floor_rustfmt "$nonrust" ${rs[@]+"${rs[@]}"}

    # Stub markers in lines this task ADDED to daemon .rs files. T-003 is the declared scaffold task and is exempt.
    if [[ "$task" == T-003 ]]; then
        record N/A stubs "T-003 is the declared scaffold task"
    else
        local hits
        hits="$(for c in ${commits[@]+"${commits[@]}"}; do git -C "$REPO" show --format= -U0 "$c" -- 'daemon/*.rs'; done \
            | grep -E '^\+[^+]' | grep -E "$STUB_PATTERN" || true)"
        if [[ -n "$hits" ]]; then
            printf 'stub markers added by %s:\n%s\n' "$task" "$hits"
            record FAIL stubs "stub markers added to daemon .rs by $task"
        else
            record PASS stubs "${#rs[@]} daemon .rs files, 0 stub markers added"
        fi
    fi

    floor_gitleaks
    floor_semgrep "$nonrust" ${rs[@]+"${rs[@]}"}
    floor_dependencies

    # File scope: every touched path must match a list entry exactly, or sit under an entry ending in '/'.
    for f in ${touched[@]+"${touched[@]}"}; do
        ok=0
        for a in "$@"; do
            if [[ "$f" == "$a" || ( "$a" == */ && "$f" == "$a"* ) ]]; then ok=1; break; fi
        done
        [[ $ok -eq 1 ]] || outside+=("$f")
    done
    if [[ ${#outside[@]} -gt 0 ]]; then
        printf 'files outside the task'"'"'s file list:\n'; printf '  %s\n' "${outside[@]}"
        record WARN scope "${#outside[@]} of ${#touched[@]} files outside the task's file list"
    else
        record PASS scope "${#touched[@]} files, all in the task's file list"
    fi
    summarize
}

# ---- self-test ---------------------------------------------------------------------------------------------------
# Runs THIS script, unmodified, as a subprocess against scratch git repos built from the fixture template. Every
# assertion checks text the real tools print (cargo, clippy, gitleaks, semgrep), so a validator that swallows exit codes
# or hardcodes results fails here.

self_test() {
    local work pass=0 fail=0
    work="$(mktemp -d "${TMPDIR:-/tmp}/validate-seven-primes.XXXXXX")"
    export CARGO_TARGET_DIR="$work/target"
    echo "self-test scratch: $work"

    # fresh_repo <name>: a git repo whose `main` holds the template, checked out on branch `feat`.
    fresh_repo() {
        local r="$work/$1"
        mkdir -p "$r" && cp -R "$FIXTURE_DIR/template/." "$r/"
        git -C "$r" init -q -b main
        git -C "$r" -c user.name=t -c user.email=t@t add -A
        git -C "$r" -c user.name=t -c user.email=t@t commit -qm base
        git -C "$r" checkout -q -b feat
        printf '%s' "$r"
    }
    commit() { git -C "$1" -c user.name=t -c user.email=t@t add -A && git -C "$1" -c user.name=t -c user.email=t@t commit -qm "$2"; }

    # run_case <label> <expect: 0|1|2|nonzero> <repo> <real-cargo 0|1> <args...>; output kept in $OUT.
    OUT=""
    run_case() {
        local label="$1" expect="$2" repo="$3" real="$4" rc
        shift 4
        local env=(SEVEN_PRIMES_BASE=main)
        if [[ "$real" -eq 1 ]]; then
            env+=("SEVEN_PRIMES_CHECK_CMD=cargo check --workspace --offline"
                "SEVEN_PRIMES_LINT_CMD=cargo clippy --workspace --all-targets --offline -- -D warnings"
                "SEVEN_PRIMES_TESTS_BASE_CMD=cargo test --workspace --offline"
                "SEVEN_PRIMES_TESTS_FULL_CMD=cargo test --workspace --offline")
        else
            # Cases about one floor check stub out cargo with `true`; the cargo gates are proven by the real-cargo cases.
            env+=(SEVEN_PRIMES_CHECK_CMD=true SEVEN_PRIMES_LINT_CMD=true SEVEN_PRIMES_TESTS_BASE_CMD=true SEVEN_PRIMES_TESTS_FULL_CMD=true)
        fi
        OUT="$(cd "$repo" && env "${env[@]}" ${CASE_PATH:+PATH="$CASE_PATH"} bash "$SCRIPT_PATH" "$@" 2>&1)"
        rc=$?
        if [[ "$expect" == nonzero && $rc -ne 0 ]] || [[ "$expect" == "$rc" ]]; then
            CASE_OK=1
        else
            CASE_OK=0
            echo "  [$label] expected exit $expect, got $rc"
        fi
    }
    # expect_text <label> <fixed string>: the case passes only if the tool's own text is in the output.
    expect_text() {
        if ! grep -qF -- "$2" <<<"$OUT"; then CASE_OK=0; echo "  [$1] output lacks: $2"; fi
    }
    expect_count() {
        local got
        got="$(grep -cF -- "$2" <<<"$OUT")"
        if [[ "$got" -ne "$3" ]]; then CASE_OK=0; echo "  [$1] expected $3 occurrences of '$2', got $got"; fi
    }
    verdict() {
        if [[ $CASE_OK -eq 1 ]]; then pass=$((pass + 1)); echo "ok   $1"; else fail=$((fail + 1)); echo "FAIL $1"; printf '%s\n' "$OUT" | tail -40 | sed 's/^/     | /'; fi
    }

    local r key
    # 1. Real cargo, failing variant: a genuinely failing test and a genuine clippy lint must both surface.
    r="$(fresh_repo cargo-fail)"
    cp "$FIXTURE_DIR/variants/lib_failing.rs" "$r/daemon/fx/src/lib.rs" && commit "$r" "T-101: failing variant"
    run_case cargo-fail 1 "$r" 1 T-101 daemon/fx/src/lib.rs
    expect_text cargo-fail "test result: FAILED"
    expect_text cargo-fail "needless_return"
    verdict "1 failing test and clippy lint -> exit 1 with cargo's own text"

    # 2. Real cargo, passing variant: the same path exits 0 and prints every check's target count.
    r="$(fresh_repo cargo-pass)"
    printf '\n// touched by the passing case\n' >>"$r/daemon/fx/src/lib.rs" && commit "$r" "T-101: passing variant"
    run_case cargo-pass 0 "$r" 1 T-101 daemon/fx/src/lib.rs
    expect_text cargo-pass "test result: ok"
    expect_text cargo-pass "suite: FULL_TESTS_BASE"
    expect_text cargo-pass "1 commits scanned, no leaks"
    expect_text cargo-pass "1 files scanned, 0 findings"
    expect_text cargo-pass "1 files formatted"
    expect_text cargo-pass "Cargo.toml examined"
    verdict "2 passing variant -> exit 0 with per-check target counts"

    # 3. A touched file outside the list is a warning (exit 2), never silent.
    r="$(fresh_repo scope)"
    printf '\n// scope case\n' >>"$r/daemon/fx/src/lib.rs" && printf 'x\n' >"$r/docs/extra.md" && commit "$r" "T-101: scope"
    run_case scope 2 "$r" 0 T-101 daemon/fx/src/lib.rs
    expect_text scope "docs/extra.md"
    verdict "3 file outside the list -> exit 2 naming the file"

    # 4. gitleaks scans committed history: a key in the last COMMIT (the index is empty) must fail with its RuleID.
    #    The key is assembled here so no literal secret is ever committed to this repo.
    r="$(fresh_repo gitleaks)"
    # gitleaks aws-access-token is (AKIA|...)[A-Z2-7]{16}: digits 0, 1, 8 and 9 are not in that alphabet.
    key="AKI""A""Z7Q3XK4P2M6N5RWT"
    printf 'aws_access_key_id = %s\n' "$key" >"$r/docs/creds.txt" && commit "$r" "T-000: planted key"
    run_case gitleaks 1 "$r" 0 T-000 docs/
    expect_text gitleaks "RuleID"
    verdict "4 committed fake AWS key -> exit 1 with gitleaks RuleID"

    # 5. Any Cargo.lock change fails.
    r="$(fresh_repo lock)"
    printf '\n[[package]]\nname = "fw"\nversion = "0.1.0"\n' >>"$r/daemon/Cargo.lock" && commit "$r" "T-000: lock entry"
    run_case lock 1 "$r" 0 T-000 daemon/Cargo.lock
    expect_text lock "Cargo.lock changed"
    verdict "5 Cargo.lock entry added -> exit 1"

    # 6. A crate already present transitively (fz, via fy) added to EACH dependency table, lockfile untouched: all fail.
    local table file line
    for table in dependencies dev-dependencies build-dependencies workspace.dependencies; do
        r="$(fresh_repo "dep-$table")"
        if [[ "$table" == workspace.dependencies ]]; then file="$r/daemon/Cargo.toml" line='fz = { path = "fz" }'
        else file="$r/daemon/fx/Cargo.toml" line='fz = { path = "../fz" }'; fi
        python3 -I -c 'import sys; p,t,l=sys.argv[1:]; s=open(p).read(); h="["+t+"]\n"; assert h in s; open(p,"w").write(s.replace(h,h+l+"\n",1))' "$file" "$table" "$line"
        commit "$r" "T-000: dep in $table"
        run_case "dep-$table" 1 "$r" 0 T-000 daemon/
        expect_text "dep-$table" "$table"
        expect_text "dep-$table" "the build adds no crates"
        verdict "6 transitive crate added to [$table], Cargo.lock unchanged -> exit 1"
    done

    # 7. Editing only [package] description passes the dependency check.
    r="$(fresh_repo package-only)"
    sed -i.bak 's/validator fixture crate/validator fixture crate, edited/' "$r/daemon/fx/Cargo.toml" && rm "$r/daemon/fx/Cargo.toml.bak"
    commit "$r" "T-000: description only"
    run_case package-only 0 "$r" 0 T-000 daemon/fx/Cargo.toml
    expect_text package-only "no dependency-table change"
    verdict "7 [package] description edit only -> exit 0"

    # 8. No semgrep on PATH is a FAIL with the missing-tool message, not a skip. The shim PATH keeps every other tool.
    r="$(fresh_repo no-semgrep)"
    printf '\n// no-semgrep case\n' >>"$r/daemon/fx/src/lib.rs" && commit "$r" "T-101: no semgrep"
    local shim="$work/shim" t
    mkdir -p "$shim"
    for t in git gitleaks cargo rustfmt python3 bash mktemp; do ln -sf "$(command -v "$t")" "$shim/$t"; done
    CASE_PATH="$shim:/usr/bin:/bin" run_case no-semgrep 1 "$r" 0 T-101 daemon/fx/src/lib.rs
    expect_text no-semgrep "semgrep not found on PATH"
    verdict "8 semgrep missing -> exit 1 with the missing-tool message"

    # 9. A planted p/rust WARNING pattern in a daemon .rs file fails with the rule id.
    r="$(fresh_repo semgrep-hit)"
    cp "$FIXTURE_DIR/variants/semgrep_bad.rs" "$r/daemon/fx/src/bad.rs" && commit "$r" "T-101: semgrep pattern"
    run_case semgrep-hit 1 "$r" 0 T-101 daemon/fx/src/bad.rs
    expect_text semgrep-hit "reqwest-accept-invalid"
    verdict "9 planted reqwest-accept-invalid -> exit 1 with the rule id"

    # 10. HEAD equal to the base: nothing to scan is a FAIL.
    r="$(fresh_repo empty-range)"
    run_case empty-range 1 "$r" 0 T-000 docs/
    expect_text empty-range "nothing to scan"
    verdict "10 empty commit range -> exit 1 'nothing to scan'"

    # 11. A Rust task whose commits touch no daemon .rs fails the Rust scope.
    r="$(fresh_repo rust-empty)"
    printf 'y\n' >"$r/docs/notes.md" && commit "$r" "T-201: docs only"
    run_case rust-empty 1 "$r" 0 T-201 docs/
    expect_text rust-empty "0 daemon .rs files touched by T-201"
    expect_text rust-empty "suite: FULL_TESTS"
    verdict "11 T-201 touching no daemon .rs -> exit 1"

    # 12. A non-Rust task with a scripts-only diff passes and prints the exact N/A line for rustfmt and semgrep.
    r="$(fresh_repo non-rust)"
    mkdir -p "$r/scripts" && printf '#!/usr/bin/env bash\necho hi\n' >"$r/scripts/x.sh" && commit "$r" "T-301: scripts only"
    run_case non-rust 0 "$r" 0 T-301 scripts/
    expect_count non-rust "rustfmt: $NA_LINE" 1
    expect_count non-rust "semgrep: $NA_LINE" 1
    verdict "12 T-301 scripts-only -> exit 0 with both N/A lines"

    # 13. A two-commit task (code, then docs-only evidence) passes: the Rust scope spans both commits.
    r="$(fresh_repo two-commit)"
    printf '\n// T-302 code\n' >>"$r/daemon/fx/src/lib.rs" && commit "$r" "T-302: code"
    printf '{}\n' >"$r/docs/live-evidence.json" && commit "$r" "T-302: evidence"
    run_case two-commit 0 "$r" 0 T-302 daemon/fx/src/lib.rs docs/live-evidence.json
    expect_text two-commit "2 commits reference T-302"
    expect_text two-commit "1 daemon .rs files touched by T-302"
    verdict "13 two-commit task with docs-only second commit -> exit 0"

    # 14. Suite selection is by task id: T-104 runs FULL_TESTS, T-105 runs FULL_TESTS_BASE.
    r="$(fresh_repo suite)"
    printf '\n// suite\n' >>"$r/daemon/fx/src/lib.rs" && commit "$r" "T-104 T-105: suite probe"
    run_case suite-104 0 "$r" 0 T-104 daemon/fx/src/lib.rs
    expect_text suite-104 "suite: FULL_TESTS,"
    verdict "14a T-104 -> FULL_TESTS"
    run_case suite-105 0 "$r" 0 T-105 daemon/fx/src/lib.rs
    expect_text suite-105 "suite: FULL_TESTS_BASE"
    verdict "14b T-105 -> FULL_TESTS_BASE"

    echo
    echo "self-test: $pass passed, $fail failed"
    rm -rf "$work"
    [[ $fail -eq 0 ]]
}

# ---- entry ---------------------------------------------------------------------------------------------------------

case "${1:-}" in
    --self-test) self_test; exit $? ;;
esac
REPO="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "not inside a git repository"; exit 1; }
case "${1:-}" in
    --floor-only) floor_only ;;
    "") echo "usage: $0 T-ID <files...> | --floor-only | --self-test"; exit 1 ;;
    *) validate_task "$@" ;;
esac
