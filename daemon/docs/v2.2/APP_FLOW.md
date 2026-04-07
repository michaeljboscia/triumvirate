# Application Flow — Triumvirate v2.2

---

## Flow 1: Event Ingestion (Ledger)

```
User works in Claude Code
         ↓
PostToolUse hook fires
         ↓
Hook: resolve project root (git rev-parse --show-toplevel)
         ↓
Hook: write event JSON → <project>/.triumvirate/spool/event-<ts>-<pid>-<rand>.tmp
         ↓
Hook: atomic rename .tmp → .ndjson (<1ms total)
         ↓
Hook: curl -s POST http://localhost:8080/ledger/wake {"project_root":"..."} &
         ↓ (non-blocking, hook returns immediately)
Daemon: receives /ledger/wake
         ↓
Daemon: adds project_root to LRU cache
         ↓
Daemon: reads spool dir, sorts by creation time
         ↓
Daemon: for each .ndjson file:
    - parse JSON
    - truncate fields >64KB
    - INSERT INTO events (idempotency check)
    - delete spool file
         ↓
Daemon: compression worker picks up pending events
         ↓
Tier 0: extractive summary → INSERT INTO summaries
         ↓
[Tier 1 if triggered]: LLM summary → INSERT INTO summaries
         ↓
Dashboard: WebSocket pushes event to connected browsers
```

**Failure paths:**
- Daemon down → spool files accumulate → daemon drains on restart
- Spool >100MB → health reports `degraded`, writes still accepted
- Compression worker crash → heartbeat expires (90s) → job resets to pending

---

## Flow 2: Fleet Execution

```
User: "spawn a fleet for auth module"
         ↓
Claude calls fleet_spawn(task="auth module", agents={claude:1, codex:1})
         ↓ (dry_run=true by default)
Daemon: returns plan summary
         ↓
Claude: "Here's the plan: 2 agents, 2 worktrees from commit abc123. Wait or background?"
         ↓
User: "background"
         ↓
Claude calls fleet_spawn(task=..., agents=..., dry_run=false, wait=false)
         ↓
Daemon: verify git is clean (REQ-030)
         ↓
Daemon: add .triumvirate/ to .gitignore if needed (REQ-018a)
         ↓
Daemon: create worktree A → git worktree add /tmp/fleet-abc/claude-1
Daemon: create worktree B → git worktree add /tmp/fleet-abc/codex-1
         ↓
Daemon: write .triumvirate/fleet-task.md to each worktree
         ↓
Daemon: set TRIUMVIRATE_PROJECT_ROOT=<source project> for each agent
         ↓
Daemon: spawn claude-1 subprocess in worktree A with prompt
Daemon: spawn codex-1 subprocess in worktree B with prompt
         ↓
Agent claude-1: reads fleet-task.md → claims T-001 → works → completes
Agent codex-1: reads fleet-task.md → claims T-002 → works → completes
         ↓ (on each completion)
Daemon: request peer review immediately (parallel with other agents)
    codex-1 done → review requested from claude
    claude-1 done → review requested from codex
         ↓
Daemon: reviews complete (bounded queue, max 2 inflight)
         ↓
Daemon: merge phase (sequential, completion order)
    merge codex-1 → check review=approve → git merge → success
    merge claude-1 → check review=approve → git merge → success
         ↓
Daemon: clean up worktrees
Daemon: fleet state → done
Daemon: emit fleet_done event to Ledger
```

**Failure paths:**
- Dirty git → fleet_spawn fails with actionable error
- Merge conflict → queue pauses, user notified via MCP
- Review rejected → merge pauses, reviewer comments surfaced
- Daemon crash mid-fleet → startup reconciliation (REQ-034a) marks fleet failed, resets tasks

---

## Flow 3: Peer Review

```
Fleet agent completes task
         ↓
Daemon: review_request(artifact=diff, author=codex)
         ↓
peer-review crate: check queue (max 2 inflight)
         ↓ (queued if at cap)
peer-review crate: round-robin → assign to claude (non-author)
         ↓
Daemon: spawn claude reviewer via ask_agent
         ↓
Claude reviewer: reads diff, reasons, calls review_submit(verdict=approve)
         ↓
peer-review crate: store result in reviews table
         ↓
Fleet merge phase: checks reviews[worktree].verdict == approve → proceed
```

**Failure paths:**
- Review timeout (120s) → review marked `failed`, merge queue surfaces to user
- Self-review attempt → rejected with error naming required reviewer
- All non-author agents busy → FIFO queue, waits for slot

---

## Flow 4: Lesson Lifecycle

```
Compression worker produces summary with type=bug_fix
         ↓
Worker: auto-create lesson (confidence=0.6)
         ↓
         ... 50 days pass ...
         ↓
User asks Claude about SQLite locking
         ↓
Claude: lesson_query("SQLite locking")
         ↓
ledger crate: SELECT + compute confidence = 0.6 * e^(-0.01 * 50) = 0.36
         ↓
Claude: "Found lesson #14, confidence 0.36 — getting stale."
         ↓
User: "that's still correct"
         ↓
Claude: lesson_validate(14)
         ↓
ledger crate: UPDATE last_validated_at = now → confidence resets to 0.8
```

---

## Flow 5: Dashboard

```
User opens browser → localhost:8080
         ↓
Daemon: serves index.html from rust-embed
         ↓
SPA loads → fetches /health, /status, /ledger/health
         ↓
SPA: connects WebSocket to /ws
         ↓
Daemon pushes events:
    agent_state (working/idle/stuck per agent)
    fleet_progress (task claimed/completed/merged)
    ledger_health (healthy/degraded/dead)
    review_completed (verdict + reviewer)
         ↓
Svelte components update reactively
         ↓
User clicks Fleet tab → kanban board renders from fleet_task_list()
User clicks Lessons tab → confidence bars from lesson_list()
User clicks search → ledger_query() results
```

---

## Flow 6: Daemon Startup

```
triumvirate daemon
         ↓
Load bearer token from ~/.triumvirate/daemon.token
         ↓
Bind HTTP server to 127.0.0.1:8080
         ↓
Fleet crash recovery (REQ-034a):
    scan fleets in spawning|running|merging
    for each: check agent PIDs alive
    if dead: mark failed, reset tasks, clean worktrees, emit events
         ↓
Ledger GC (REQ-058):
    if last GC >24h ago AND no active fleets → run GC
         ↓
Spool drain (REQ-011a):
    for each known project in LRU: drain spool dir → SQLite
         ↓
Start WebSocket listener
         ↓
Start Prometheus /metrics endpoint
         ↓
Ready for MCP connections
```
