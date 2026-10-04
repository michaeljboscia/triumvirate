#!/usr/bin/env bash
# Build and INSTALL the triumvirate binary to the stable path production runs from.
#
# IRON LAW (2026-07-23): nothing in production ever points at daemon/target/. That is Cargo's
# DISPOSABLE build directory — `cargo clean`, a toolchain change, or any disk cleaner wipes it.
# When it was wiped on 2026-07-23, ~/.claude.json pointed the MCP server at
# daemon/target/release/triumvirate, so EVERY new MCP session in EVERY repo failed with ENOENT.
# Only already-running processes kept working (Unix keeps a deleted binary alive for an open
# fd), which disguised a total outage as "broken everywhere except here".
#
# So: build into target/ (scratch), then INSTALL to a stable path that no cleaner touches.
# This is the same place agy and codex live.
#
# Usage:  bash scripts/install.sh
# Then:   restart the daemon and MCP bridges to actually pick it up (see the note at the end).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${TRIUMVIRATE_BIN:-$HOME/.local/bin/triumvirate}"

cd "$REPO_ROOT/daemon"
echo "building release binary..."
cargo build --release -p triumvirate

mkdir -p "$(dirname "$DEST")"
# install(1) writes atomically-ish and sets the mode in one step; a plain cp over a RUNNING
# binary can fail with ETXTBSY, install replaces the directory entry instead.
install -m 755 target/release/triumvirate "$DEST"
# A STABLE code-signing identity. The linker's ad-hoc signature names the binary
# `triumvirate-<hash>`, a new identity on every build, and macOS Local Network privacy decides per
# identity: the launchd daemon's Temporal worker got "No route to host" to 192.168.2.110 after an
# install (2026-10-04) until the new identity was approved. One fixed identifier, approved once.
if [[ "$(uname)" == Darwin ]]; then
  codesign --force --sign - --identifier com.triumvirate.daemon "$DEST"
fi
# The launcher launchd runs (scripts/start-daemon.sh --foreground), at a stable path for the
# same reason as the binary: a plist pointing into a worktree breaks when the worktree goes.
install -m 755 "$REPO_ROOT/scripts/start-daemon.sh" "$(dirname "$DEST")/triumvirate-start-daemon"

echo "installed: $("$DEST" --version 2>/dev/null || echo unknown) -> $DEST"
cat <<'NOTE'

NEXT (launchd agent loaded): bash scripts/start-daemon.sh restarts the daemon through launchd.
NEXT (no launchd agent): an install alone changes nothing that is already running:
  1. Kill the daemon so it reloads the new binary AND the current ~/.claude.json env:
       kill -TERM "$(lsof -tnP -iTCP:8080 -sTCP:LISTEN | head -1)"
       bash scripts/start-daemon.sh
  2. Quit/resume Claude Code so the MCP bridges respawn on the new binary.
A Claude Code restart alone does NOT restart the persistent daemon.
NOTE
