# Tech Stack — Triumvirate v2.1 "Flow State"

**Status:** Final
**Inherits:** `docs/v2/TECH_STACK.md` (all v2.0 dependencies unchanged)

---

## New Crate: agent-adapter

| Dependency | Version | Purpose |
|-----------|---------|---------|
| serde | 1.x (workspace) | Serialization for WorkingState types |
| serde_json | 1.x (workspace) | NDJSON line parsing |
| tokio | 1.x (workspace) | Async runtime (mpsc channels, time) |
| anyhow | 1.x (workspace) | Error handling |
| tracing | 0.1.x (workspace) | Structured logging |

**No new external dependencies.** All deps already in the workspace. agent-adapter does NOT add `rmcp`, `axum`, `reqwest`, or any networking crate.

## Modified Dependencies

None. No version changes. No new workspace-level dependencies.

## Build Impact

- One new crate added to workspace (incremental compilation)
- No new proc macros
- No new native/C dependencies
- No new feature flags
- Estimated compile time impact: <2s incremental

## Runtime Dependencies

| Dependency | Version | Required By |
|-----------|---------|-------------|
| Gemini CLI | 0.35.0+ | FEAT-001 (stream-json format) |
| Codex CLI | 0.118.0+ | FEAT-002 (exec --json format, TokenCountEvent) |

**Note:** Minimum CLI versions based on features used. Older versions may work but produce fewer event types (graceful degradation — unknown events skipped per AR-20).

## Platform Requirements

| Requirement | Reason | Fallback |
|-------------|--------|----------|
| Unix/macOS | `process_group(0)` via `setpgid` (REQ-014) | None — Unix-only (AR-7) |
| libc crate | `killpg` for process group termination | Already available via tokio |

## Environment Variables (New)

| Variable | Type | Default | Description |
|----------|------|---------|-------------|
| `TRIUMVIRATE_AGENT_VERBOSITY` | string | `standard` | quiet, standard, detailed, raw |
| `TRIUMVIRATE_GEMINI_STREAMING` | bool | `true` | false = batch mode fallback |
| `TRIUMVIRATE_CODEX_PROTOCOL` | string | `exec` | exec (v2.1), app-server (v2.2) |

## Environment Variables (Removed)

| Variable | Reason |
|----------|--------|
| `TRIUMVIRATE_DAEMON_ASK_TWINS_URL` | ask_twins removed (FEAT-008) |
| `TRIUMVIRATE_ASK_TWINS_ROLE_ADAPT` | ask_twins removed (FEAT-008) |
