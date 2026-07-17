#!/usr/bin/env bash
# Start the triumvirate daemon with the SAME env the MCP clients use.
#
# Why this exists: the daemon is the only process that dispatches agents (in a release
# build `ask_agent` always proxies to it), and it resolves the Gemini backend from its
# OWN env via gemini_backend(). A daemon hand-started from a shell inherits that shell's
# env, not the MCP block in ~/.claude.json. On 2026-07-16 that drift ran the daemon on
# the gemini-cli backend for four days while every client's config said agy. gemini-cli
# no longer works, so each request burned its whole 4-model faildown chain and failed,
# and the agy resilience layer (concurrency cap, RPM ceiling, circuit breaker) sat inert
# because every one of its gates is `matches!(backend, Agy)`.
#
# So: read the env from ~/.claude.json, the same block the MCP servers get. One source of
# truth. Do not hand-start the daemon any other way.

set -euo pipefail

CLAUDE_JSON="${CLAUDE_JSON:-$HOME/.claude.json}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$REPO_ROOT/daemon/target/release/triumvirate"
# Must match daemon-http::open_daemon_log() EXACTLY: TRIUMVIRATE_DAEMON_LOG, else
# $TRIUMVIRATE_HOME/daemon.log, else $HOME/.triumvirate/daemon.log. If these two disagree,
# a hand-started and an autostarted daemon write to different files and half the history
# vanishes depending on who booted it. TRIUMVIRATE_HOME is read from the MCP block below.
[[ -x "$BIN" ]] || { echo "no daemon binary at $BIN (cargo build --release -p triumvirate)" >&2; exit 1; }
[[ -f "$CLAUDE_JSON" ]] || { echo "no $CLAUDE_JSON to read env from" >&2; exit 1; }

# Pull the triumvirate MCP server's env block verbatim. Fail loud if it or the backend key
# is missing: starting with a silently-empty env is exactly the bug this script prevents.
ENV_KV="$(python3 - "$CLAUDE_JSON" <<'PY'
import json, sys

def find(o):
    if isinstance(o, dict):
        for k, v in o.items():
            if k == "triumvirate" and isinstance(v, dict) and "env" in v:
                return v["env"]
            found = find(v)
            if found is not None:
                return found
    return None

env = find(json.load(open(sys.argv[1])))
if env is None:
    sys.exit("no triumvirate MCP env block found")
# Fail loud on anything whose ABSENCE is silent at runtime. Each of these was, or would
# have been, an invisible outage:
#   TRIUMVIRATE_GEMINI_BACKEND -> defaults to the retired gemini-cli (the 4-day outage)
#   PATH                       -> we exec with `env -i`, so an absent PATH means the daemon
#                                 cannot find codex/sandbox-exec and every dispatch ENOENTs
#   POSTHOG_*                  -> capture() no-ops when unset, so a drifted daemon would be
#                                 undetectable precisely when you most need to detect it
required = ["TRIUMVIRATE_GEMINI_BACKEND", "PATH", "POSTHOG_HOST", "POSTHOG_API_KEY"]
missing = [k for k in required if not env.get(k)]
if missing:
    sys.exit("MCP env block is missing required keys: " + ", ".join(missing))
if env.get("TRIUMVIRATE_GEMINI_BACKEND") != "agy":
    sys.exit(
        "TRIUMVIRATE_GEMINI_BACKEND is %r, not 'agy'. gemini-cli is retired and does not "
        "work; refusing to start a daemon that would serve a dead backend."
        % env.get("TRIUMVIRATE_GEMINI_BACKEND")
    )
for k, v in env.items():
    # Raw KEY=VALUE, one per line. The caller reads these into an array, so values
    # containing spaces (OTEL_EXPORTER_OTLP_HEADERS is "Authorization=Bearer phc_...")
    # stay one argument. Quoting here would not survive word-splitting anyway.
    print(f"{k}={v}")
PY
)"

# readarray, not word-splitting: an env VALUE with a space must stay a single argv element.
ENV_ARGS=()
while IFS= read -r line; do
  [[ -n "$line" ]] && ENV_ARGS+=("$line")
