# APP_FLOW — v3.3.0 Live Agent Streaming

**Version:** 3.3.0
**Context:** CLI daemon, not a web app. "Screens" are CLI commands and terminal panes.

## User Journeys

### Journey 1: First-Time Setup (Golden Path)

```
1. User has triumvirate installed (cargo install or build from source)
2. User runs: triumvirate daemon
   → Daemon starts on :8080
   → Prints: "Triumvirate daemon v3.3.0 listening on 127.0.0.1:8080"
   → Axum HTTP + MCP SSE + WebSocket all active

3. User configures Claude Code:
   ~/.claude.json → mcpServers → triumvirate:
     command: "/path/to/triumvirate"
     args: ["proxy"]

4. User opens a second terminal pane (or Zellij split):
   triumvirate watch
   → Connects to ws://127.0.0.1:8080/ws
   → Prints: "connected to daemon v3.3.0 — watching agent_stream events"

5. User opens Claude Code in main pane:
   claude
   → Claude Code spawns `triumvirate proxy` as MCP subprocess
   → Proxy connects to daemon :8080/mcp
   → All 35+ MCP tools available
```

### Journey 2: Ask Agent with Live Streaming

```
Main pane (Claude Code):
  User types: "ask research to analyze our auth middleware"
  1s:  Claude calls ask_session MCP tool
  1s:  Proxy forwards JSON-RPC to daemon HTTP
  1s:  Daemon starts Gemini subprocess
  ...  (user sees spinner in Claude Code)
  15s: Claude displays: "The auth middleware has three issues..."

Watch pane (triumvirate watch):
  1s:  → Gemini: turn started [research]
  3s:  → Gemini: calling read_file (src/middleware/auth.rs)
  4s:  → Gemini: calling read_file (src/middleware/jwt.rs)
  8s:  → Gemini: generating response (1s elapsed)
  10s: → Gemini: generating response (3s elapsed)
  12s: → Gemini: generating response (5s elapsed)
  15s: → Gemini: responded (12,847 in / 1,203 out / 8,400 cached, 2 tools, 4.1s)
```

### Journey 3: Daemon Not Running

```
User opens Claude Code with proxy configured:
  Claude Code spawns `triumvirate proxy`
  Proxy tries :8080 → connection refused
  Proxy retries for 5 seconds (exponential backoff)
  Proxy exits with error:
    "daemon not reachable at 127.0.0.1:8080 — run 'triumvirate daemon' first"
  Claude Code shows MCP server failed to start

User opens watch CLI:
  triumvirate watch
  → "connecting to daemon at ws://127.0.0.1:8080/ws..."
  → "daemon not reachable — retrying in 2s..."
  → (retries with backoff until daemon starts or user Ctrl+C)
```

### Journey 4: Daemon Restarts Mid-Session

```
User is in an active Claude Code session with proxy running.
Admin restarts daemon (deploy new version).

Watch pane:
  → [connection lost — reconnecting...]
  → [reconnected to daemon v3.3.0]
  → (resumes streaming events)

Proxy:
  → In-flight tool call fails: "daemon connection lost"
  → Claude Code sees tool error, can retry
  → Proxy auto-reconnects (bounded backoff)
  → Next tool call succeeds through reconnected proxy
```

### Journey 5: Fallback to Local Stdio

```
User wants to use Triumvirate without the daemon (offline, debugging).
User changes ~/.claude.json:
  args: ["mcp"]   (instead of ["proxy"])
Restarts Claude Code.
→ All MCP tools work via local stdio execution
→ No streaming in watch pane (events don't flow through daemon)
→ Same tool results, just no live visibility
```

## Error States

| Error | Where Visible | What User Sees | Recovery |
|-------|--------------|----------------|----------|
| Daemon not running | Proxy startup | "daemon not reachable at 127.0.0.1:8080" | Run `triumvirate daemon` |
| Daemon not running | Watch CLI | "daemon not reachable — retrying..." | Run `triumvirate daemon` or Ctrl+C |
| Daemon crashes mid-call | Proxy | Tool call fails with error message | Proxy auto-reconnects, retry tool call |
| Agent subprocess dies | Watch pane | "→ Gemini: error (process exited code 1)" | Claude Code shows error in tool result |
| WS events dropped | Watch pane | "[events skipped, resynced at seq N]" | Informational only, resumes automatically |
| Auth failure on /mcp | Proxy | 401 Unauthorized | Check daemon.token file |
