# Lifecycle Hooks Reference

Triumvirate ships 17 hooks across all three agents. These are installed by `starter-kit/install.sh` and work out of the box.

For configuration variables, see [configuration-reference.md](../configuration-reference.md).

---

## Claude Code Hooks

| Hook | Event | What It Does |
|------|-------|-------------|
| `session-start.sh` | SessionStart | **Project Picker** (from HOME) or **Session Recovery** (from project dir) |
| `post-compact-recovery.sh` | SessionStart:compact | Restores context from session log after compaction |
| `pre-compact.sh` | PreCompact | Gemini summarizes transcript → session log → git commit |
| `post-tool-use.sh` | PostToolUse | Auto-stages files, logs activity to session log |
| `post-tool-use-token-gate.sh` | PostToolUse | Auto-saves at ~50K token intervals via **Stenographer** (local Ollama) |
| `pre-tool-use-artifact-guard.sh` | PreToolUse | **The Airlock** — snapshots every file before edit |
| `pre-tool-use-bash-guard.sh` | PreToolUse | Blocks destructive SQL without a fresh backup |
| `pre-tool-use-supabase-mcp-gate.sh` | PreToolUse | Blocks Supabase MCP SQL without fresh backup |
| `post-tool-use-oracle-pressure.sh` | PostToolUse | Oracle context pressure monitoring — recommends checkpoints |
| `post-tool-use-mode-nudge.sh` | PostToolUse | Suggests formalizing execution mode after 15+ tool calls |
| `session-start-v3.sh` | SessionStart | Orphan recovery — cleans stale locks and saves |
| `_find-session-log.sh` | (shared) | Helper that finds the latest session log across locations |

## Gemini CLI Hooks

| Hook | Event | What It Does |
|------|-------|-------------|
| `session-start.sh` | SessionStart | Session log recovery |
| `pre-compact.sh` | PreCompact | Self-summarization → session log → git commit |
| `post-tool-use.sh` | PostToolUse | Auto-stages files, logs activity |

## Codex CLI Hooks

| Hook | Event | What It Does |
|------|-------|-------------|
| `session-start.sh` | SessionStart | Session log recovery |
| `pre-compact.sh` | PreCompact | Auto-save before context compaction |

---

## How the Lifecycle Works

```
SessionStart
  └─► session-start.sh
        ├─ HOME? → Project Picker (show list, user picks, cd there)
        └─ Project? → Read latest session log → inject context

Every Tool Call:
  └─► PreToolUse: The Airlock (artifact-guard)
        ├─ Supabase SQL → Check backup freshness → ALLOW or DENY
        ├─ Edge functions, n8n workflows → Snapshot, always allow
        ├─ Source files, markdown → Snapshot, always allow
        └─ node_modules, .git, /tmp → No action (pass)
  └─► PreToolUse: bash-guard
        └─ DELETE/TRUNCATE/DROP/pg_restore → Check for fresh backup → ALLOW or DENY
  └─► [Tool executes]
  └─► PostToolUse: auto-stage + activity log
  └─► PostToolUse: token-gate
        └─ Transcript growth > threshold? → Stenographer (local Ollama)

Before Memory Loss:
  └─► PreCompact: pre-compact.sh
        └─ Extract transcript → Gemini summarization → session log → git commit
  └─► SessionStart:compact: post-compact-recovery.sh
        └─ Read the summary back into context
```

---

## Extending Hooks

The hooks are extensible. Wire PM tools (Linear, Jira, GitHub Projects) into the lifecycle using the same pattern:

1. Source the shared library (`_find-session-log.sh`)
2. Run in a background subshell `( ... ) &`
3. Gate with an environment variable

See [starter-kit/README.md](../../starter-kit/README.md) for the full hook source code and configuration variables.
