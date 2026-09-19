#!/usr/bin/env bash
# Prove this repo's git hooks are ARMED, not merely present (D-009).
#
# From 2026-05-10 to 2026-07-29 the hooks here were dead in two independent ways, and each looked
# fine on inspection:
#   1. The .git/hooks/ symlinks pointed at /Users/mikeboscia/..., a user that does not exist on
#      this machine. `ls -la` showed the hooks present. They could not run.
#   2. core.hooksPath in .git/config ALSO pointed there. When that key is set git uses it
#      EXCLUSIVELY and never looks in .git/hooks/, so repointing the symlinks changed nothing.
# The first "fix" was verified by running the hook script by hand, which proves the script works
# and says nothing about whether GIT calls it. That is the trap this script exists to close.
#
# So every check here asks GIT, never the filesystem directly:
#   - `git rev-parse --git-path hooks/<name>` is the path git will actually execute. It honours
#     core.hooksPath (verified on git 2.39.5), which is exactly what failure 2 hid behind.
#   - `test -x` on that path follows symlinks, so a dangling link fails, which is failure 1.
#   - pre-commit is also RUN through `git hook run`, git's own dispatcher, not by path.
#     pre-push is resolved but not run: it is `cargo check` + `clippy`, too slow for a routine
#     check, and resolution is what both historical failures broke.
#
# Silence is not success here. Every hook prints a line either way, and --self-test proves the
# checks can FAIL by reproducing both historical failures in a scratch repo.
#
# Usage:
#   bash scripts/verify-guards.sh              # check this repo's hooks
#   bash scripts/verify-guards.sh --self-test  # prove the checks catch both D-009 failures

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOKS=(pre-commit pre-push)

# check_hooks <repo> [run_pre_commit]: prints one line per hook, returns non-zero if any is inert.
check_hooks() {
    local repo="$1" run_pre_commit="${2:-1}" failed=0 hook resolved
    for hook in "${HOOKS[@]}"; do
        resolved="$(git -C "$repo" rev-parse --git-path "hooks/$hook")"
        # --git-path can return a path relative to the repo; resolve it against the repo.
        [[ "$resolved" = /* ]] || resolved="$repo/$resolved"
        if [[ -x "$resolved" && -f "$resolved" ]]; then
            echo "ARMED   $hook -> $resolved"
        else
            echo "INERT   $hook: git would run '$resolved', which is missing, dangling, or not executable"
            failed=1
        fi
    done
    local hooks_path
    hooks_path="$(git -C "$repo" config --get core.hooksPath || true)"
    if [[ -n "$hooks_path" ]]; then
        echo "NOTE    core.hooksPath is set to '$hooks_path'; git ignores .git/hooks/ entirely while it is"
    fi
    if [[ "$run_pre_commit" = 1 && "$failed" = 0 ]]; then
        if (cd "$repo" && git hook run pre-commit >/dev/null 2>&1); then
            echo "RAN     pre-commit through git's own dispatcher: ok"
        else
            echo "INERT   pre-commit resolved but 'git hook run pre-commit' did not succeed"
            failed=1
        fi
    fi
    return "$failed"
}

self_test() {
    local scratch rc=0
    scratch="$(mktemp -d)"
    trap 'rm -rf "$scratch"' RETURN

    # Failure 1: the hook is a symlink to a path that does not exist.
    git init -q "$scratch/dangling"
    for h in "${HOOKS[@]}"; do ln -s "/Users/mikeboscia/nope/$h" "$scratch/dangling/.git/hooks/$h"; done
    if check_hooks "$scratch/dangling" 0 >/dev/null; then
        echo "SELF-TEST FAIL: dangling hook symlinks were reported ARMED (D-009 failure 1)"; rc=1
    else
        echo "SELF-TEST ok:   dangling symlinks are caught (D-009 failure 1)"
    fi

    # Failure 2: real, executable hooks in .git/hooks/, but core.hooksPath points elsewhere, so
    # git never runs them. This is the one a manual run of the script can never catch.
    git init -q "$scratch/shadowed"
    for h in "${HOOKS[@]}"; do
        printf '#!/bin/sh\nexit 0\n' > "$scratch/shadowed/.git/hooks/$h"
        chmod +x "$scratch/shadowed/.git/hooks/$h"
    done
    git -C "$scratch/shadowed" config core.hooksPath /Users/mikeboscia/projects/triumvirate/.git/hooks
    if check_hooks "$scratch/shadowed" 0 >/dev/null; then
        echo "SELF-TEST FAIL: hooks shadowed by core.hooksPath were reported ARMED (D-009 failure 2)"; rc=1
    else
        echo "SELF-TEST ok:   hooks shadowed by core.hooksPath are caught (D-009 failure 2)"
    fi

    # Control: a correctly armed repo passes, so the two results above are not "always fails".
    git init -q "$scratch/armed"
    for h in "${HOOKS[@]}"; do
        printf '#!/bin/sh\nexit 0\n' > "$scratch/armed/.git/hooks/$h"
        chmod +x "$scratch/armed/.git/hooks/$h"
    done
    if check_hooks "$scratch/armed" 1 >/dev/null; then
        echo "SELF-TEST ok:   a correctly armed repo passes (control)"
    else
        echo "SELF-TEST FAIL: a correctly armed repo was reported INERT, so the checks prove nothing"; rc=1
    fi
    return "$rc"
}

if [[ "${1:-}" = "--self-test" ]]; then
    self_test
    exit $?
fi

echo "verify-guards: $REPO_ROOT"
if check_hooks "$REPO_ROOT" 1; then
    echo "verify-guards: every hook is armed"
else
    echo ""
    echo "verify-guards: A GUARD IS INERT. It is installed and git will not run it."
    echo "Re-arm with: bash scripts/install-git-hooks.sh   and unset any stray core.hooksPath."
    exit 1
fi
