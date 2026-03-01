# Triumvirate Starter Kit

One-command setup for the complete multi-agent operating environment.

## What You Get

### Claude Code Hooks (8 files)
| Hook | Event | Purpose |
|------|-------|---------|
| `session-start.sh` | SessionStart | **Project Picker** when in HOME, **Session Recovery** when in a project |
| `post-compact-recovery.sh` | SessionStart:compact | Restores context after memory compaction |
| `pre-compact.sh` | PreCompact | Auto-saves session state before compaction (Gemini summarization → git commit) |
| `post-tool-use.sh` | PostToolUse | Auto-stages files, logs activity, taxonomy enforcement |
| `post-tool-use-token-gate.sh` | PostToolUse | Auto-saves at configurable token thresholds (~50K tokens) |
| `pre-tool-use-artifact-guard.sh` | PreToolUse | **The Airlock** — Snapshots every file before edit; enforces backup requirements for Supabase SQL |
| `pre-tool-use-bash-guard.sh` | PreToolUse | Blocks destructive SQL (DELETE/TRUNCATE/DROP) without a fresh backup |
| `_find-session-log.sh` | (shared) | Helper that finds the latest session log across multiple locations |

### Codex CLI Configuration
- `config.toml` — Model, approval mode, hooks, MCP server wiring
- `hooks/session-start.sh` — Session log recovery for Codex
- `hooks/pre-compact.sh` — Auto-save before Codex context compaction
- `skills/send-to-claude/` — Send inter-agent messages to Claude
- `skills/save-session/` — Save Codex session logs in shared format

### Gemini CLI Configuration
- `GEMINI.md` — Starter instructions for Gemini as a Triumvirate member

### Shared Templates
- `.env.example` — Credential vault template (API keys)
- `taxonomy.json.example` — Project taxonomy template

## Quick Start

```bash
# 1. Clone the repo
git clone https://github.com/michaeljboscia/triumvirate
cd triumvirate/starter-kit

# 2. Run the installer
chmod +x install.sh
./install.sh

# 3. Set up your credentials
cp ~/.claude/.env.example ~/.claude/.env
# Edit ~/.claude/.env with your API keys

# 4. Uncomment .env sourcing in session-start.sh
# Edit ~/.claude/hooks/session-start.sh — uncomment the .env block

# 5. Create your first project
mkdir -p ~/projects/my-project/.claude
cp ~/.claude/taxonomy.json.example ~/projects/my-project/.claude/taxonomy.json
# Edit taxonomy.json with your project details

# 6. Start working
cd ~/projects/my-project
claude
```

## Prerequisites

- **jq** — Required by all hooks (`brew install jq` on macOS)
- **Claude Code** — The CLI itself ([claude.ai/claude-code](https://claude.ai/claude-code))
- **Gemini CLI** (optional) — For intelligent pre-compact summarization. Without it, hooks fall back to jq-based transcript extraction.
- **Codex CLI** (optional) — For multi-agent code generation. Requires the [hooks-enabled fork](https://github.com/michaeljboscia/codex).
- **git** — Used by hooks for auto-staging and session log commits

## How the Hooks Work

```
SessionStart
  └─► session-start.sh
        ├─ HOME? → Project Picker (show list, user picks, cd there)
        └─ Project? → Read latest session log → inject context

Every Tool Call:
  └─► PreToolUse: The Airlock (pre-tool-use-artifact-guard.sh)
        ├─ Supabase SQL → Check backup freshness → ALLOW or DENY
        ├─ Edge functions, n8n workflows → Snapshot, always allow
        ├─ Source files, markdown → Snapshot, always allow
        └─ node_modules, .git, /tmp → No action (pass)
  └─► PreToolUse: bash-guard (pre-tool-use-bash-guard.sh)
        └─ DELETE/TRUNCATE/DROP/pg_restore → Check for fresh backup → ALLOW or DENY
  └─► [Tool executes]
  └─► PostToolUse: post-tool-use.sh
        └─ git add (staged = experimental) → on test pass: git commit (blessed)
  └─► PostToolUse: token-gate.sh
        └─ Transcript growth > threshold? → Background save via pre-compact.sh

Before Memory Loss:
  └─► PreCompact: pre-compact.sh
        └─ Extract transcript → Gemini Pro summarization → session log → git commit
  └─► SessionStart:compact: post-compact-recovery.sh
        └─ Read the summary back into context
```

## Configuration

### Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `GEMINI_CLI_PATH` | Path to Gemini CLI | `gemini` |
| `AI_MEMORY_DIR` | Central session log store | `~/.ai-memory` |
| `ENABLE_CLICKUP` | ClickUp task integration | `0` (off) |
| `SUPABASE_BACKUP_TTL_MINS` | Backup freshness window | `30` min |
| `TOKEN_GATE_THRESHOLD_KB` | Auto-save threshold | `200` KB (~50K tokens) |
| `TOKEN_GATE_COOLDOWN_SECS` | Min time between saves | `300` sec |
| `TOKEN_GATE_DISABLE` | Disable token gate | `0` |
| `ARTIFACT_GUARD_BYPASS` | Emergency bypass for Airlock | `0` |
| `BASH_GUARD_BYPASS` | Emergency bypass for bash guard | `0` |

### Taxonomy

Each project needs a `.claude/taxonomy.json`:

```json
{
  "owner": "your-username",
  "client": "personal",
  "domain": "infrastructure",
  "repo": "my-project",
  "feature": "main"
}
```

This drives session log naming: `owner--client_domain_repo_feature_YYYYMMDD_vN_agent.md`

All three agents (Claude, Codex, Gemini) read/write session logs in this format — they become shared memory.

## What the Installer Does

1. **Copies hooks** to `~/.claude/hooks/` (backs up existing files)
2. **Merges hook config** into `~/.claude/settings.json` (or creates if absent)
3. **Installs CLAUDE.md** starter template (skips if you already have one)
4. **Copies Codex hooks + skills** to `~/.codex/`
5. **Installs config.toml** template for Codex (skips if exists)
6. **Installs GEMINI.md** starter template (skips if exists)
7. **Copies .env.example** and **taxonomy.json.example** as reference

Safe to re-run — always backs up before overwriting.

## Customization

After installation, customize:
- `~/.claude/CLAUDE.md` — Add your own Iron Laws and project instructions
- `~/.claude/settings.json` — Add tool permissions as needed
- `~/.codex/config.toml` — Configure MCP servers and model preferences
- `~/.gemini/GEMINI.md` — Add Gemini-specific instructions

The hooks themselves rarely need modification — they're configured via environment variables.
