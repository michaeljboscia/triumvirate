# Test Plan — Triumvirate v2.1 "Flow State"

**Status:** Final
**Source:** PRD.md, GOATRODEO_FLOW_STATE.md (REQ-027)

---

## Test Strategy

- **Unit tests:** In `agent-adapter` crate. Feed recorded NDJSON through parsers, verify output.
- **Stuck detection:** Use `tokio::test(start_paused = true)` for deterministic time control.
- **Integration tests:** Mock connector scripts emit NDJSON with delays. Verify live event emission.
- **Regression:** `cargo test` across entire workspace after every phase.
- **Golden traces:** Real CLI output captured and committed as test fixtures before parser implementation.

---

## Acceptance Test Matrix

| REQ-ID | FEAT-ID | Acceptance Criteria | Test Type | Pass Condition | Pre-Implementation Baseline |
|--------|---------|-------------------|-----------|----------------|---------------------------|
| REQ-001 | FEAT-001 | All 6 Gemini stream-json event types parsed | Unit | Parser produces correct WorkingStateEvent for each type from golden trace | Only `message` (assistant) parsed; others discarded |
| REQ-002 | FEAT-002 | Codex exec JSONL events parsed (tool, command, file, token) | Unit | Parser produces correct events from golden trace; token counts populated | Only thread_id and final text extracted |
| REQ-003 | FEAT-003 | Real-time events emitted during execution | Integration | WorkingStateEvents arrive on mpsc channel BEFORE process exits | Events only available after .output() completes |
| REQ-004 | FEAT-005 | Human-readable progress messages via ProgressEmitter | Integration | Messages contain tool names, file paths, commands — not just "working..." | "working... (Ns elapsed)" only |
| REQ-005 | FEAT-005 | Heartbeat is fallback only | Integration | Heartbeat timer fires only when no real events received for 30s | Heartbeat fires on fixed 10s/40s/60s schedule |
| REQ-006 | FEAT-004 | Unified WorkingState enum handles both protocols | Unit | Both Gemini and Codex parsers produce valid WorkingState variants | No unified type exists |
| REQ-007 | FEAT-003 | .spawn() with line-by-line reading | Integration | Lines read from stdout before process exit; timeout still works | .output() buffers everything |
| REQ-008 | FEAT-007 | Token usage captured from both agents | Unit | ParsedAgentResult.token_usage populated from Gemini result.stats and Codex TokenCountEvent | Token usage not captured |
| REQ-009 | FEAT-006 | Stuck detection: Gemini LoopDetected + Codex StuckDetector | Unit + Integration | Gemini: LoopDetected → Stuck event. Codex: 60s idle → Stuck. Same tool+args 5x → Stuck. Repeated requestUserInput → Stuck | No stuck detection |
| REQ-010 | FEAT-006 | Stuck events surface to MCP caller | Integration | WorkingState::Stuck emitted through ProgressEmitter | No stuck notification |
| REQ-011 | FEAT-010 | Codex app-server opt-in via env var | Unit | `TRIUMVIRATE_CODEX_PROTOCOL` defaults to "exec"; "app-server" activates app-server path | N/A (app-server path deferred to v2.2, but env var plumbing in v2.1) |
| REQ-012 | FEAT-010 | Fallback to raw stdout on empty parse | Unit + Integration | If parser.finish().response_text is empty, raw stdout used | Current behavior (raw stdout) |
| REQ-013 | FEAT-010 | Gemini streaming disableable | Integration | `TRIUMVIRATE_GEMINI_STREAMING=false` → .output() batch mode, events only at end | N/A (streaming not yet implemented) |
| REQ-014 | FEAT-003 | kill_on_drop + process_group(0) | Manual | Spawn agent, kill daemon mid-execution, verify no orphan processes remain | Orphan processes possible |
| REQ-015 | FEAT-008 | ask_twins fully removed | Unit + Grep | `cargo test` passes. `grep -r "ask_twins"` returns zero matches in src/ | ask_twins tool exists |
| REQ-016 | FEAT-004 | Tool call records include name, success, duration | Unit | ToolCallRecord populated from both Gemini tool_result and Codex *.end events | No tool call records |
| REQ-017 | — | OutboxEvent extended (deferred to v2.2) | — | DEFERRED | — |
| REQ-018 | FEAT-009 | agent-adapter crate exists and compiles | Unit | `cargo check -p agent-adapter` succeeds. Types serialize/deserialize. | Crate does not exist |
| REQ-019 | FEAT-010 | ask_session response unchanged | Integration | MCP ask_session returns final text string, same format as today | Current format |
| REQ-020 | — | Codex app-server handshake (deferred to v2.2) | — | DEFERRED | — |
| REQ-021 | FEAT-010 | All existing tests pass | Regression | `cargo test` across workspace: 0 failures | All tests pass |
| REQ-022 | FEAT-003 | Stderr drained concurrently | Integration | Agent writing to stderr does NOT deadlock. Stderr output appears in tracing::debug! | Stderr not read (deadlock risk) |
| REQ-023 | FEAT-005 | AgentVerbosity filter works | Unit | should_display() returns correct bool for all (state, verbosity) combinations per matrix | No verbosity filter |
| REQ-024 | — | App-server auto-approve (deferred to v2.2) | — | DEFERRED | — |
| REQ-025 | — | Outbox GC (deferred) | — | DEFERRED | — |
| REQ-026 | FEAT-002 | Codex exec --json mapping complete | Unit | Golden trace parsed correctly. All event families produce correct WorkingState | No exec --json parsing |
| REQ-027 | FEAT-010 | Test strategy implemented (mocks, start_paused, fixtures) | Meta | Golden traces committed. Mock scripts exist. StuckDetector tests use start_paused. | No streaming tests exist |

---

## Deferred Tests (v2.2)

| REQ-ID | Reason |
|--------|--------|
| REQ-017 | OutboxEvent enrichment deferred |
| REQ-020 | Codex app-server deferred |
| REQ-024 | App-server approval deferred |
| REQ-025 | Outbox GC deferred |

---

## Test Fixtures Required

| Fixture | Source | Phase |
|---------|--------|-------|
| `tests/fixtures/gemini-stream-trace.jsonl` | Live capture: `gemini -o stream-json -p "..."` | Phase 1 |
| `tests/fixtures/codex-exec-trace.jsonl` | Live capture: `codex exec "..." --json` | Phase 2 |
| `tests/fixtures/gemini-mock-streaming.sh` | Mock script: emits NDJSON with sleep delays | Phase 3 |
| `tests/fixtures/codex-mock-streaming.sh` | Mock script: emits exec JSONL with sleep delays | Phase 3 |
| `tests/fixtures/stuck-no-events.jsonl` | Synthetic: turn.started then silence | Phase 4 |
| `tests/fixtures/stuck-loop.jsonl` | Synthetic: same tool+args repeated 6x | Phase 4 |