done <<< "$ENV_KV"

# Resolve the log path from the SAME parse above (one pass, no second python, no silent
# `|| true` fallback). It must match daemon-http::open_daemon_log() exactly: a mismatch
# splits history between hand-started and autostarted daemons, which is how today's outage
# stayed invisible for four days in the first place.
TV_HOME_FROM_BLOCK=""
for kv in "${ENV_ARGS[@]}"; do
  [[ "$kv" == TRIUMVIRATE_HOME=* ]] && TV_HOME_FROM_BLOCK="${kv#TRIUMVIRATE_HOME=}"
done
LOG="${TRIUMVIRATE_DAEMON_LOG:-${TV_HOME_FROM_BLOCK:-$HOME/.triumvirate}/daemon.log}"

# Select daemons STRUCTURALLY, not by substring. `pgrep -f "triumvirate daemon"` matches any
# process whose command line merely CONTAINS that text: a tail, an editor, a grep, or the
# very shell running this script. This function SIGKILLs what it finds, so a false positive
# means killing the operator's terminal. Match argv[0] ending in /triumvirate AND argv[1]
# being exactly "daemon" — that excludes shells (argv[0] is /bin/zsh) and, critically, the
# per-session `triumvirate mcp` stdio servers, which must never be killed here.
daemon_pids() {
  ps -ax -o pid=,args= | awk '$2 ~ /(^|\/)triumvirate$/ && $3 == "daemon" { print $1 }'
}

# Stop whatever is running now, so :8080 is free and one daemon owns the limiters.
for pid in $(daemon_pids); do
  echo "stopping daemon $pid"
  kill -TERM "$pid" 2>/dev/null || true
done
sleep 2
for pid in $(daemon_pids); do
  echo "SIGKILL $pid (ignored TERM)"
  kill -KILL "$pid" 2>/dev/null || true
done

mkdir -p "$(dirname "$LOG")"

# Build the env assignments and exec. `env -i` is deliberate: inheriting the caller's shell
# is how the drift happened. Only HOME/USER plus the MCP block get through.
# nohup (not setsid, which macOS lacks) detaches the daemon from this shell's session so it
# survives the terminal that started it.
# RUST_LOG passes through when set, so an operator can raise the log level without editing
# the MCP env block. Everything else comes from ~/.claude.json, on purpose.
# RUST_LOG and the agy tuning knobs pass through when explicitly set in the calling env, so
# an operator can raise the log level or tighten a limit for a test WITHOUT editing the MCP
# block. They come LAST so they override the block's values. Everything else is the single
# source of truth from ~/.claude.json. Placed after "${ENV_ARGS[@]}" so a passthrough wins.
nohup env -i \
  HOME="$HOME" USER="${USER:-$(id -un)}" \
  "${ENV_ARGS[@]}" \
  ${RUST_LOG:+RUST_LOG="$RUST_LOG"} \
  ${TRIUMVIRATE_AGY_MAX_RPM:+TRIUMVIRATE_AGY_MAX_RPM="$TRIUMVIRATE_AGY_MAX_RPM"} \
  ${TRIUMVIRATE_AGY_MAX_CONCURRENT:+TRIUMVIRATE_AGY_MAX_CONCURRENT="$TRIUMVIRATE_AGY_MAX_CONCURRENT"} \
  ${TRIUMVIRATE_AGY_BREAKER_THRESHOLD:+TRIUMVIRATE_AGY_BREAKER_THRESHOLD="$TRIUMVIRATE_AGY_BREAKER_THRESHOLD"} \
  "$BIN" daemon >>"$LOG" 2>&1 < /dev/null &
disown 2>/dev/null || true

sleep 3

PID="$(daemon_pids | head -1 || true)"
if [[ -z "$PID" ]]; then
  echo "daemon failed to start; last log lines:" >&2
  tail -5 "$LOG" >&2 || true
  exit 1
fi

echo "daemon started: pid=$PID log=$LOG"
echo "backend: $(ps -Eww -p "$PID" | tr ' ' '\n' | grep '^TRIUMVIRATE_GEMINI_BACKEND=' || echo 'MISSING (drift!)')"
