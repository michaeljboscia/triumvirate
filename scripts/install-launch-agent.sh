#!/usr/bin/env bash
# Run the triumvirate daemon under launchd (decision D, temporal-migration design triumvirate-fleet.md).
#
# Before this the daemon was a child of whichever session's `triumvirate mcp` started it: no
# supervisor, gone when that session went, and "the daemon restarted" meant nothing. launchd
# starts it at login and restarts it after a crash or a kill (KeepAlive, throttled).
#
# The plist runs ~/.local/bin/triumvirate-start-daemon --foreground (installed by
# scripts/install.sh), which reads the env from ~/.claude.json exactly like a hand start and
# refuses a dead backend. It never points into a repo checkout or target/.
#
# `triumvirate install` writes the same plist (it used to write one with only TRIUMVIRATE_HOME in
# its env; fixed 2026-10-04). This script also loads it.
#
# Usage: bash scripts/install-launch-agent.sh            (install or update, then load)
#        bash scripts/install-launch-agent.sh --remove   (unload and delete)
set -euo pipefail

LABEL=com.triumvirate.daemon-v2
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LAUNCHER="$HOME/.local/bin/triumvirate-start-daemon"
DOMAIN="gui/$(id -u)"

if [[ "${1:-}" == "--remove" ]]; then
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  echo "removed $LABEL"
  exit 0
fi

[[ -x "$LAUNCHER" ]] || { echo "missing $LAUNCHER: run scripts/install.sh first" >&2; exit 1; }
mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.triumvirate"

cat > "$PLIST" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/bash</string>
    <string>$LAUNCHER</string>
    <string>--foreground</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>StandardOutPath</key>
  <string>$HOME/.triumvirate/launchd.out.log</string>
  <key>StandardErrorPath</key>
  <string>$HOME/.triumvirate/launchd.err.log</string>
</dict>
</plist>
PL
plutil -lint "$PLIST" >/dev/null

# Replace whatever runs now: the foreground launcher stops any hand-started daemon before it
# execs, so :8080 and the pid lock are free for the supervised one.
launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
launchctl bootstrap "$DOMAIN" "$PLIST"
sleep 6
PID="$(launchctl print "$DOMAIN/$LABEL" | awk '$1 == "pid" && $2 == "=" {print $3}')"
[[ -n "$PID" ]] || { echo "launchd loaded $LABEL but no pid; see $HOME/.triumvirate/launchd.err.log" >&2; exit 1; }
echo "launchd running $LABEL: pid=$PID"
ps -o pid=,ppid=,args= -p "$PID"
