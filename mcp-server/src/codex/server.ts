/**
 * Codex Agent MCP Server — entry point.
 *
 * This server wraps the Codex CLI behind MCP tools:
 *   - send_message: `codex exec -` (stdin, works anywhere)
 *   - code_review:  `codex review --scope` (git-aware, needs repo)
 *
 * Run as: node dist/codex/server.js
 * Transport: stdio (Claude Code connects via stdin/stdout)
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { registerCodexTools } from "./tools.js";

const server = new McpServer({
  name: "codex-agent",
  version: "1.0.0",
});

registerCodexTools(server);

// Global error handlers to prevent silent crashes
process.on("uncaughtException", (err) => {
  console.error(`[codex-agent] Uncaught exception: ${err.message}`);
  console.error(err.stack);
  process.exit(1);
});

process.on("unhandledRejection", (reason) => {
  console.error(`[codex-agent] Unhandled rejection: ${reason}`);
  process.exit(1);
});

// Connect to Claude Code via stdio transport
const transport = new StdioServerTransport();
await server.connect(transport);
