# Research 003: Go Language MCP Server Implementation

**Query:** Building MCP servers in Go/golang 2025-2026

## Go MCP Is Production-Ready

### Available SDKs
- **Official:** `modelcontextprotocol/go-sdk` (available early 2026)
- **Community:** `mark3labs/mcp-go`, `metoro-io/mcp-golang` (popular, battle-tested)
- All support stdio and HTTP/SSE transports

### Why Go for MCP
- Performance and simplicity — perfect fit for MCP's JSON-RPC model
- Native concurrency (goroutines) for parallel tool execution
- Single binary deployment — no npm, no node_modules
- Go's `encoding/json` is native — no jq dependency

### Building Blocks
- **Tools** — functions AI can invoke with parameters
- **Resources** — data sources AI can read
- **Prompts** — pre-defined templates for AI interactions
- Transport: stdio (for Claude Code) or HTTP/SSE (for networked agents)

### MCP Ecosystem Status
- Anthropic donated MCP to Linux Foundation (Dec 2025)
- Adopted by OpenAI, Google DeepMind, VS Code
- Security concerns: prompt injection, tool permissions — need careful implementation

## Implication for Triumvirate
We can write a Go MCP server that replaces the current Node.js inter-agent server. Single binary, proper concurrency, real error handling.

## Sources
modelcontextprotocol.io, bytesizego.com, navendu.me, dev.to, appliedgo.net, wu-boy.com, wikipedia.org, anthropic.com
