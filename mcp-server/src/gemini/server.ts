/**
 * Gemini Agent MCP Server — entry point.
 *
 * This server wraps the Gemini CLI (`gemini -p "..."`) behind MCP tools,
 * eliminating shell escaping issues that plague the bash-based skills.
 *
 * Run as: node dist/gemini/server.js
 * Transport: stdio (Claude Code connects via stdin/stdout)
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { registerGeminiTools } from "./tools.js";

const server = new McpServer({
  name: "gemini-agent",
  version: "1.0.0",
});

registerGeminiTools(server);

// Global error handlers to prevent silent crashes
process.on("uncaughtException", (err) => {
  console.error(`[gemini-agent] Uncaught exception: ${err.message}`);
  console.error(err.stack);
  process.exit(1);
});

process.on("unhandledRejection", (reason) => {
  console.error(`[gemini-agent] Unhandled rejection: ${reason}`);
  process.exit(1);
});

// Connect to Claude Code via stdio transport
const transport = new StdioServerTransport();
await server.connect(transport);
