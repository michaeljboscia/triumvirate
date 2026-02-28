# Setup: Gemini CLI

Gemini is the passive target in Triumvirate — the MCP server spawns and manages it. You don't need to configure anything special. You just need it installed and authenticated.

## Install

```bash
npm install -g @google/gemini-cli
```

Or via Homebrew:

```bash
brew install gemini-cli
```

Verify:

```bash
gemini --version
```

## Authenticate

Gemini CLI uses your Google account or an API key.

**Interactive login (Google account):**

```bash
gemini auth login
```

**API key:**

```bash
export GEMINI_API_KEY="your-key-here"
```

Or add it to `~/.gemini/settings.json`:

```json
{
  "apiKey": "your-key-here"
}
```

Verify authentication works:

```bash
gemini -p "" <<< "Say hello"
```

## That's it

You don't need to configure `--approval-mode`, `--output-format`, or `--include-directories` in your Gemini settings. The MCP server passes all of these flags automatically on every invocation:

- `--approval-mode yolo` — disables interactive approval prompts (required for headless operation)
- `--output-format text` — disables the rich TUI (required for programmatic output)
- `--include-directories ~` — expands file access beyond the session's working directory

If you configure any of these in your own Gemini settings, they'll be overridden by the flags the MCP server passes.

## Verify the daemon works

Once the MCP server is wired into Claude Code (see `claude-code.md`), restart Claude and run:

```
spawn_daemon({ session_name: "test" })
```

You should see:

```
Gemini daemon ready.

Daemon ID: gd_abc123
Session: daemon-test
...
```

Dismiss it when done:

```
dismiss_daemon({ daemon_id: "gd_abc123" })
```

Expected: `"Gemini daemon gd_abc123 dismissed (soft). Session files preserved..."`

The soft dismiss means Gemini's conversation history is kept on disk. Running `spawn_daemon({ session_name: "test" })` again will resume the session instantly with zero token cost.

## Troubleshooting

**`gemini: command not found`**
The `GEMINI_CLI_PATH` env var in your MCP config points to the wrong location. Find the binary:
```bash
which gemini
```
Set `GEMINI_CLI_PATH` in your `~/.claude.json` MCP config to the output.

**`Failed to start Gemini session: authentication error`**
Run `gemini auth login` or set `GEMINI_API_KEY`.

**Daemon always starts fresh instead of resuming**
The session dir or Gemini's history dir was deleted. Both `~/.gemini/daemon-sessions/<session>/` and `~/.gemini/tmp/<session>/` must exist for resume to work. Start fresh with a new session name.
