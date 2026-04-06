# CONTEXT HANDOFF — TRIUMVIRATE V2 CONVERSATIONAL RESET

**Date:** 2026-04-05
**From:** Claude session (frustrated user, trust broken by UX regression)
**For:** Next agent picking up this work

---

## What Happened

Core user expectation: conversational-first UX (plain language, no command ceremony), like old v1.

We delivered orchestration/backend power but regressed UX:
- Required explicit helper commands/scripts
- Unclear message lifecycle visibility
- Daemon lifecycle friction and perceived instability

User is asking to reset direction and salvage with another agent.

---

## Current Repo/Work Status

**Branch:** `feat/phase-0` in `/Users/mikeboscia/projects/triumvirate`
**Working dir:** `/Users/mikeboscia/projects/triumvirate/daemon`

### Commits Made This Session

| Hash | Message | Files |
|------|---------|-------|
| `a59b764` | fix: resolve BUG-001 Claude dashboard response pipeline | `daemon/crates/agentd/src/agent/claude.rs`, `daemon/crates/proto/src/claude_events.rs` |
| `aa05dbc` | feat: add ask-the-twins CLI wrapper for v2 API | `daemon/scripts/triumvirate-cli.sh`, `daemon/scripts/ask-the-twins` |
| `672b058` | fix: harden daemon lifecycle with launchd service + cli auto-start | `daemon/scripts/triumvirate-service.sh`, `daemon/scripts/triumvirate-cli.sh`, `daemon/README.md` |
| `f92daec` | fix: make ask-the-twins work via global symlink installs | `daemon/scripts/ask-the-twins` |
| `d152ec9` | RC1 test script | `daemon/scripts/test_rc1.sh` |
| `d61c409` | FEAT-031 implementation (earlier) | — |

### Test Status (Last Known)

- `cargo test` — GREEN
- `./daemon/scripts/test_rc1.sh --mock` — GREEN
- `./daemon/scripts/test_rc1.sh --live` — GREEN
- Core issue is NOT test red — it's UX/product mismatch

---

## Key User Complaint (BLOCKER)

> "This is a conversational interface first."

- "ask the twins" should be intent-level behavior, not command syntax
- User does not want to run scripts manually
- Wants confidence/status visibility: got it? working? hung? failed? retry?

---

## Non-Negotiable Product Requirements

1. **Plain message input defaults to twins fanout** — no command needed, just type and all agents get it
2. **Visible lifecycle state per request** — received → routed → in_progress → completed/failed
3. **Explicit agent error reporting** — auth missing, process down, timeout — with retry option
4. **Advanced commands remain optional power tools** — `/fleet`, `/debate` are not the primary flow
5. **Reliability feels invisible** — daemon/service managed underneath, user never thinks about it

---

## BUG-001 Diagnosis (Already Fixed in a59b764)

Two root causes were identified and fixed:

### Bug A: Missing `-p` flag on CLI invocation

`daemon/crates/agentd/src/agent/claude.rs` — the connector spawned Claude without `-p`. The CLI help says `--input-format` and `--output-format` **only work with `--print`**. Without it, both flags are silently ignored and Claude enters interactive TUI mode (hangs on piped stdio).

**Fix:** Changed to per-turn invocation model. Each message spawns:
```
claude -p --output-format stream-json --bare --dangerously-skip-permissions --session-id <uuid>
```
Message piped as plain text to stdin. Process exits after turn. Session continuity via `--session-id`.

### Bug B: Event parser didn't match real CLI output format

`daemon/crates/proto/src/claude_events.rs` — parser was written against assumed output format.

Real Claude CLI output (verified empirically):
- Assistant messages have `type: "assistant"`, not `"message"`
- Text is at `message.content[0].text` (array of content blocks), not a flat string
- Result `result` field is a plain string, not a nested object

**Fix:** Updated parser to handle all three cases.

### `--input-format stream-json` Does NOT Work for Persistent Sessions

Tested 8+ JSON formats — none produced a response. The feature either expects an undocumented format or is designed for a different use case. Per-turn invocation with plain text is the working approach.

---

## Immediate Salvage Plan

1. **STOP adding wrappers as primary UX**
2. **Implement default twins routing** for plain conversation path in API/router — every plain message fans out to all enabled agents
3. **Add request tracking** — message ID + per-agent status endpoint (received/routing/working/done/failed)
4. **Surface lifecycle + failures in dashboard/chat stream** — WebSocket events for state transitions
5. **Write "v2 conversational parity" doc** with user stories before further coding

---

## Files to Inspect First

| Priority | File | Why |
|----------|------|-----|
| 1 | `daemon/crates/agentd/src/routing.rs` | Currently routes to ONE agent. Needs twins fanout default. |
| 2 | `daemon/crates/agentd/src/web/server.rs` | Message handler, request tracking, lifecycle events |
| 3 | `daemon/frontend/src/lib/stores/commands.ts` | Frontend command handling |
| 4 | `daemon/frontend/src/routes/App.svelte` | Dashboard layout (4K fix landed but needs rebuild) |
| 5 | `docs/v2/TEST_PLAN.md` | 190+ tests, Part 8 is E2E runbook |
| 6 | `docs/v2/progress.txt` | Full project status + known bugs |

---

## Canonical Docs

| Doc | Path |
|-----|------|
| Test Plan | `/Users/mikeboscia/projects/triumvirate/docs/v2/TEST_PLAN.md` |
| Progress | `/Users/mikeboscia/projects/triumvirate/docs/v2/progress.txt` |
| PRD | `/Users/mikeboscia/projects/triumvirate/docs/v2/PRD.md` |
| SPEC | `/Users/mikeboscia/projects/triumvirate/SPEC.md` |
| CLAUDE.md (project) | `/Users/mikeboscia/projects/triumvirate/docs/v2/CLAUDE.md` |
| CLAUDE.md (daemon) | `/Users/mikeboscia/projects/triumvirate/daemon/.claude/CLAUDE.md` |
| Design System | `/Users/mikeboscia/projects/triumvirate/docs/v2/DESIGN_SYSTEM.md` |
| Backend Structure | `/Users/mikeboscia/projects/triumvirate/docs/v2/BACKEND_STRUCTURE.md` |
| BUG-001 Diagnosis | `/Users/mikeboscia/projects/triumvirate/docs/v2/BUG-001-DIAGNOSIS.md` |

---

## Tone Note

User is extremely frustrated and feels trust was broken by UX regression despite technical progress. Prioritize product-shape correction over more backend features. Lead with what works for the human, not what's architecturally interesting.
