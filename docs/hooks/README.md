# Hooks (Tier 2 — coming soon)

This directory will contain reference implementations of the lifecycle hooks that give Triumvirate full session persistence across context compaction.

## What this is

Claude Code, Gemini CLI, and Codex each support lifecycle hooks — shell scripts that fire on session start, before compaction, and after tool use. Wired together, they create a memory system:

```
Claude session approaching context limit
  → pre-compact.sh fires
  → Extracts conversation as structured event log
  → Sends to Gemini CLI for summarization
  → Gemini writes SESSION_LOG_SPEC-compatible log to session-logs/
  → Git commits the log
  → Compaction happens
  → post-compact-recovery.sh fires
  → Reads the Gemini summary back into context
  → Claude continues with full context restored
```

Same pattern for Codex and Gemini — each agent writes its own log on pre-compact.

## Status

In progress. The hooks are running in production on the system that built Triumvirate — they need to be extracted, generalized, and documented before they're ready to publish here.

Tracked in the main Roadmap.

## What's coming

- `claude/pre-compact.sh` — Gemini-powered transcript summarization
- `claude/post-compact-recovery.sh` — context restoration from session log
- `claude/post-tool-use.sh` — auto-staging and token gate monitoring
- `claude/session-start.sh` — reads latest session log on startup
- `codex/pre-compact.sh` — Codex session summarization
- `gemini/pre-compact.sh` — Gemini session summarization
- Setup guide — how to register hooks in each CLI's config
