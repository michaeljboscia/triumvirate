# Setup: Codex (Codex→Gemini channel)

This gives Codex the ability to spawn Gemini daemons autonomously — without Claude in the middle.

## Prerequisites

Complete `claude-code.md` setup first (the MCP server must be built).

## Wire into Codex

Add to `~/.codex/config.toml`:

```toml
[mcp_servers.inter-agent-gemini]
command = "/bin/bash"
args = ["/absolute/path/to/triumvirate/mcp-server/start-gemini.sh"]

[mcp_servers.inter-agent-gemini.env]
GEMINI_CLI_PATH = "/usr/local/bin/gemini"
DEFAULT_TIMEOUT_MS = "300000"
MAX_TIMEOUT_MS = "600000"
```

## Update Codex standing instructions

Add to `~/.codex/AGENTS.md`:

```markdown
## Gemini as Context Window

You have access to `mcp__inter-agent-gemini__spawn_daemon`, `ask_daemon`, and `dismiss_daemon`.

Use Gemini when:
- A file you need to reason about holistically is >500 lines AND you don't already have its content in context
- Your task description says "use Gemini for context"

How:
1. `spawn_daemon(cwd: <project-root>)` → daemon_id
2. `ask_daemon(id, "Read /full/path/to/file and explain X")` — pass PATHS, not content
3. Ask follow-up questions as needed
4. `dismiss_daemon(id)` when done

Never pass file content between agents. Pass paths and let Gemini read directly from disk.
Always dismiss when done — this triggers Gemini's session log.
```

## How it works

When Claude dispatches Codex with a large task, Codex can decide on its own to spin up a Gemini daemon:

```
Claude → "Review /path/to/large-codebase for security issues"

Codex:
  spawn_daemon(cwd: /path/to/large-codebase)
  ask_daemon(id, "Read /path/to/large-codebase/src/auth.ts and identify security issues")
  ask_daemon(id, "Read /path/to/large-codebase/src/api.ts and check input validation")
  dismiss_daemon(id)
  → Returns complete security review to Claude
```

Claude never loads the files. Codex never loads the full file content. Gemini holds it all.
