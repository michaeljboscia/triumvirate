# v3.3.0 Live Agent Streaming — Codex Spec Review

## Executive Verdict
Given the confirmed Claude Code behavior (no `progressToken` forwarding + no visible `notifications/progress` rendering in terminal), **Phase 1 as currently written is not a user-visible solution**. The viable path is **Streamable HTTP + SSE with incremental tool result frames**.

Recommendation:
- Drop Phase 1 requirements that are specifically about MCP progress notifications over stdio.
- Keep parser/event-model work, but retarget it to SSE output.
- Implement HTTP streaming in a compatibility-preserving way (additive, not replacement).

## REQ-by-REQ Review

### Phase 1 (REQ-S01 .. REQ-S10)

| REQ | Verdict | Reasoning |
|---|---|---|
| REQ-S01 | DROP | Emitting `notifications/progress` during stdio `ask_session` will not be visible in Claude Code terminal, so this does not satisfy the product goal. |
| REQ-S02 | MODIFY | Keep human-readable activity strings, but apply them to SSE stream event payloads (partial tool result frames), not MCP progress notifications. |
| REQ-S03 | DROP | `progress`/`total` semantics are tied to `notifications/progress`. Also event-count totals are speculative and noisy for agent turns. |
| REQ-S04 | DROP | Same issue as S01 for `ask_agent`: invisible in Claude Code stdio path. |
| REQ-S05 | DROP | `progressToken` dependency is non-viable because Claude Code does not pass it through. |
| REQ-S06 | DROP | `context.peer.notify_progress()` is the wrong transport for this client reality. |
| REQ-S07 | MODIFY | Keep structured parser events, but emit a transport-agnostic stream enum consumed by SSE writer first; progress layer optional/secondary. |
| REQ-S08 | MODIFY | Same as S07 for Codex parser: keep structured events, target SSE-visible output. |
| REQ-S09 | AGREE | Graceful degradation remains important; if streaming path unavailable, final tool call should still complete normally. |
| REQ-S10 | MODIFY | Replace progress-layer unit tests with event-mapping tests + SSE integration tests that assert user-visible incremental frames. |

### Phase 2 (REQ-H01 .. REQ-H10)

| REQ | Verdict | Reasoning |
|---|---|---|
| REQ-H01 | AGREE | Core requirement. Streamable HTTP endpoint is the only path likely to produce live visible output in Claude Code. |
| REQ-H02 | MODIFY | Keep `GET /mcp` only if needed for spec compliance/future server-initiated notifications. Not required for immediate user-visible streaming if POST stream suffices. |
| REQ-H03 | AGREE | Session continuity via `Mcp-Session-Id` is correct and useful for multi-turn behavior. |
| REQ-H04 | MODIFY | Coexistence is desirable, but dual-transport on the exact same runtime/port increases complexity and blast radius. Prefer HTTP streaming as primary path; keep stdio as legacy path, not feature-parity requirement. |
| REQ-H05 | MODIFY | Stream **partial tool result chunks** as SSE frames (Claude-visible), not only JSON-RPC `notifications/progress`. |
| REQ-H06 | AGREE | Final result as terminal SSE frame then close is a clean completion contract. |
| REQ-H07 | AGREE | Using rmcp streamable HTTP server support is safer than custom framing. |
| REQ-H08 | AGREE | Must explicitly validate the documented Claude config command and behavior. |
| REQ-H09 | AGREE | Auth parity with existing HTTP API is required. |
| REQ-H10 | MODIFY | Integration tests must assert at least 2 incremental **tool_result/partial** SSE frames visible before final result frame (not just progress notifications). |

### Event Schema (REQ-E01 .. REQ-E04)

| REQ | Verdict | Reasoning |
|---|---|---|
| REQ-E01 | MODIFY | `AgentStreamEvent` is right, but include `request_id`, monotonic `seq`, and `ts_ms` for deterministic ordering/replay across transports. |
| REQ-E02 | AGREE | `shared-types` placement is correct for reuse by parser, SSE, and websocket layers. |
| REQ-E03 | MODIFY | Parsers should emit `AgentStreamEvent` over channel, but keep compatibility adapter from existing `WorkingStateEvent` during migration to reduce churn. |
| REQ-E04 | MODIFY | Do not replace current `/ws` envelope outright; emit both formats (or versioned payload) during transition to prevent consumer breakage. |

## Architecture Question Answers

### 1) Should Phase 1 be dropped?
**Yes, as specified.**
- Drop stdio progress-notification work as a delivery milestone because it is not user-visible in Claude Code.
- Salvage parser/event normalization work from Phase 1 and re-scope it under SSE delivery.

### 2) Dual transport (stdio + HTTP same port) vs HTTP-only
**Safer choice for v3.3.0: HTTP-first streaming + legacy stdio fallback, not strict dual-parity requirement.**
- For the streaming objective, HTTP is the only meaningful path.
- Maintaining two transports with identical semantics from day one increases test matrix and regression surface.
- Keep stdio operational for compatibility, but do not block release on matching streaming behavior there.

### 3) `agent_executor` blob-return → channel-return refactor risk
**High risk if done as a hard replacement; medium-low if additive.**
- Current flow already has an internal event channel (`mpsc<WorkingStateEvent>`) plus final `AskAgentResponse`.
- A full signature-level replacement will ripple across MCP tools, daemon HTTP executor wiring, shared types, and tests.
- Safer path: keep final response contract, add a stream sink/adapter that forwards normalized events to SSE in parallel.

### 4) New `AgentStreamEvent` vs existing WebSocket broadcast format
**Both (during migration), then deprecate old format later.**
- Current `/ws` uses a stable envelope (`type`, `ts_ms`, `payload`) that existing consumers may rely on.
- Replace-in-place creates unnecessary break risk.
- Emit new schema in parallel (or nest it inside current payload), add a deprecation window, then remove old path in a later version.

## Proposed Spec Rewrite Direction (Concise)
- Reframe "Phase 1" as **Event Normalization Layer** (parser -> `AgentStreamEvent`), not progress notifications.
- Reframe "Phase 2" as **User-Visible Streaming Delivery** (HTTP SSE partial tool result chunks + final frame).
- Make compatibility explicit: additive transport/event rollout, no abrupt websocket schema break.

