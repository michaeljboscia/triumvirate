# Triumvirate MCP Server

The TypeScript MCP server at the core of Triumvirate. Wraps the Gemini and Codex CLIs with a daemon API that Claude (and Codex) can call via the Model Context Protocol.

## What it does

Exposes two MCP servers — one for Gemini, one for Codex — each providing:

| Tool | Description |
|------|-------------|
| `spawn_daemon` | Start a persistent session. Named sessions survive MCP restarts. |
| `ask_daemon` | Ask a question. Full conversation history maintained via `-r latest`. |
| `dismiss_daemon` | Soft dismiss (keeps session files) or hard dismiss (deletes everything). |
| `list_daemons` | List active and hibernated sessions. |
| `list_scratchpad` | List shared scratchpad files for the current project. |
| `write_scratchpad` | Write a note or artifact to the shared scratchpad. |

Gemini additionally exposes:
| Tool | Description |
|------|-------------|
| `send_message` | Fire-and-forget async request (SYN/ACK pattern). |
| `get_response` | Retrieve response from a `send_message` job. |
| `list_jobs` | List outstanding async jobs. |
| `summarize_transcript` | Summarize a transcript (used by pre-compact hooks). |

## Architecture

**No PTY.** Each `ask_daemon` is a fresh subprocess invocation of the CLI (`gemini -r latest` or `codex exec resume <thread>`). Conversation continuity comes from the CLIs' own session storage — not from a persistent process or PTY. Each call takes ~2-3s (Gemini) or ~7s (Codex).

**Session persistence.** Gemini sessions are keyed by a directory name under `~/.gemini/daemon-sessions/`. Named sessions (`session_name: "arch-auditor"`) use a deterministic directory name, so they survive MCP server restarts and can be resumed with zero token cost. Codex sessions use thread IDs that persist in Codex's own storage.

**Shared scratchpad.** All three agents share a scratchpad at `<project-root>/.claude/scratchpad/`. Files are tagged by agent/daemon ID and reaped on dismiss. 2-hour TTL.

**Model fallback (Gemini).** If a model quota-exhausts, the server automatically falls back through the chain: `gemini-3-pro-preview → gemini-2.5-pro → gemini-3-flash-preview → gemini-2.5-flash`. State persists in `~/.gemini/quota-state.json` with a 1-hour TTL.

## Build

```bash
npm install
npm run build
```

Requires Node.js 20+. Output goes to `dist/`.

## Entry points

| File | Purpose |
|------|---------|
| `src/gemini/server.ts` | Gemini MCP server entry point |
| `src/gemini/tools.ts` | All Gemini tool definitions |
| `src/gemini/model-fallback.ts` | Model quota tracking and fallback chain |
| `src/codex/server.ts` | Codex MCP server entry point |
| `src/codex/tools.ts` | All Codex tool definitions |
| `src/shared/cli-executor.ts` | Subprocess execution with timeout, retry, heartbeat |
| `src/shared/scratchpad-reaper.ts` | Scratchpad lifecycle management |
| `src/shared/session-log-finder.ts` | Finds the latest session log for a project |
| `src/shared/context-detector.ts` | Detects project taxonomy from cwd |
| `src/shared/message-formatter.ts` | Formats inter-agent protocol messages |
| `src/shared/job-store.ts` | Async job tracking for send_message/get_response |
| `src/shared/outbox-logger.ts` | Logs outbound messages for debugging |

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `GEMINI_CLI_PATH` | `gemini` | Path to the Gemini CLI binary |
| `CODEX_CLI_PATH` | `codex` | Path to the Codex CLI binary |
| `DEFAULT_TIMEOUT_MS` | `300000` | Default tool call timeout (5 min) |
| `MAX_TIMEOUT_MS` | `600000` | Maximum allowed timeout (10 min) |

## Session persistence in depth

Gemini session files live in two places:

```
~/.gemini/daemon-sessions/daemon-<name>/   ← session working directory (cwd for CLI)
~/.gemini/tmp/daemon-<name>/               ← Gemini's conversation history (-r latest reads this)
```

Both must exist for a session to be resumable. `spawn_daemon` checks both before deciding whether to bootstrap fresh or resume.

`dismiss_daemon` (soft, default) removes the session from the in-memory registry but leaves both directories on disk. `dismiss_daemon({ hard: true })` deletes both.

`list_daemons` scans `~/.gemini/daemon-sessions/` for hibernated sessions (on disk, not in the current in-memory registry) and reports them with a resume hint.

## Security

Both CLIs run with their sandboxes disabled in daemon mode:

- **Codex:** `--dangerously-bypass-approvals-and-sandbox`
- **Gemini:** `--approval-mode yolo`

This is intentional — the agents need to read and write files in your project. Only run Triumvirate with agents you trust on a machine you control.
