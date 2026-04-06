# CLAUDE.md Addendum — v2.1 "Flow State"

**Add this to the existing `docs/v2/CLAUDE.md` when starting v2.1 implementation.**

---

## v2.1 Canonical Docs (Supplement to v2.0 docs)

| Doc | Path | Contains |
|-----|------|----------|
| PRD (v2.1) | `docs/v2.1/PRD.md` | 10 features (FEAT-001 to FEAT-010) |
| Implementation Plan | `docs/v2.1/IMPLEMENTATION_PLAN.md` | 6 phases, exact files |
| Test Plan | `docs/v2.1/TEST_PLAN.md` | 27 REQ acceptance tests |
| Backend Structure | `docs/v2.1/BACKEND_STRUCTURE.md` | New crate, types, parsers |
| Tech Stack | `docs/v2.1/TECH_STACK.md` | Dependencies, env vars |
| Progress | `docs/v2.1/progress.txt` | Current phase/step |
| Goatrodeo Ledger | `docs/v2/GOATRODEO_FLOW_STATE.md` | All decisions + reasoning |
| Flow State Spec | `docs/v2/FLOW_STATE_SPEC.md` | Full spec with research |

## v2.1 Rules (In Addition to v2.0 Rules)

### New Crate: agent-adapter
- All protocol parsing logic lives here, NOT in triumvirate/src/main.rs
- Parsers are line-at-a-time: `fn parse_line(&str) -> Option<WorkingStateEvent>`
- Same parser works in batch mode (Phase 1/2) and streaming mode (Phase 3)
- Type is `ParsedAgentResult` NOT `AgentResult` (name collision avoidance)

### Process Safety
- ALL subprocess spawns use `kill_on_drop(true)` + `process_group(0)`
- ALL subprocess spawns drain stderr concurrently (deadlock prevention)
- Use `AsyncBufReadExt::lines()` or `LinesCodec` (cancellation safe), NEVER `read_line()`
- Bounded channels only (1024 cap) with `try_send()` — NEVER unbounded for event streams

### What's Forbidden (v2.1 additions)
- No `read_line()` in select! — use `lines()` stream (cancellation safety)
- No unbounded channels for WorkingStateEvents (OOM risk)
- No `thread/shellCommand` in Codex adapter (unsandboxed escape)
- No dollar-amount cost calculations (token counts only — user has unlimited subs)
- No blocking the stdout reader task (try_send, never send().await on event channel)

### AgentVerbosity
- Read from `TRIUMVIRATE_AGENT_VERBOSITY` env var
- Default: `standard`
- Invalid value: warn + fall back to standard
- Filter BEFORE ProgressEmitter — don't format strings you'll drop
- Outbox gets all events regardless of verbosity

### Session Startup Sequence (v2.1)
1. Read CLAUDE.md (this file + addendum)
2. Read `docs/v2.1/progress.txt`
3. Read `docs/v2.1/IMPLEMENTATION_PLAN.md`
4. Read `docs/v2/LESSONS.md`
5. Write `docs/v2.1/tasks/todo.md`
6. Verify plan with user before executing
