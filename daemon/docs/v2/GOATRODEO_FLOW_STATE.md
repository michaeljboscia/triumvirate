# Goat Rodeo Decision Ledger — Flow State v2.1

**Date:** 2026-04-06
**Rounds:** 4
**Questions:** 28 (all answered)
**Auto-resolves:** 29
**User decisions:** 5
**Final REQ count:** 27

---

## User Decisions

### D-1: Stderr Handling (REQ-022)
Concurrent reader task drains stderr to `tracing::debug!` (elevated to `error!` on non-zero exit). Alternative was redirecting stderr to stdout (breaks JSON parsing) or /dev/null (loses debug info).

### D-2: Event Channel + Verbosity (REQ-023)
Bounded channel (1024) with `try_send()` drop-oldest. AgentVerbosity taxonomy (Quiet/Standard/Detailed/Raw) instead of log levels. Outbox gets everything. Codex tweaks: WaitingOnApproval always visible, TurnStarted/Completed at Standard, ConfigWarning/ModelRerouted at Standard.

### D-3: App-Server Approval Requests (REQ-024)
Auto-approve all server-request types by default, wired through sandbox config. `workspace-write` sandbox scoped to project dir. OS enforces real boundary (Seatbelt/Landlock). Airlock configurable later.

### D-4: ask_twins Removal (REQ-015)
Hard remove now. No deprecation. No external callers. Must scrub stale refs in docs/env/README.

### D-5: Agent Event Taxonomy (REQ-023)
Gemini's AgentVerbosity with Codex's tweaks. Semantic events separate from system tracing. One env var: `TRIUMVIRATE_AGENT_VERBOSITY=quiet|standard|detailed|raw`. Default: `standard`.

**Mapping matrix:**

| Event | Quiet | Standard | Detailed | Raw |
|-------|:-----:|:--------:|:--------:|:---:|
| Error / Stuck / Completed | yes | yes | yes | yes |
| WaitingOnApproval / WaitingOnUserInput | yes | yes | yes | yes |
| TurnStarted / TurnCompleted | — | yes | yes | yes |
| ToolCall / Generating / Initializing | — | yes | yes | yes |
| ConfigWarning / ModelRerouted | — | yes | yes | yes |
| Thinking / ToolResult (duration) | — | — | yes | yes |
| Heartbeat / per-delta streams | — | — | — | yes |

---

## Clanker Consensus (Auto-Resolved)

1. Rename to `ParsedAgentResult` during transition (AR-1)
2. Stream-json for v2.1, ACP deferred to v2.2 (AR-2)
3. Use `LinesCodec`/`FramedRead` for cancellation safety (AR-3)
4. StuckDetector checks tool + arguments, not just name (AR-4)
5. Add `initialized` notification to Codex handshake (AR-5)
6. Handle all server-request types in app-server (AR-6)
7. `process_group(0)` is Unix-only — document constraint (AR-7)
8. All 21 original REQs directionally correct (AR-8)
9. Stderr drains to `debug!` not `warn!` (AR-9)
10. Channel `try_send()` drop-oldest, never block reader (AR-10)
11. Hard-deny `thread/shellCommand` (unsandboxed) (AR-11)
12. Auto-approve all server-request types (yolo default) (AR-12)
13. Scrub stale ask_twins refs in docs/env/README (AR-13)
14. `workspace-write` scoped to project dir, never home (AR-14)
15. Parsers line-at-a-time from Phase 1 (stream-ready) (AR-15)
16. exec --json and app-server are separate wire formats (AR-16)
17. exec --json lacks reasoning deltas (app-server only) (AR-17)
18. `requestUserInput`: deny with explicit message (AR-18)
19. StuckDetector catches repeated `requestUserInput` (AR-19)
20. Unknown event types: log and skip, never panic (AR-20)
21. Add version telemetry (cli_version, parser_mode) (AR-21)
22. Phase 3 PR checklist: setpgid + stderr drain + buffer accumulation (AR-22)
23. Codex can panic before JSONL — stderr drain essential (AR-23)
24. Codex exec --json mapping defined from binary symbols (AR-24)
25. Token counts in v2.1, not deferred (AR-25)
26. "Cost" reframed as token usage visibility (no dollars) (AR-26)
27. Phase 3 unified, Gemini first in PR (AR-27)
28. Golden trace capture required before Phase 2 (AR-28)
29. Unknown event types: graceful skip, never crash (AR-29)

