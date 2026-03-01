---
name: send-to-siblings
description: Send the same inter-agent request to BOTH Gemini and Codex simultaneously. Use for dual escalation after 3 failures.
---

# send-to-siblings

**Purpose:** Dual escalation — fires `/send-to-gemini` and `/send-to-codex` in parallel, waits for both responses.

**Created:** 2026-02-14
**Version:** 1.0

---

## When to Use

**The 3-Failures Rule:** If you've failed to fix or understand something after 3 attempts, STOP and escalate to both siblings simultaneously.

- Same problem approached 3 different ways with no progress
- Architectural decision where you want two independent perspectives
- Bug where you need fresh eyes from different reasoning approaches

## Usage

**Basic:**
```
/send-to-siblings "I've failed 3 times to fix this. What am I missing?"
```

**With context:**
```
/send-to-siblings --type debug --context "Tried X, Y, Z — all failed" "What am I missing?"
```

**With request type:**
```
/send-to-siblings --type review "Two reviews are better than one for this critical function"
```

## Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `QUESTION` | Yes* | The question to send to both siblings |
| `--type TYPE` | No | Request type: `question`, `review`, `debug`, `architecture`, `other` |
| `--context TEXT` | No | Brief context summary |
| `--session-log PATH` | No | Override session log path |
| `--no-commit` | No | Skip git commit of session log |

## How It Works

1. Fires `send-to-gemini` in the background
2. Fires `send-to-codex` in the background
3. Waits for both to complete
4. Prints both responses side-by-side

Both agents receive identical questions and context. Their independent reasoning
catches different types of errors — Gemini tends to spot requirement misunderstandings,
Codex tends to spot implementation bugs.

## Notes

- Both `send-to-gemini` and `send-to-codex` must be installed (included in this starter-kit)
- Responses are printed sequentially after both complete (not streamed)
- If one agent fails, the other's response is still shown

## Files

- `scripts/send.sh` — Parallel launcher (delegates to sibling send scripts)

## Related Skills

- `/send-to-claude` — Send to Claude for infrastructure/development questions
- `/send-to-gemini` — Send to Gemini only
- `/send-to-codex` — Send to Codex only
