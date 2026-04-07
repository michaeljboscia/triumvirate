#!/usr/bin/env bash
set -euo pipefail

LABEL="com.triumvirate.agentd"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DAEMON_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${DAEMON_DIR}/target/release/triumvirate-agentd"
PLIST="${HOME}/Library/LaunchAgents/${LABEL}.plist"
STDOUT_LOG="/tmp/triumvirate_agentd.out.log"
STDERR_LOG="/tmp/triumvirate_agentd.err.log"
USER_UID="$(id -u)"
SERVICE_TARGET="gui/${USER_UID}/${LABEL}"

usage() {
  cat <<EOF
Usage: $(basename "$0") <command>

Commands:
  install    Build binary, write plist, bootstrap + start service
  start      Start service
  stop       Stop service
  restart    Restart service
  status     Show launchd status
  logs       Tail service logs
  uninstall  Stop and remove launchd service/plist
EOF
}

write_plist() {
  mkdir -p "${HOME}/Library/LaunchAgents"
  cat > "${PLIST}" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>${LABEL}</string>
    <key>ProgramArguments</key>
    <array>
      <string>${BIN}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>${DAEMON_DIR}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>EnvironmentVariables</key>
    <dict>
      <key>PATH</key>
      <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${HOME}/.local/bin</string>
      <key>HOME</key>
      <string>${HOME}</string>
    </dict>
    <key>StandardOutPath</key>
    <string>${STDOUT_LOG}</string>
    <key>StandardErrorPath</key>
    <string>${STDERR_LOG}</string>
  </dict>
</plist>
EOF
}

build_release() {
  (cd "${DAEMON_DIR}" && cargo build --release -p triumvirate-agentd --bin triumvirate-agentd)
}

bootstrap() {
  launchctl bootout "gui/${USER_UID}" "${PLIST}" >/dev/null 2>&1 || true
  launchctl bootstrap "gui/${USER_UID}" "${PLIST}"
}

start_service() {
  launchctl kickstart -k "${SERVICE_TARGET}"
}

stop_service() {
  launchctl stop "${SERVICE_TARGET}" >/dev/null 2>&1 || true
}

status_service() {
  launchctl print "${SERVICE_TARGET}"
}

uninstall_service() {
  launchctl bootout "gui/${USER_UID}" "${PLIST}" >/dev/null 2>&1 || true
  rm -f "${PLIST}"
  echo "Uninstalled ${LABEL}"
}

cmd="${1:-}"
if [[ -z "${cmd}" ]]; then
  usage
  exit 1
fi

case "${cmd}" in
  install)
    build_release
    write_plist
    bootstrap
    start_service
    echo "Installed and started ${LABEL}"
    ;;
  start)
    start_service
    echo "Started ${LABEL}"
    ;;
  stop)
    stop_service
    echo "Stopped ${LABEL}"
    ;;
  restart)
    start_service
    echo "Restarted ${LABEL}"
    ;;
  status)
    status_service
    ;;
  logs)
    touch "${STDOUT_LOG}" "${STDERR_LOG}"
    tail -n 120 -f "${STDOUT_LOG}" "${STDERR_LOG}"
    ;;
  uninstall)
    uninstall_service
    ;;
  *)
    usage
    exit 1
    ;;
esac