---

## New Requirements Added During Goatrodeo

| REQ | Description | Round |
|-----|------------|-------|
| REQ-022 | Stderr drained concurrently to prevent deadlock | R1 |
| REQ-023 | Bounded channel + AgentVerbosity + full outbox logging | R1 |
| REQ-024 | Auto-approve via config, sandbox is real safety net | R1 |
| REQ-025 | Outbox rotation/GC (not blocking v2.1, but needed) | R2 |
| REQ-026 | Codex exec --json → WorkingState mapping (Section 11.3) | R4 |
| REQ-027 | Test strategy: mock scripts, start_paused, golden traces | R4 |

---

## Scope: v2.1 vs v2.2

### v2.1 (Core Flow State) — ships
- Phase 0: Types + agent-adapter crate
- Phase 1: Gemini stream-json parser (line-at-a-time)
- Phase 2: Codex exec --json parser (line-at-a-time)
- Phase 3: Both agents switch to .spawn() + live streaming
- Phase 5: StuckDetector + token usage capture
- Phase 6: AgentVerbosity display filter
- ask_twins removal

### v2.2 (Enrichment) — deferred
- Codex app-server mode (JSON-RPC over stdio)
- Gemini ACP mode (thinking tokens, tool kinds, diffs)
- Outbox enrichment (working_state, tool_name fields)
- Outbox GC/rotation

---

## Codex exec --json Event Mapping (from binary analysis)

| Event | WorkingState |
|-------|-------------|
| `turn.started` | `TurnStarted` |
| `agent_reasoning*` / `reasoning_*` | `Thinking` |
| `plan_delta` | `Planning` |
| `agent_message*` | `Generating` |
| `mcp_tool_call.begin` | `ToolCalling { tool, kind }` |
| `mcp_tool_call.end` | `ToolDone { tool, success, duration_ms }` |
| `exec_command.begin` | `ExecutingCommand { command }` |
| `exec_command.output_delta` | `ExecutingCommand` (streaming) |
| `exec_command.end` | `ToolDone { command, success }` |
| `patch_apply.begin` / `turn_diff` | `WritingFile { path }` |
| `patch_apply.end` | `ToolDone { success }` |
| `*_approval_request` / `request_user_input` | `WaitingForApproval` |
| `context_compacted` | `ContextCompacting` |
| `error` | `Error { message }` |
| `turn.complete` / `turn.aborted` | `TurnCompleted { status }` |
| `TokenCountEvent` | Populate `TokenUsage` |

Token fields: `input_token_count`, `output_token_count`, `cached_token_count`, `reasoning_token_count`
Correlators: `item_id`, `process_id`
Tool lifecycle: `*.begin` → `*.end`

---

## Phase 3 PR Checklist (Mandatory)

- [ ] `process_group(0)` via `.pre_exec(|| unsafe { libc::setpgid(0, 0); Ok(()) })`
- [ ] Kill process group on drop: `killpg(-pid, SIGKILL)`
- [ ] Concurrent stderr drain task (to `tracing::debug!`)
- [ ] Output buffer accumulation for final `ParsedAgentResult`
- [ ] `kill_on_drop(true)` on all spawns
- [ ] Timeout wraps entire spawn+read future
- [ ] `LinesCodec`/`FramedRead` or `AsyncBufReadExt::lines()` (cancellation safe)

---

## Next Step

Run `/uncompromising-executor` on the battle-tested spec to produce:
1. PRD.md
2. IMPLEMENTATION_PLAN.md
3. TEST_PLAN.md
4. BACKEND_STRUCTURE.md
(+ other canonical docs as needed)
