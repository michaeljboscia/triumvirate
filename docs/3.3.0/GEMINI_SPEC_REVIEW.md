# Gemini Spec Review — v3.3.0 Live Agent Streaming

## Key Findings

1. **DROP Phase 1 entirely** — Claude Code doesn't support progressToken/notifications/progress from MCP servers
2. **Phase 2 is the only path** — but REQ-H05 must change: stream partial tool_result TEXT chunks, not JSON-RPC progress notifications
3. **Dual transport: YES** — stdio stays as legacy fallback, HTTP becomes canonical
4. **agent_executor refactor: HIGH RISK** — mitigate with sync wrapper for ABE/non-streaming callers
5. **WebSocket: run alongside** — don't replace v3.2.0 event schemas

## REQ Verdicts

| REQ | Verdict | Reason |
|-----|---------|--------|
| S01-S10 | DROP | Claude Code ignores progress notifications from MCP servers |
| H01 | AGREE | POST endpoint for JSON-RPC |
| H02 | AGREE | GET endpoint for SSE |
| H03 | AGREE | Session ID management |
| H04 | AGREE | Dual transport |
| H05 | MODIFY | Stream partial tool_result text chunks, NOT JSON-RPC progress |
| H06 | AGREE | Final result frame |
| H07 | AGREE | Use rmcp feature |
| H08 | AGREE | claude mcp add config |
| H09 | AGREE | Bearer auth |
| H10 | MODIFY | Test SSE text chunks, not progress frames |
| E01 | AGREE | AgentStreamEvent enum |
| E02 | AGREE | shared-types crate |
| E03 | AGREE | mpsc channel refactor |
| E04 | MODIFY | Emit alongside existing WS events, don't replace |
