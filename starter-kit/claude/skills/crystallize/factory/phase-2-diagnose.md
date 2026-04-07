# Phase 2: Diagnose

## Purpose
Find the root cause at the MENTAL MODEL level, not the implementation level. Use the triumvirate.

## Triumvirate Dispatch — MANDATORY

All three agents diagnose independently. This is not optional.

### Prepare the Payload
Compile from Phase 1 output:
- The structured failure log (all failures in the class)
- The confirmed boundary
- Any related artifacts (iron laws, lessons, memory entries)

### Dispatch to All Three

**Claude (self):** Analyze the failure log. Answer:
1. Why did all approaches fail?
2. What mental model was wrong?
3. What would NEVER have worked and why?

**Gemini daemon:** Spawn or resume daemon. Send the same payload and questions.
```
mcp__triumvirate__spawn_session(cwd: "<project-dir>")
mcp__triumvirate__ask_session(session_name, "<payload + questions>")
```

**Codex daemon:** Spawn daemon. Send the same payload and questions.
```
mcp__triumvirate__spawn_session(cwd: "<project-dir>")
mcp__triumvirate__ask_session(session_name, "<payload + questions>")
```

### If Triumvirate Already Happened
If the diagnosis was already done during normal debugging (before crystallization was invoked), capture that output instead of re-dispatching. Don't duplicate work.

### Convergence
- If 2+ agents agree on root cause → that's the diagnosis
- If all 3 disagree → present all three analyses to user, user decides
- Document the convergent root cause

## Output
Root cause statement at the mental model level. Pass to Phase 3.

## Anti-Rationalization

**You will be tempted to:** Skip the triumvirate and just diagnose yourself. "I already understand the problem."
**Why that fails:** One agent's diagnosis is one perspective. The root cause you identify might be a symptom of a deeper issue that another agent would catch. The triumvirate exists because single-agent diagnosis has failed before.
