# Pattern: Gemini Quick Search

Use Gemini's native MCP tools for real-time web search without leaving Claude's context.

## When to use

- You need current information (post training cutoff)
- Quick research question that doesn't need multi-source synthesis
- You want real Google results, not RAG over cached data

## How it works

Gemini CLI exposes native MCP tools for web search. If you install Gemini's own MCP server alongside Triumvirate, Claude can call `mcp__gemini__gemini-search` directly — no daemon needed.

```
mcp__gemini__gemini-search({ query: "latest Gemini CLI changelog 2026" })
```

This hits Google search and returns results in Claude's context. Fast, no daemon lifecycle.

## Setup

Gemini's MCP tools are separate from Triumvirate's inter-agent server. Add the Gemini MCP server to `~/.claude.json`:

```json
"gemini": {
  "command": "gemini",
  "args": ["mcp-server"]
}
```

Verify the tools are available in Claude by checking for `mcp__gemini__gemini-search`.

## When to use the daemon instead

Quick search (`gemini-search`) is stateless. For tasks that require:
- Reading your own codebase alongside web results
- Multi-turn follow-up questions
- Holding more than ~8K tokens of context

Use `spawn_daemon` + `ask_daemon` instead. The daemon has a 2M token context window and maintains conversation state across turns.

## Example

```
# Quick fact lookup
mcp__gemini__gemini-search({ query: "n8n webhook authentication options 2026" })

# vs. research that needs your codebase too → use daemon
spawn_daemon({ session_name: "n8n-research", cwd: "/path/to/project" })
ask_daemon({ daemon_id: "...", question: "Search for n8n webhook auth options, then read /path/to/project/workflows/webhook.json and tell me what we're missing." })
```
