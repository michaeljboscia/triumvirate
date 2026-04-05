# APP_FLOW — Triumvirate v2

**Version:** 1.0
**Date:** 2026-04-05
**Cross-refs:** PRD.md (FEAT-IDs), DESIGN_SYSTEM.md, FRONTEND_GUIDELINES.md

---

## Entry Points

### 1. Daemon Boot (CLI)

```
$ triumvirate-agentd
  → Load config (~/.triumvirate/config.toml)
  → Initialize fabric, SQLite, agents
  → Open browser to http://127.0.0.1:8080
```

User interacts exclusively via the web dashboard (GR1-D1). Terminal shows health logs only.

### 2. Resume Session (CLI)

```
$ triumvirate-agentd --resume <session-id>
  → Load previous session from SQLite
  → Restore agent states and conversation history
  → Dashboard shows previous context
```

---

## Dashboard Routes

| Route | View | Data Required | FEAT |
|-------|------|---------------|------|
| `/` | Tasks View (default) | Active tasks, assigned agents, progress | FEAT-015 |
| `/agents` | Agents View | Running agents, health, streaming output | FEAT-016 |
| `/memory` | Memory Viewer | All memories from SQLite | FEAT-012 |
| `/sessions` | Session History | Stenographer logs, searchable | FEAT-011 |
| `/workflows` | Workflow Panel | State machine states, steps, retries | FEAT-007 |
| `/quota` | Quota Dashboard | Per-agent token spend, budget bars | FEAT-017 |
| `/settings` | Config Editor | Agent config, backend selection | FEAT-026 |

---

## User Journeys

### Journey 1: Single Agent Conversation

```
User opens dashboard → Tasks View (empty)
  → Types in input: "How should we handle auth?"
  → Daemon routes to Claude (lead for architecture, GR1-D3)
  → Claude-1 output streams in real-time to Tasks View
  → Digest sent to Gemini-1 and Codex-1 (same task, FEAT-018)
  → Gemini-1 self-activates: "JWT has a revocation gap"
  → Dashboard shows both responses under the task
  → User types "@codex implement the auth module"
  → Codex-1 begins implementation
```

**Error states:**
- Agent unresponsive: dashboard shows "Claude-1: unresponsive (30s)" with retry spinner
- Agent dead: dashboard shows "Claude-1: dead — restarting..." with backoff timer
- Quota exhausted: dashboard shows "Claude: 92% quota, digest OFF" — only @-mentions reach Claude

**Empty state:**
- No agents configured: dashboard shows setup wizard pointing to config file
- No conversation yet: input area with hint "Type a message or /debate a topic"

### Journey 2: Fleet Task

```
User opens dashboard → Types: "Build the auth system with 3 Claudes and 2 Codexes"
  → Daemon parses fleet request: {claude: 3, codex: 2}
  → Fleet Workflow starts (FEAT-010):
    1. Wave 0: Claude-1 defines contracts (interfaces, types, API shapes)
    2. Contracts displayed in dashboard for user approval
    3. User approves → daemon provisions:
       - 5 git worktrees (one per fleet member)
       - 5 CLI subprocesses (3 claude, 2 codex)
       - Task list with dependencies in SQLite
    4. Tasks View shows: "Auth System" task group with 5 sub-tasks
    5. Each agent claims a task → status: in_progress
    6. Agent output streams under its sub-task in Tasks View
    7. Agent completes → task status: completed → dependents unblocked
    8. All tasks done → sequential merge begins
    9. Merge panel shows: branch-by-branch merge with diff preview
    10. Conflicts → human resolution in dashboard
    11. All merged → "Auth System: COMPLETE"
```

**Error states:**
- Agent dies mid-task: auto-restart, resume from last checkpoint
- Merge conflict: dashboard shows conflict diff with resolve options
- Quota exhausted mid-fleet: daemon pauses that agent's tasks, redistributes to remaining agents
- Worktree creation fails: dashboard shows error, fleet degrades to N-1

**Empty state:**
- Fleet not supported (single-instance config): input hint changes to "Fleet requires multiple agent instances. See /settings"

### Journey 3: Debate

```
User types: "/debate Should we use Redis or Postgres for caching?"
  → Debate Workflow starts (FEAT-009):
    1. Claude-1 proposes: claim + data + warrant
    2. Gemini-1 challenges: rebuttal + counter-evidence
    3. Codex-1 votes with reasoning
    4. Dashboard shows debate in structured format:
       - PROPOSAL panel (Claude's claim)
       - CHALLENGE panel (Gemini's rebuttal)
       - VOTE panel (all positions)
    5. Winner declared: "Postgres (2-1)"
    6. Decision auto-extracted to memory (FEAT-013)
```

### Journey 4: Session Resume

```
User starts new session → daemon boots
  → SQLite loaded: previous memories injected into agent prompts
  → Dashboard shows: "Previous session: abc-123 (47 min, 3 decisions)"
  → User clicks "Resume" → conversation continues with full context
  → Or starts fresh → agents still have shared memory from last session
```

---

## Input Handling

### Message Routing

| Input Pattern | Route | Example |
|---------------|-------|---------|
| `@claude <msg>` | Direct to Claude | `@claude design the auth API` |
| `@gemini <msg>` | Direct to Gemini | `@gemini research JWT best practices` |
| `@codex <msg>` | Direct to Codex | `@codex implement auth.rs` |
| `/debate <topic>` | Debate Workflow | `/debate Redis vs Postgres` |
| `/fleet <spec>` | Fleet Workflow | `/fleet 3 claude, 2 codex: build auth` |
| `/status` | Dashboard panel | Shows fleet status, quota, health |
| Plain text | Lead agent (GR1-D3) | `How should we handle auth?` |

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Ctrl+Enter / Cmd+Enter | Send message |
| Escape | Interrupt active agent |
| Tab | Toggle Tasks/Agents view |

---

## WebSocket Event Stream

Browser connects to `ws://127.0.0.1:8080/ws`. Events are JSON:

```json
{"type": "agent_output", "agent": "claude-1", "task": "auth-design", "content": "...", "streaming": true}
{"type": "health_change", "agent": "gemini-1", "status": "ready"}
{"type": "task_update", "task": "auth-impl", "status": "completed", "agent": "codex-1"}
{"type": "quota_update", "agent_type": "claude", "used_pct": 45}
{"type": "decision_proposed", "content": "Use JWT for auth tokens", "source": "claude-1"}
{"type": "merge_conflict", "task": "auth-system", "file": "src/types.rs", "branches": ["claude-2", "codex-1"]}
```

---

## Data Flow Summary

```
Human Input (browser)
  → WebSocket → daemon
  → Routing decision (lead agent / @-mention / /command)
  → Agent stdin (JSON)
  → Agent stdout (JSON) → fabric → WebSocket → browser
  → Digest to idle agents on same task
  → Stenographer captures to session log
  → Decision extraction → dashboard confirmation → SQLite
```
