---
name: inter-agent-protocol
description: Full inter-agent communication protocol, peer review requirements, and auto-escalation rules. Use when sending messages to Gemini or Codex, doing peer review, or after 3 consecutive failures on the same problem.
---

# Inter-Agent Communication Protocol

**Full spec:** `~/.claude/INTER_AGENT_PROTOCOL.md`
**Session log spec:** See `SESSION_LOG_SPEC.md` in the triumvirate repo root

## The Trifecta

| Agent | Command | Config | Model | Best For |
|-------|---------|--------|-------|----------|
| Claude | `claude` | `~/.claude/` | Opus 4.6 | Primary coding, orchestration |
| Gemini | `gemini` | `~/.gemini/` | Gemini Pro (2M context) | Research, large context, web search |
| Codex | `codex` | `~/.codex/` | GPT-5.2-Codex | Code generation, refactoring |

All three share: Supabase, GitHub, HubSpot, Google Drive, ClickUp.

## Sending Messages (MCP — Primary)

### DEFAULT: spawn_session + ask_session

**To Gemini:**
```
mcp__triumvirate__spawn_session(cwd: "/path/to/project")  → session_name
mcp__triumvirate__ask_session(session_name, question)
mcp__triumvirate__dismiss_session(session_name)
```

**To Codex:**
```
mcp__triumvirate__spawn_session(cwd: "/path/to/project")  → session_name
mcp__triumvirate__ask_session(session_name, question)
mcp__triumvirate__dismiss_session(session_name)
```

### Performance Rules (v2.1)

- **Pre-digest context**: Claude reads files, extracts key details, sends inline. Receiver thinks, doesn't hunt.
- **Never tell receiver "full context in session log"** — send the actual context inline
- **Session logs are fallback**, not prerequisite — receivers answer the question first

### Web Search — Always Gemini Native MCP

| Need | Tool |
|------|------|
| Quick web search | `mcp__gemini__gemini-search` |
| Deep research | `mcp__gemini__gemini-deep-research` |

Inter-agent = for OUR stuff (code, Supabase, infrastructure).
Gemini native MCP = for EVERYTHING external (web, market research).

## Siblings Are First-Party MCP Peers

All three agents have identical MCP access. **Never relay data between agents.** Give the sibling a task and let it pull data itself.

**WRONG:** Claude queries Supabase → pastes results → asks Gemini to analyze
**RIGHT:** Claude spawns Gemini → "query Supabase for X, analyze, report back"

## Multi-Agent Peer Review (Mandatory for Critical Work)

**Required for:** Legal/compliance code, security implementations, core business logic, database schema changes, spatial/mathematical algorithms, API integrations.

**Workflow:**
1. Implement + test + commit
2. Send to Gemini (requirements validation) AND Codex (implementation review) — parallel
3. Incorporate feedback
4. Document review results in session log

## Auto-Escalation: 3 Failures = Ask for Help

**After 3 consecutive failures on the SAME problem:**
1. STOP attempting to fix it yourself
2. Send to BOTH Gemini AND Codex (parallel MCP calls)
3. Include: problem description, all 3 attempts, what each produced
4. Ask: "What am I missing?"

**Counts as failure:** Test fails 3x, error persists 3x, can't understand behavior 3x.
**Does NOT count:** Iterating on design, progressive implementation, refactoring.
