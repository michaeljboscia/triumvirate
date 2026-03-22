# Phase 1: Capture

## Purpose
Gather failure data, search for related artifacts, and confirm the failure class boundary with the user.

## Skip Condition
If entering from our-systematic-debugging Phase 5, failure data already exists. Skip to Phase 2.

## Process

### Step 1: User States Scope
The user describes the problem. This will be broad and emotional: "the Tellus batch processing disaster" or "the cold email problem." Accept the scope as stated.

### Step 2: Search Existing Artifacts
Search ALL of these locations for related material:

- `~/.claude/lessons/` — lesson files
- `~/.claude/projects/*/memory/` — memory entries (iron laws, feedback, project notes)
- Session logs in the relevant project directory
- Git history: `git log --oneline --all --grep="<keywords>"`

Look for: lessons, iron laws (feedback-type memories), related project memories, session log entries that mention the same domain or error patterns.

### Step 3: Present Findings
Show the user what you found:

"I found these related artifacts:
- [iron law] Batch output verification — created [date]
- [memory] Tellus road KNN scaling — created [date]
- [lesson] (if any)

Are these all part of the same failure class, or are some adjacent/unrelated?"

### Step 4: User Confirms Boundary
User tells you what's in scope. This defines the failure class.

### Step 5: Structured Failure Log
For each failure within the confirmed boundary, capture:

```
Failure: [what happened]
Attempted approach: [what was tried]
Why it failed: [root cause or symptom]
Assumption that was wrong: [the mental model error]
Hours/resources wasted: [quantify the cost]
```

## Adapting to Intake Shape

**Acute (hours, one session):**
- Read the session log from that day
- Artifacts are concentrated — straightforward search

**Chronic (weeks/months, many sessions):**
- Dispatch Gemini daemon with 2M context
- Load: all related memory entries, iron laws, and session logs across the timeframe
- Ask Gemini: "Find the pattern across these failures"
- Present Gemini's pattern analysis to user for confirmation

**Prophylactic (no failure yet):**
- No failure data to gather
- Ask user: "What research exists? What do you expect to go wrong?"
- Reference layer will be built from research in Phase 4
- Anti-rationalization will be sparse initially — expect iterative refinement

## Output
A structured failure log with confirmed boundary. Pass to Phase 2.
