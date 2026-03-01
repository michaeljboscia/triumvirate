---
name: save-session
description: Generate a cross-agent compatible markdown session summary from the current Codex transcript, using standardized taxonomy/filename/versioning and optional git commit.
---

# Save Session Skill

## Purpose
Generate a cross-agent compatible session summary from Codex transcripts so Codex, Claude, and Gemini logs are unified and searchable.

## Trigger
Use this skill when the user says:
- `/save-session`
- `save session`
- `save session log`
- `write a session handoff`

## Workflow
1. Determine the transcript source.
- Default: latest file under `~/.codex/sessions/YYYY/MM/DD/*.jsonl`.
- Optional override via `--transcript`.

2. Resolve taxonomy (in order).
- `<project>/.codex/taxonomy.json`
- `<project>/.claude/taxonomy.json`
- git remote parsing (`owner/repo`)
- directory/default fallback (`unknown`)

3. Determine output location.
- If project context exists (`.git` or taxonomy file): `<project>/session-logs/`
- Otherwise: `~/.codex/session-logs/`

4. Compute version.
- Filename format:
`<owner>--<client>_<domain>_<repo>_<feature>_<YYYYMMDD>_v<N>_codex.md`
- Increment `v<N>` for same day + feature + agent.

5. Generate markdown with required sections.
- TAXONOMY
- TRANSCRIPT HISTORY
- CONTEXT SUMMARY
- SESSION ACTIVITY LOG
- INSTRUCTIONS FOR NEXT SESSION

6. Optionally commit in project repo.
- Commit message format:
`session(codex): <feature> v<N> - session summary`

## Script
Use:
`~/.codex/skills/save-session/scripts/save_session.sh`

Examples:
```bash
# Standard manual save (current directory)
~/.codex/skills/save-session/scripts/save_session.sh --trigger manual

# End-of-session save without committing
~/.codex/skills/save-session/scripts/save_session.sh --trigger session-end --no-commit

# Explicit transcript and feature override
~/.codex/skills/save-session/scripts/save_session.sh \
  --project-root /path/to/project \
  --transcript ~/.codex/sessions/2026/02/12/rollout-...jsonl \
  --feature narrative_generator
```

## Notes
- Keep sensitive values redacted in summaries.
- Use `TZ='America/New_York'` timestamps for output consistency.
- Force plain style for session logs (no persona overlays, no themed voice).
- If markdown quality needs richer narrative, manually edit the generated log before commit.
