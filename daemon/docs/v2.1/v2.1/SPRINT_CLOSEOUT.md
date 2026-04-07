# v2.1 Sprint Closeout (daemon-v2)

Date: 2026-04-06
Branch: feat/mcp-first-final

## Scope Delivered

- Added `agent-adapter` crate with:
  - unified types (`WorkingState`, `WorkingStateEvent`, `ParsedAgentResult`, `TokenUsage`, `ToolCallRecord`, `ToolKind`, `AgentVerbosity`)
  - `GeminiStreamParser` (stream-json)
  - `CodexExecParser` (exec --json)
  - `StuckDetector` foundation
  - `format_working_state` + verbosity filtering
- Switched agent execution from batch-only parsing to live line-by-line `.spawn()` parsing for Gemini and Codex in `agent_exec`.
- Added bounded working-event channel path and progress emission from real events.
- Added process safety hardening:
  - `kill_on_drop(true)`
  - process-group setup (`setpgid`) on unix
  - process-group kill on timeout
  - concurrent stderr drain
- Added env controls:
  - `TRIUMVIRATE_AGENT_VERBOSITY=quiet|standard|detailed|raw` (legacy aliases supported)
  - `TRIUMVIRATE_GEMINI_STREAMING=false` fallback
  - `TRIUMVIRATE_CODEX_PROTOCOL` parser for `exec|app-server` value plumbing
- Removed stale ask-twins env surface from README and verified no `ask_twins` symbol references remain in source.
- Added fixtures:
  - `tests/fixtures/gemini-stream-trace.jsonl`
  - `tests/fixtures/codex-exec-trace.jsonl`
  - `tests/fixtures/gemini-mock-streaming.sh`
  - `tests/fixtures/codex-mock-streaming.sh`

## REQ Status (from docs/v2.1/TEST_PLAN.md)

- REQ-001: PASS
- REQ-002: PASS
- REQ-003: PASS
- REQ-004: PASS
- REQ-005: PASS
- REQ-006: PASS
- REQ-007: PASS
- REQ-008: PASS
- REQ-009: PASS (detector foundation + loop/input/timeout checks)
- REQ-010: PASS
- REQ-011: PASS (env plumbing)
- REQ-012: PASS
- REQ-013: PASS
- REQ-014: PASS (process group + kill path implemented)
- REQ-015: PASS
- REQ-016: PASS
- REQ-017: DEFERRED (v2.2)
- REQ-018: PASS
- REQ-019: PASS
- REQ-020: DEFERRED (v2.2)
- REQ-021: PASS
- REQ-022: PASS
- REQ-023: PASS
- REQ-024: DEFERRED (v2.2)
- REQ-025: DEFERRED (v2.2)
- REQ-026: PASS
- REQ-027: PASS

## Verification

- `cargo test` full workspace: PASS
- Existing daemon/session reliability tests remain green.
