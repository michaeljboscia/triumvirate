# Setup: Claude Code

## Prerequisites

- Claude Code installed (`claude --version`)
- Gemini CLI installed (`gemini --version`)
- Codex CLI installed (`codex --version`)
- Node.js 20+

## Build the MCP server

```bash
cd triumvirate/mcp-server
npm install
npm run build
chmod +x start-gemini.sh
```

## Wire into Claude Code

Add both servers to `~/.claude.json` under the `mcpServers` key:

```json
{
  "mcpServers": {
    "inter-agent-gemini": {
      "command": "/bin/bash",
      "args": ["/absolute/path/to/triumvirate/mcp-server/start-gemini.sh"],
      "env": {
        "GEMINI_CLI_PATH": "/usr/local/bin/gemini",
        "DEFAULT_TIMEOUT_MS": "300000",
        "MAX_TIMEOUT_MS": "600000"
      }
    },
    "inter-agent-codex": {
      "command": "node",
      "args": ["/absolute/path/to/triumvirate/mcp-server/dist/codex/server.js"],
      "env": {
        "CODEX_CLI_PATH": "/usr/local/bin/codex",
        "DEFAULT_TIMEOUT_MS": "300000",
        "MAX_TIMEOUT_MS": "600000"
      }
    }
  }
}
```

Replace `/absolute/path/to/triumvirate` with the actual path.

If `gemini` and `codex` are in your PATH, you can omit `GEMINI_CLI_PATH` and `CODEX_CLI_PATH`.

## Verify

Restart Claude Code and check that the tools are available:

```
mcp__inter-agent-gemini__spawn_daemon
mcp__inter-agent-gemini__ask_daemon
mcp__inter-agent-gemini__dismiss_daemon
mcp__inter-agent-codex__spawn_daemon
mcp__inter-agent-codex__ask_daemon
mcp__inter-agent-codex__dismiss_daemon
```

## Usage

```typescript
// Spawn a Gemini daemon
spawn_daemon({ cwd: "/path/to/project" })
// → { daemon_id: "gd_abc123" }

// Ask it something
ask_daemon({ daemon_id: "gd_abc123", question: "Read /path/to/file.ts and explain the auth flow" })
// → { text: "The auth flow works by..." }

// Soft dismiss (default) — session files preserved, resumable later
dismiss_daemon({ daemon_id: "gd_abc123" })
// → { text: "Gemini daemon gd_abc123 dismissed (soft). Session files preserved at: ~/.gemini/tmp/daemon-gd_abc123" }

// Resume the same session in a future call (zero token cost — no re-feed)
spawn_daemon({ session_name: "gd_abc123" })
// → { text: "Gemini daemon resumed (existing session)..." }

// Hard dismiss — permanently delete session files
dismiss_daemon({ daemon_id: "gd_abc123", hard: true })
// → { text: "Gemini daemon gd_abc123 permanently dismissed. Session files deleted." }
```
