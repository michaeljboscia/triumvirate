# Session Notes - Full Narrative

**Slash Command:** `/session-notes-full`

**Alias for:** `/session-notes` — both commands do the same thing.

---

## Execution Steps

Follow the exact steps in `/session-notes`:

1. Find the latest session log via `python3 ~/.triumvirate/stenographer/session_log_path.py --recover "$(pwd)" --agent claude`
2. Append a full `## SESSION UPDATE:` section with Summary, Accomplished, Key Decisions, What Works, Issues, Current State, Sub-agent Work, Technical Notes
3. Add one-liner to activity log table
4. Confirm with filename and one-line summary
