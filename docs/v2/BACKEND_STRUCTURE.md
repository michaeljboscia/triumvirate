# BACKEND_STRUCTURE — Triumvirate v2

**Version:** 1.0
**Date:** 2026-04-05
**Cross-refs:** PRD.md (FEAT-IDs), TECH_STACK.md, IMPLEMENTATION_PLAN.md

---

## SQLite Schema — memory.db

### memories

```sql
CREATE TABLE memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    memory_type TEXT NOT NULL CHECK(memory_type IN ('user', 'feedback', 'project', 'reference')),
    agent TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    verified INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_memories_type ON memories(memory_type);
```

FEAT: FEAT-012

### sessions

```sql
CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    agents TEXT NOT NULL,          -- JSON array: ["claude-1", "gemini-1"]
    fleet_composition TEXT,        -- JSON: {"claude": 3, "codex": 2}
    working_directory TEXT,
    summary_json TEXT              -- Stenographer output
);
```

FEAT: FEAT-011, FEAT-012

### decisions

```sql
CREATE TABLE decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    decision_text TEXT NOT NULL,
    proposed_by TEXT NOT NULL,     -- agent id: "claude-1"
    validated_by TEXT,             -- agent id or "human"
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    evidence TEXT                  -- JSON: fabric message IDs
);
CREATE INDEX idx_decisions_session ON decisions(session_id);
```

FEAT: FEAT-013

### routing_log

```sql
CREATE TABLE routing_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    target_agent TEXT NOT NULL,
    message_type TEXT NOT NULL CHECK(message_type IN ('direct', 'summary', 'background', 'broadcast')),
    input_tokens INTEGER,
    output_tokens INTEGER,
    cost_usd REAL,
    latency_ms INTEGER,
    langfuse_trace_id TEXT,
    triggered_response INTEGER NOT NULL DEFAULT 0,
    task_id TEXT,
    fleet_id TEXT
);
CREATE INDEX idx_routing_timestamp ON routing_log(timestamp);
CREATE INDEX idx_routing_task ON routing_log(task_id);
```

FEAT: FEAT-017, FEAT-029, FEAT-030

---

## SQLite Schema — workflow.db

### workflows

```sql
CREATE TABLE workflows (
    workflow_id TEXT PRIMARY KEY,
    workflow_type TEXT NOT NULL CHECK(workflow_type IN ('conversation', 'debate', 'task', 'fleet')),
    state TEXT NOT NULL CHECK(state IN ('pending', 'running', 'paused', 'completed', 'failed', 'cancelled')),
    current_step INTEGER NOT NULL DEFAULT 0,
    payload TEXT NOT NULL,         -- JSON: workflow-specific data
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    session_id TEXT
);
```

FEAT: FEAT-007

### workflow_events (write-ahead log)

```sql
CREATE TABLE workflow_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id TEXT NOT NULL REFERENCES workflows(workflow_id),
    step INTEGER NOT NULL,
    event_type TEXT NOT NULL CHECK(event_type IN ('step_started', 'step_completed', 'step_failed', 'retry', 'compensation', 'human_gate', 'human_approved')),
    payload TEXT,                  -- JSON: step-specific data
    timestamp TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_events_workflow ON workflow_events(workflow_id);
```

FEAT: FEAT-007, FEAT-024

### fleet_tasks (shared task list)

```sql
CREATE TABLE fleet_tasks (
    task_id TEXT PRIMARY KEY,
    fleet_workflow_id TEXT NOT NULL REFERENCES workflows(workflow_id),
    title TEXT NOT NULL,
    description TEXT,
    state TEXT NOT NULL CHECK(state IN ('pending', 'claimed', 'in_progress', 'completed', 'blocked', 'failed')) DEFAULT 'pending',
    assigned_agent TEXT,           -- agent instance id
    depends_on TEXT,               -- JSON array of task_ids
    worktree_branch TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);
CREATE INDEX idx_fleet_tasks_workflow ON fleet_tasks(fleet_workflow_id);
CREATE INDEX idx_fleet_tasks_state ON fleet_tasks(state);
```

FEAT: FEAT-020

---

## REST API Endpoints

Base: `http://127.0.0.1:8080/api`

### Health

| Method | Path | Request | Response | FEAT |
|--------|------|---------|----------|------|
| GET | `/health` | — | `{"status": "ok", "version": "0.1.0", "agents": {...}}` | FEAT-023 |

### Agents

| Method | Path | Request | Response | FEAT |
|--------|------|---------|----------|------|
| GET | `/agents` | — | `[{"id": "claude-1", "type": "claude", "status": "ready", "quota_pct": 45}]` | FEAT-001 |
| POST | `/agents/spawn` | `{"type": "claude", "count": 2}` | `{"spawned": ["claude-2", "claude-3"]}` | FEAT-001 |
| DELETE | `/agents/{id}` | — | `{"shutdown": "claude-3"}` | FEAT-001 |

### Messages

| Method | Path | Request | Response | FEAT |
|--------|------|---------|----------|------|
| POST | `/message` | `{"content": "How should we handle auth?", "target": null}` | `{"routed_to": "claude-1", "message_id": "uuid"}` | FEAT-008 |
| POST | `/message` | `{"content": "implement auth", "target": "codex-1"}` | `{"routed_to": "codex-1", "message_id": "uuid"}` | FEAT-008 |

