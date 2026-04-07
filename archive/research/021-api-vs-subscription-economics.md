# Research 021: API vs Subscription Economics — HARD CONSTRAINT

**The user has 3 subscription accounts (Claude Pro/Max, Gemini Ultra, ChatGPT/Codex). NOT API keys with per-token billing.**

## What This Means

### Current Cost Model (Subscriptions)
- Claude: Max subscription (~$100/mo?) — unlimited* within rate limits
- Gemini: Ultra subscription (~$20/mo) — CLI access is FREE (no API key)
- Codex: ChatGPT/Codex subscription — CLI access included

### API Cost Model (What We Must AVOID for MVP)
- Claude API: $15/M input, $75/M output tokens (Opus)
- Gemini API: varies by model, per-token
- OpenAI API: varies, per-token
- Multi-agent debate with 3 models = 3x token cost per exchange
- A single DebateWorkflow could easily burn $5-20 in API tokens

## Architecture Implications

### CLI-First, API-Optional
The Go daemon MUST support CLI-backed connectors as the PRIMARY path:
- `claude --print --stream` for Claude (uses subscription, not API)
- `gemini` CLI for Gemini (uses Ultra subscription, $0)
- `codex` CLI for Codex (uses subscription)

API connectors are a FUTURE upgrade path when budget allows.

### This Is Actually Good
- CLI access gives us the FULL agent capabilities (tools, files, MCP)
- API access gives us raw completions (no tools, no file access unless we build it)
- The CLI IS the agent — the API is just the model

### The Real Architecture
```
triumvirate-agentd (Go)
├── Connector: Claude CLI (subprocess, streaming stdout)
├── Connector: Gemini CLI (subprocess, streaming stdout)  
├── Connector: Codex CLI (subprocess, streaming stdout)
├── Future: Claude API (direct HTTP, when budget allows)
├── Future: Gemini API (direct HTTP, when budget allows)
└── Future: OpenAI API (direct HTTP, when budget allows)
```

### What Changes from the Twin Visions
- Gemini's "persistent WebSocket connections" → deferred to API phase
- "Connection pooling" → not needed for CLI connectors
- "Keep agents warm" → keep CLI subprocesses ALIVE instead of spawn/kill per request
- The key insight: a long-running `claude --print --stream` subprocess IS a warm connection

### Persistent CLI Sessions
Instead of spawning a new CLI process per request:
1. Start `claude -p --output-format stream-json` as a long-lived subprocess
2. Pipe tasks to stdin, read streaming JSON from stdout
3. Process stays warm — no cold start per request
4. Same for gemini and codex CLIs

This is CHEAPER than API and gives us MORE capabilities.

## Searches Still Needed
- Claude Code `--print --input-format stream-json` for bidirectional streaming
- Gemini CLI subprocess management — keep alive patterns
- Codex CLI programmatic access patterns
