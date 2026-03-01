---
name: send-to-gemini
description: Send an inter-agent request to Gemini using the standardized protocol format.
---

# send-to-gemini

**Purpose:** Send an inter-agent request to Gemini for research, analysis, or long-running tasks.

**Created:** 2026-02-14
**Version:** 1.0

---

## When to Use

- Deep research or web search tasks
- Analyzing large datasets or documents
- Getting a second opinion on architecture
- Escalating after 3 failures on a problem (use `/send-to-siblings` for dual escalation)

## Usage

**Basic:**
```
/send-to-gemini "Research the best approach for X"
```

**With request type:**
```
/send-to-gemini --type review "Does this architecture make sense?"
/send-to-gemini --type question "What are the tradeoffs between A and B?"
```

## Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `QUESTION` | Yes* | The question to ask Gemini |
| `--type TYPE` | No | Request type: `question`, `review`, `debug`, `architecture`, `other` |
| `--context TEXT` | No | Brief context summary |
| `--session-log PATH` | No | Override session log path |
| `--no-commit` | No | Skip git commit of session log |

## Notes

- Uses `--output-format text` to disable glamour (markdown renderer) — required for clean programmatic output
- Uses `--approval-mode yolo` for headless operation
- Respects `GEMINI_CLI_PATH` env var if gemini is not in PATH

## Files

- `scripts/send.sh` — Main execution script
- `scripts/detect-context.sh` — Auto-detects repo/branch/taxonomy/session log
- `scripts/validate-message.sh` — Pre-flight validation of protocol fields

## Related Skills

- `/send-to-claude` — Send to Claude for infrastructure/development questions
- `/send-to-codex` — Send to Codex for code generation and review
- `/send-to-siblings` — Send to BOTH Gemini and Codex simultaneously (dual escalation)
