---
name: send-to-codex
description: Send an inter-agent request to Codex for code generation, refactoring, and optimization.
---

# send-to-codex

**Purpose:** Send an inter-agent request to Codex for code-focused tasks.

**Created:** 2026-02-14
**Version:** 1.0

---

## When to Use

- Code generation or refactoring
- Performance optimization
- Getting a code review from a different model
- Escalating after 3 failures on a problem (use `/send-to-siblings` for dual escalation)

## Usage

**Basic:**
```
/send-to-codex "Refactor this function to be more efficient"
```

**With request type:**
```
/send-to-codex --type review "Is this implementation correct?"
/send-to-codex --type debug "Why is this failing?"
```

## Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `QUESTION` | Yes* | The question to ask Codex |
| `--type TYPE` | No | Request type: `question`, `review`, `debug`, `architecture`, `other` |
| `--context TEXT` | No | Brief context summary |
| `--session-log PATH` | No | Override session log path |
| `--no-commit` | No | Skip git commit of session log |

## Notes

- Requires the hooks-enabled Codex fork: `github.com/michaeljboscia/codex`
- Respects `CODEX_CLI_PATH` env var if codex is not in PATH
- Uses `--approval-mode full-auto` for headless operation

## Files

- `scripts/send.sh` — Main execution script
- `scripts/detect-context.sh` — Auto-detects repo/branch/taxonomy/session log
- `scripts/validate-message.sh` — Pre-flight validation of protocol fields

## Related Skills

- `/send-to-claude` — Send to Claude for infrastructure/development questions
- `/send-to-gemini` — Send to Gemini for research and analysis
- `/send-to-siblings` — Send to BOTH Gemini and Codex simultaneously (dual escalation)
