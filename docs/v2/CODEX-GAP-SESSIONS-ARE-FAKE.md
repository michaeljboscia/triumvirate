# Critical Gap: Sessions Don't Spawn Real Agents

**Date:** 2026-04-05
**Found by:** Live testing — Claude called spawn_session + ask_session expecting real agent interaction
**Severity:** BLOCKER — semantic mismatch between tool name and behavior

---

## The Problem

`spawn_session` and `ask_session` create an in-memory HashMap entry that stores conversation strings. They do NOT spawn a real Gemini or Codex CLI process. They do NOT talk to any agent.

When Claude calls `spawn_session("gemini", "my-research")` followed by `ask_session("my-research", "what time is it?")`, Claude expects Gemini to answer. Instead, `ask_session` returns whatever is in the HashMap — which is nothing, because no agent was ever invoked.

This is a semantic failure: the tool NAMES promise agent interaction. The IMPLEMENTATION is a local string buffer.

---

## What the Spec Says (SPEC_FINAL.md, US-8)

```
GIVEN I'm working on a multi-hour task
WHEN I spawn a persistent session
THEN agents stay ALIVE indefinitely
AND I can ask follow-up questions that build on prior context
```

"Agents stay ALIVE" means a real CLI process is running. Not a HashMap.

---

## What Needs to Change

### Option A: Sessions invoke real agents

`spawn_session("gemini", "my-research")` should:
1. Spawn a real Gemini CLI process (or reuse an alive one)
2. Keep it alive for the session duration
3. `ask_session("my-research", "question")` pipes the question to that process and returns the real response
4. `dismiss_session("my-research")` kills the process

This is what the TS inter-agent `spawn_daemon` / `ask_daemon` do today. The daemon pattern is the whole point.

### Option B: Rename the tools to not lie

If sessions are just conversation buffers for context tracking, rename them:
- `spawn_session` → `create_context` or `start_thread`
- `ask_session` → `ask_agent_with_context` (which calls `run_named_agent` internally with accumulated context)

This is less ideal because it still spawns a fresh process per question, but at least the names don't promise something they don't deliver.

### Option C (Recommended): Hybrid

`spawn_session` creates a session record AND spawns a real agent process. The process stays alive. `ask_session` pipes to the alive process. Context accumulates both in the process (agent memory) and in the session record (for recovery).

This matches how the TS `spawn_daemon` works and is what the user expects.

---

## The Real Agent Invocation Problem

This connects to the broader issue: the daemon currently spawns a FRESH CLI process for every `ask_agent` / `ask_twins` call. Each request pays the full MCP ceremony cost (15-30s for Gemini).

The spec says agents should be pre-warmed and stay alive. `spawn_session` is where that should happen — spawn once, keep alive, reuse on every `ask_session`.

The fix for sessions and the fix for pre-warming are the SAME fix: persistent agent processes managed by the daemon.

---

## Current State of ask_session

```rust
// What it does now: reads from a HashMap
async fn ask_session(&self, Parameters(req): Parameters<AskSessionRequest>) -> Result<String, String> {
    let sessions = self.sessions.lock().await;
    // ... looks up session by name
    // ... appends message to history vector
    // ... returns the history as a string
    // NEVER calls run_named_agent or any real CLI
}
```

## What It Should Do

```rust
async fn ask_session(&self, Parameters(req): Parameters<AskSessionRequest>) -> Result<String, String> {
    let sessions = self.sessions.lock().await;
    let session = sessions.get(&req.name).ok_or("session not found")?;
    
    // Send to the ALIVE agent process
    let response = session.agent_process.send(&req.message).await?;
    
    // Record in history for recovery
    session.history.push(req.message, response.clone());
    
    Ok(response)
}
```

---

## Acceptance Criteria

- [ ] `spawn_session("gemini", "name")` spawns a real Gemini CLI process (or reuses alive one)
- [ ] `ask_session("name", "question")` sends to the real process and returns real response
- [ ] Subsequent `ask_session` calls go to the SAME process (no re-spawn)
- [ ] `dismiss_session("name")` terminates the process
- [ ] Session persists across MCP bridge restarts (process stays alive in daemon)
- [ ] Lifecycle events emitted for session operations

---

## Also Fix: Codex --skip-git-repo-check

From live testing: Codex fails with "Not inside a trusted directory" when cwd is ~ or a non-git directory. The Codex connector needs to pass `--skip-git-repo-check` or validate that cwd is a git repo before invoking.

Current error from ask_twins:
```
"Codex failed: codex connector failed: Not inside a trusted directory and --skip-git-repo-check was not specified."
```

---

## Status Update (2026-04-06)

### Done
- Added daemon-managed persistent worker registry keyed by `(agent, cwd)`.
- `ask_agent`, `ask_twins`, `spawn_session`, `ask_session`, and `dismiss_session` now route through the same persistent backend.
- Added daemon HTTP session routes:
  - `POST /session/spawn`
  - `POST /session/ask`
  - `POST /session/dismiss`
  - `GET /session/list`
- MCP session tools proxy through daemon when daemon mode is enabled, matching `ask_agent`/`ask_twins` behavior.
- `ask_twins` defaults to raw prompt passthrough to both agents; role adaptation is opt-in via `TRIUMVIRATE_ASK_TWINS_ROLE_ADAPT`.
- Added regression tests for:
  - persistent worker reuse and faster second call,
  - shared backend between session tools and `ask_twins`,
  - passthrough prompt semantics.

### Honest Semantics
- Current implementation is **persistent session-backed orchestration**, not a single forever-running stdin process per agent.
- Persistence uses connector-native resumable session/thread IDs and daemon-managed worker records.
