#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HOOK="$ROOT/daemon/assets/pre-commit-hook.sh"

assert_ok() {
  if ! "$@"; then
    echo "ASSERTION FAILED: command expected success: $*"
    return 1
  fi
}

assert_fail() {
  if "$@"; then
    echo "ASSERTION FAILED: command expected failure: $*"
    return 1
  fi
}

TMP="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

REPO="$TMP/repo"
mkdir -p "$REPO/.triumvirate"
cd "$REPO"

git init >/dev/null 2>&1
git config user.email test@example.com
git config user.name test
printf "init\n" > README.md
git add README.md
git commit -m "T-000: init" >/dev/null 2>&1

cat > .triumvirate/contract.json <<'JSON'
{
  "task_id": "T-101",
  "allowed_files": ["src/allowed.rs"],
  "commit_format": "^T-000:",
  "allowed_commands": [["cargo", "test"]]
}
JSON

mkdir -p src
printf "pub fn ok() {}\n" > src/allowed.rs
git add src/allowed.rs
assert_ok "$HOOK"
git reset >/dev/null 2>&1

printf "pub fn nope() {}\n" > src/blocked.rs
git add src/blocked.rs
assert_fail "$HOOK"
git reset >/dev/null 2>&1

printf "// TODO: stub\n" > src/allowed.rs
git add src/allowed.rs
assert_fail "$HOOK"

echo "pre-commit hook tests passed"
