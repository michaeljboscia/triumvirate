# Session Notes - Full Narrative

**Slash Command:** `/session-notes`

**Purpose:** Capture comprehensive session narrative immediately. No menus, no options - just write the full update.

---

## When to Use

Use `/session-notes` when you need to manually capture what happened:
- After completing significant work
- Before ending a session
- After sub-agents complete
- Whenever you want to verify the session log is current

Note: Stenographer writes session notes automatically every ~50K tokens. This command is for when you want to force a write or capture something specific.

---

## Execution Steps

**When invoked, immediately do this:**

### Step 1: Find Session Log

Use the single source of truth path resolver:

```bash
# Find session log for current project (uses session_log_path.py)
python3 ~/.triumvirate/stenographer/session_log_path.py --recover "$(pwd)" --agent claude
```

If that returns empty, find the transcript and create/get the log:

```bash
# Find current transcript
TRANSCRIPT=$(ls -t ~/.claude/projects/$(echo "$HOME" | sed 's|/|-|g')/*.jsonl 2>/dev/null | head -1)
# Get or create the session log
python3 ~/.triumvirate/stenographer/session_log_path.py --get "$(pwd)" --transcript "$TRANSCRIPT" --agent claude
```

The session log path resolver handles HOME vs project routing, AI_MEMORY_DIR, and taxonomy automatically.

### Step 2: Write Full Narrative Update

Append this section to the session log:

```markdown
---

## SESSION UPDATE: [YYYY-MM-DD HH:MM TZ]

### Summary
[1-2 sentence overview of what happened since last update]

### What Was Accomplished
- [Bullet point 1 - be specific, include file names and commit hashes]
- [Bullet point 2]
- [Bullet point 3]

### Key Decisions & Why
- [Decision 1]: [Why we made it]
- [Decision 2]: [Why we made it]

### What Works Now
- [Thing that's verified working]

### What Doesn't Work / Known Issues
- [Issue or gap identified]

### Current State
**Phase:** [Current phase/milestone]
**Next Step:** [Immediate next action]

### Sub-agent Work (if any)
[If sub-agents ran, summarize: count, what they found, key insights]

### Technical Notes
[Gotchas, warnings, or context for future sessions]
```

### Step 3: Update Activity Log

Also add a one-liner to the activity log table:

```markdown
| [HH:MM] | Full narrative update | ✓ |
```

### Step 4: Confirm

```
Session log updated with full narrative.

File: [filename]
Summary: [one-line summary of what was captured]
```

---

## Important

- **Be specific** - Include file names, commit hashes, actual findings
- **Capture decisions** - Not just what, but WHY
- **Note blockers** - If something didn't work, say so
- **Include sub-agent work** - Don't lose visibility into what agents did
- **Keep it scannable** - Future sessions will skim this
