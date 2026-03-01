---
name: send-to-claude
description: Send an inter-agent request to Claude using the standardized protocol format.
---

# send-to-claude

**Purpose:** Send an inter-agent request to Claude using the standardized protocol format.

**Created:** 2026-02-14
**Version:** 1.0

---

## What This Skill Does

Automates sending properly formatted inter-agent requests to Claude:
1. Auto-detects context (REPO, BRANCH, CWD, TAXONOMY)
2. Validates all 9 required protocol fields
3. Creates/updates session log if needed
4. Git commits session log
5. Generates properly formatted message
6. Sends to Claude via CLI
7. Returns Claude's response

## When to Use

- Need Claude's Supabase/n8n/infrastructure expertise
- Escalating after 3 failures on a problem (use `/send-to-siblings` for dual escalation)
- Requesting architectural guidance or system design help
- Any question where Claude's domain knowledge applies

## Usage

**Basic:**
```
/send-to-claude "How should I structure this Supabase function?"
```

**With request type:**
```
/send-to-claude --type question "What's the right n8n workflow pattern for this?"
/send-to-claude --type review "Review this edge function for correctness"
/send-to-claude --type architecture "Should this use RLS or app-level permissions?"
```

**With context:**
```
/send-to-claude --context "Tried direct calls and queue-based" "Which approach is better?"
```

**Interactive mode:**
```
/send-to-claude
# Prompts for request type, question, and context
```

## Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `QUESTION` | Yes* | The question to ask Claude (if not provided, prompts interactively) |
| `--type TYPE` | No | Request type: `question`, `review`, `debug`, `architecture`, `other` (default: `question`) |
| `--context TEXT` | No | Brief context summary (default: "Full context in session log") |
| `--session-log PATH` | No | Override session log path (default: auto-detect or create) |
| `--no-commit` | No | Skip git commit of session log (use with caution) |

## What Gets Auto-Detected

The skill automatically detects:
- **REPO:** Git remote URL or current directory path
- **BRANCH:** Current git branch name
- **CWD:** Absolute path to current working directory
- **TAXONOMY:** From `.gemini/taxonomy.json` or `.codex/taxonomy.json`, git remote, or directory
- **TIMESTAMP:** Current time in America/New_York timezone (ISO-8601)
- **SESSION_LOG:** Most recent session log in `session-logs/` or creates new one

## Protocol Fields

All 9 required fields are populated:
1. FROM: `gemini` or `codex` (auto-detected)
2. TO: `claude`
3. TIMESTAMP: Auto-generated (EST/EDT)
4. REPO: Auto-detected from git
5. BRANCH: Auto-detected from git
6. SESSION_LOG: Auto-detected or created
7. TAXONOMY: Auto-detected
8. CWD: Auto-detected
9. REQUEST_TYPE: From `--type` or default `question`

## Pre-Flight Checks

Before sending, the skill verifies:
- [ ] All 9 required fields are non-empty
- [ ] Session log exists (creates if missing)
- [ ] Git repo is valid (if in a git directory)
- [ ] Session log is committed (commits if needed, unless `--no-commit`)
- [ ] Claude CLI is available

## Output

Returns Claude's response in formatted output.

## Examples

See `examples/usage.md` for detailed examples.

## Files

- `scripts/send.sh` - Main execution script
- `scripts/detect-context.sh` - Auto-detection logic
- `scripts/validate-message.sh` - Pre-flight validation
- `message-template.txt` - Protocol message template
- `examples/usage.md` - Usage examples

## Related Skills

- `/send-to-gemini` — Send to Gemini for research and analysis
- `/send-to-codex` — Send to Codex for code generation and review
- `/send-to-siblings` — Send to BOTH Gemini and Codex simultaneously (dual escalation)

## Protocol Reference

See the Triumvirate README for the full inter-agent protocol specification:
`https://github.com/michaeljboscia/triumvirate`
