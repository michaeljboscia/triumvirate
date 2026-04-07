# SPEC ADDENDUM — Lessons from Live Testing

**Date:** 2026-04-05
**Context:** First live test of triumvirate v2 daemon
**Status:** MANDATORY — these clarify intent that was ambiguous in SPEC_FINAL.md

**2026-04-06 Update:** This addendum captures historical `ask_twins` semantics. The tool has now been removed; keep the session-backed lifecycle requirements, but execute fan-out through explicit sessions.

---

## Clarification 1: ALL agent interaction is session-backed

The spec says "agents stay alive." The implementation spawns fresh processes per request.

**The rule:** There is NO code path that spawns a disposable agent process. Every agent interaction goes through a persistent session.

```
ask_twins    → reuses alive Gemini + Codex sessions (spawns on first use)
ask_agent    → reuses alive session for that agent (spawns on first use)
ask_session  → reuses the named session (must be spawned first)
```

"Spawn" means a real CLI process starts. "Reuse" means the SAME process handles the next request. "Alive" means the process is running between requests, with full MCP context loaded, ready to respond instantly.

The cold start (MCP ceremony, context refresh) happens ONCE per agent per machine boot. Every subsequent request is fast.

---

## Clarification 2: ask_twins = spawn pair + send + keep alive

`ask_twins` is NOT fire-and-forget. It is shorthand for:

1. Check if a default Gemini session is alive → if not, spawn one
2. Check if a default Codex session is alive → if not, spawn one  
3. Send role-adapted prompt to both
4. Return results as they arrive (non-blocking)
5. Sessions STAY ALIVE for the next ask_twins call

The user says "ask the twins" 10 times in a session. The first call takes 15-30s (cold start). Calls 2-10 take 3-5s (agent already warm).

---

## Clarification 3: spawn_session / ask_session invoke REAL agents

`spawn_session("gemini", "my-research")` spawns a real Gemini CLI process. Not a HashMap entry. Not a conversation buffer.

`ask_session("my-research", "question")` sends the question to the RUNNING Gemini process and returns its real response.

The session record in SQLite tracks: session name, agent type, PID, state (alive/dead), conversation history (for recovery). But the PRIMARY state is the running process, not the database.

---

## Clarification 4: Mock CLIs are for tests. Real CLIs are for production.

The daemon MUST NOT use mock connectors outside of `#[cfg(test)]`. In production mode, if `TRIUMVIRATE_GEMINI_BIN` is not set, the daemon should fail loudly: "Gemini CLI not configured. Set TRIUMVIRATE_GEMINI_BIN."

Every `run_agent_process` call in production hits the real CLI with the real invocation pattern:
- Gemini: `gemini -p "prompt"` (argument, not stdin)
- Codex: `codex exec --json --skip-git-repo-check "prompt"` (needs skip-git-repo-check for non-repo cwds)

**Tests against mocks prove the plumbing works. They do NOT prove the product works.** Both must pass before shipping.

---

## Clarification 5: Codex needs --skip-git-repo-check

Codex refuses to run outside a trusted git directory. The daemon's Codex connector must pass `--skip-git-repo-check` to avoid failures when cwd is ~ or a non-git path.

Alternatively, the daemon can validate that cwd is a git repo before invoking Codex and return a clear error if not: "Codex requires a git repository as working directory. Current cwd /Users/mikeboscia is not a git repo."

---

## Clarification 6: Pre-warming is not optional

The spec says "JIT spawn on first ask, then stay alive." This means:

- First `ask_twins` → spawns Gemini + Codex → 15-30s cold start → responds
- Second `ask_twins` → reuses alive sessions → 3-5s → responds
- Tenth `ask_twins` → same sessions → 3-5s → responds

The 15-30s cold start is the Gemini MCP ceremony (loading github, supabase, hubspot notification handlers, context refresh). This CANNOT be eliminated, only front-loaded.

If the user experiences the 15-30s ceremony on every request, the system is broken. It should happen once.

---

## Integration with SPEC_FINAL.md

These clarifications do not change any goatrodeo decisions. They make explicit what the spec MEANT but didn't say precisely enough for implementation.

The 10 build increments should be re-read with these clarifications in mind:
- Increment 4 (Sessions) and Increment 5 (Alive Sessions) are the core of this — they must implement REAL persistent processes, not in-memory buffers
- Increment 2 (ask_twins) must be session-backed, not fire-and-forget
- All increments must test against REAL CLIs, not just mocks

---

## Files for Codex

| File | What |
|------|------|
| This file | Spec clarifications from live testing |
| `CODEX-GAP-SESSIONS-ARE-FAKE.md` | Detailed gap: sessions are HashMap, not real agents |
| `CODEX-GAP-PROGRESS-NOTIFICATIONS.md` | Progress streaming gap (may be fixed already) |
| `SPEC_FINAL.md` | The spec (read with this addendum) |
| `TEST_PLAN_V2.md` | Tests (add real-CLI integration tests) |