### Memory

| Method | Path | Request | Response | FEAT |
|--------|------|---------|----------|------|
| GET | `/memory` | `?type=feedback` | `[{"key": "...", "value": "...", "type": "feedback"}]` | FEAT-012 |
| POST | `/memory/confirm` | `{"decision_id": 7, "action": "approve"}` | `{"persisted": true}` | FEAT-013 |

### Fleet

| Method | Path | Request | Response | FEAT |
|--------|------|---------|----------|------|
| POST | `/fleet/spawn` | `{"task": "Build auth", "agents": {"claude": 3, "codex": 2}}` | `{"fleet_id": "uuid", "agents": [...], "worktrees": [...]}` | FEAT-010 |
| GET | `/fleet/{id}` | — | `{"status": "running", "tasks": [...], "agents": [...]}` | FEAT-010 |
| POST | `/fleet/{id}/merge` | `{"branch": "claude-2-auth-review"}` | `{"merged": true, "conflicts": []}` | FEAT-021 |

### Tasks

| Method | Path | Request | Response | FEAT |
|--------|------|---------|----------|------|
| GET | `/tasks` | `?fleet_id=uuid` | `[{"task_id": "...", "state": "in_progress", "agent": "codex-1"}]` | FEAT-020 |

### Sessions

| Method | Path | Request | Response | FEAT |
|--------|------|---------|----------|------|
| GET | `/sessions` | — | `[{"session_id": "...", "started_at": "...", "agents": [...]}]` | FEAT-011 |
| GET | `/sessions/{id}` | — | `{...full stenographer output...}` | FEAT-011 |

### Quota

| Method | Path | Request | Response | FEAT |
|--------|------|---------|----------|------|
| GET | `/quota` | — | `{"claude": {"used_pct": 45, "fallback_active": false}, ...}` | FEAT-017 |

---

## WebSocket Protocol

Endpoint: `ws://127.0.0.1:8080/ws`

### Client → Server

```json
{"type": "subscribe", "topics": ["agents.*", "tasks.*"]}
{"type": "message", "content": "How should we handle auth?", "target": null}
{"type": "interrupt", "agent": "claude-1"}
```

### Server → Client

```json
{"type": "agent_output", "agent": "claude-1", "task": "auth-design", "content": "...", "streaming": true, "is_final": false}
{"type": "agent_output", "agent": "claude-1", "task": "auth-design", "content": "", "streaming": false, "is_final": true}
{"type": "health_change", "agent": "gemini-1", "status": "ready", "detail": null}
{"type": "task_update", "task_id": "...", "state": "completed", "agent": "codex-1"}
{"type": "quota_update", "agent_type": "claude", "used_pct": 45, "fallback_active": false}
{"type": "decision_proposed", "decision_id": 7, "content": "Use JWT for auth", "source": "claude-1"}
{"type": "merge_request", "fleet_id": "...", "branch": "claude-2-auth-review", "conflicts": []}
{"type": "digest_sent", "to": "gemini-1", "summary": "Claude-1 proposed JWT auth. [3 tool calls]. Anything to add?"}
```

---

## Agent Protocol Contracts

### Claude (stream-json JSONL)

**Daemon → Claude stdin:**
```json
{"type": "user", "content": "Design the auth API"}
```

**Claude stdout → Daemon:**
```json
{"type": "init", "session_id": "uuid", "model": "claude-opus-4-6"}
{"type": "message", "role": "assistant", "content": "Here's my proposal...", "delta": true}
{"type": "tool_use", "tool": "write_file", "input": {"path": "src/auth.rs", "content": "..."}}
{"type": "result", "content": "File written successfully"}
{"type": "message", "role": "assistant", "content": "Done.", "delta": false}
```

### Gemini (ACP JSON-RPC)

**Daemon → Gemini stdin:**
```json
{"jsonrpc": "2.0", "method": "sendMessage", "params": {"content": "Research JWT best practices"}, "id": 1}
```

**Gemini stdout → Daemon:**
```json
{"jsonrpc": "2.0", "result": {"content": "Based on current standards...", "model": "gemini-3-pro"}, "id": 1}
```

### Codex (MCP JSON-RPC)

**Daemon → Codex stdin:**
```json
{"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "codex-reply", "arguments": {"message": "Implement auth.rs"}}, "id": 1}
```

**Codex stdout → Daemon:**
```json
{"jsonrpc": "2.0", "result": {"content": [{"type": "text", "text": "Implementing..."}]}, "id": 1}
```

---

## Config Schema

```toml
# ~/.triumvirate/config.toml

web_port = 8080
# db_path = "~/.triumvirate/memory.db"  # default
# workflow_db_path = "~/.triumvirate/workflow.db"  # default

[agents.claude]
enabled = true
backend = "cli"         # "cli" or "api"
instances = 1           # default pool size
# api_key = "sk-..."   # only for api backend

[agents.gemini]
enabled = true
backend = "cli"
instances = 1

[agents.codex]
enabled = true
backend = "cli"
instances = 1

[quota]
fallback_threshold_pct = 80   # auto-disable digests at this %
```
