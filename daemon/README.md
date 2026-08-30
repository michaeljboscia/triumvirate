# Triumvirate Daemon v2

A Rust workspace for the Triumvirate daemon + MCP bridge runtime.

## Workspace Crates

- `crates/triumvirate`: main binary (`mcp`, `daemon`, `install`, `uninstall`, `status`, `doctor`)
- `crates/daemon-core`: daemon/runtime shared helpers (state IO, dead-drop, queue, launchd, context)
- `crates/mcp-bridge`: MCP-facing bridge helpers (routing inputs, env/URL parsing, command resolution)
- `crates/agent-adapter`: unified agent event/types/parsers (Gemini stream-json + Codex exec-json)
- `crates/shared-types`: shared request/response DTOs used across crate boundaries

## Build & Test

```bash
cargo build
cargo test
```

## CLI Commands

```bash
# Run stdio MCP bridge
cargo run -p triumvirate -- mcp

# Run daemon HTTP process
cargo run -p triumvirate -- daemon

# Install launchd plist for autostart
cargo run -p triumvirate -- install

# Remove launchd plist
cargo run -p triumvirate -- uninstall

# Print status snapshot
cargo run -p triumvirate -- status

# Print local diagnostics
cargo run -p triumvirate -- doctor
```

## Runtime Environment Variables

### Daemon networking

- `TRIUMVIRATE_DAEMON_BIND_ADDR`
  - Daemon listen address for `daemon` mode.
  - Default: `127.0.0.1:8080`
- `TRIUMVIRATE_DAEMON_BASE_URL`
  - Base URL for MCP-side daemon HTTP calls.
  - If unset, bridge derives from `TRIUMVIRATE_DAEMON_BIND_ADDR` as `http://<bind-addr>`.
  - Final fallback: `http://127.0.0.1:8080`

### Explicit daemon endpoint overrides

- `TRIUMVIRATE_DAEMON_HEALTH_URL` (`/health`)
- `TRIUMVIRATE_DAEMON_URL` (`/status`)
- `TRIUMVIRATE_DAEMON_ASK_AGENT_URL`
- `TRIUMVIRATE_DAEMON_MEMORY_WRITE_URL`
- `TRIUMVIRATE_DAEMON_MEMORY_READ_URL`
- `TRIUMVIRATE_DAEMON_SCRATCHPAD_WRITE_URL`
- `TRIUMVIRATE_DAEMON_SCRATCHPAD_LIST_URL`
- `TRIUMVIRATE_DAEMON_OUTBOX_RECENT_URL`
- `TRIUMVIRATE_DAEMON_FALLBACK_LIST_URL`
- `TRIUMVIRATE_DAEMON_FALLBACK_ACK_URL`
- `TRIUMVIRATE_DAEMON_FALLBACK_GC_URL`

### MCP/daemon execution mode

- `TRIUMVIRATE_MCP_USE_DAEMON`
  - Truthy values (`1`, `true`, `yes`, `on`) route MCP tools through daemon HTTP.
  - Default: disabled/false.

### Autostart behavior

- `TRIUMVIRATE_DAEMON_AUTOSTART`
  - Falsey values (`0`, `false`, `no`, `off`) disable one-shot autostart attempts.
  - Default: enabled/true.
- `TRIUMVIRATE_DAEMON_AUTOSTART_DRYRUN`
  - Truthy values simulate autostart without spawning daemon process.

### Agent command resolution

- `TRIUMVIRATE_GEMINI_BIN`, `TRIUMVIRATE_GEMINI_ARGS`
- `TRIUMVIRATE_CODEX_BIN`, `TRIUMVIRATE_CODEX_ARGS`
- `TRIUMVIRATE_GROK_BIN`, `TRIUMVIRATE_GROK_ARGS` (binary is `grok`; there is no `supergrok` executable)
- `TRIUMVIRATE_GROK_MODEL`, `TRIUMVIRATE_GROK_EFFORT`
- `TRIUMVIRATE_GROK_MAX_TURNS` (default `20`; every turn re-ships the whole system prompt, so turns are the unit of spend)
- `TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS` (default `900`)
- `TRIUMVIRATE_GROK_STREAMING` (default on; `0` falls back to batch `json`)
- `TRIUMVIRATE_GROK_YOLO` (default off; `1` adds `--always-approve`)
- `TRIUMVIRATE_GROK_SANDBOX` (default `read-only`; also `workspace`, `strict`, or `off`. An unknown profile does
  NOT fail grok, it warns and runs uncontained, so the runner treats that warning as a hard error)
- `XAI_API_KEY` is inherited if set, but a cached `grok login --oauth` (subscription) is sufficient and is the
  preferred path. Note the two bill different accounts; `triumvirate doctor` reports which is in use.
- `TRIUMVIRATE_GEMINI_STREAMING`
  - Falsey disables live stream parse path and falls back to batch parse.
- `TRIUMVIRATE_AGENT_VERBOSITY`
  - `minimal|normal|verbose` progress-event filter.

### Data root

- `TRIUMVIRATE_HOME`
  - Root for daemon token, sessions, outbox, memory, scratchpad, and dead-drop.
  - Default: `~/.triumvirate`

## Operational Notes

- `doctor` prints token file path/existence, launchd plist path/existence, configured bind address, and daemon reachability.
- `doctor` also prints resolved daemon routing URLs (`daemon_base_url`, `daemon_status_url`) so env-derived endpoint behavior is explicit.
- daemon `/health` and `/status` payloads include `daemon_bind_addr` for runtime network observability.
- `status` reports active sessions, supported agents, fallback queue state, and daemon bind address.
- `status` degrades gracefully: if daemon HTTP is unreachable, it still returns a local snapshot (with `daemon_reachable: false`) instead of exiting with an error.
- Dead-drop fallback tickets live under `<TRIUMVIRATE_HOME>/dead-drop`.
