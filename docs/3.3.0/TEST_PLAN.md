# TEST_PLAN — v3.3.0 Live Agent Streaming

**Version:** 3.3.0
**Spec:** docs/3.3.0/SPEC.md (25 REQs, 10 dropped in goatrodeo = 25 active)

## Acceptance Test Matrix

| REQ-ID | FEAT-ID | Acceptance Criteria | Test Type | Pass Condition | Reality Test | Pre-Implementation Baseline |
|--------|---------|-------------------|-----------|---------------|-------------|---------------------------|
| REQ-E01 | FEAT-001 | AgentStreamEvent enum has 6 variants with seq field | Unit | All variants construct, serialize, deserialize with correct serde tag | Construct all 6 variants, round-trip JSON, assert `event_type` tag and `seq` field present | Type does not exist |
| REQ-E02 | FEAT-001 | AgentStreamEvent defined in shared-types crate | Unit | `use shared_types::AgentStreamEvent` compiles from any workspace crate | Import from triumvirate binary crate, construct a variant, assert it compiles | Type does not exist in shared-types |
| REQ-E03 | FEAT-001 | Parsers emit events via mpsc channel; adapter collects to blob | Unit + Integration | GeminiStreamParser produces TurnStarted + ToolCall + TurnCompleted events for a recorded stream; execute_ask_agent() returns blob unchanged | Feed recorded NDJSON fixture, assert channel yields >=3 events with correct types AND final string matches expected response | Parsers return String only, no channel |
| REQ-E04 | FEAT-001 | WS broadcast emits agent_stream alongside existing events | Unit | Subscribe to WS broadcast, publish AgentStreamEvent, receive it as agent_stream JSON. Publish token_update, receive it unchanged. | Publish both event types, assert both received with correct type discriminators, assert existing event shapes unchanged | WS broadcast has no agent_stream event type |
| REQ-H01 | FEAT-002 | POST /mcp accepts JSON-RPC, returns SSE or JSON | Integration | POST initialize to /mcp returns valid JSON-RPC response with server capabilities | POST request to /mcp, parse response, assert tools/list returns 35+ tools | /mcp endpoint does not exist |
| REQ-H02 | FEAT-002 | GET /mcp establishes SSE connection | Integration | GET /mcp with Accept: text/event-stream opens persistent connection | Connect via GET, assert Content-Type is text/event-stream, assert connection stays open | /mcp endpoint does not exist |
| REQ-H03 | FEAT-002 | Session ID via Mcp-Session-Id header | Integration | Initialize response contains Mcp-Session-Id header; subsequent requests with that header are accepted | POST initialize, extract header, POST tools/list with same header, assert success | No MCP session management exists |
| REQ-H04 | FEAT-002 | Both stdio and HTTP transports active simultaneously | Integration | Start daemon, connect via HTTP /mcp AND run `triumvirate mcp` via stdio. Both return same tool list. | Call tools/list on both transports, assert identical tool counts | Only stdio transport exists |
| REQ-H05 | FEAT-002 | Streaming formatted text chunks during tool execution | Integration | POST ask_agent tool call, SSE stream contains >=1 intermediate frame with "→ {agent}:" prefix before final result | Parse SSE frames, assert at least one contains formatted progress text, assert final frame is CallToolResult | Tool calls return single blob |
| REQ-H06 | FEAT-002 | Final result as last SSE frame, stream closes | Integration | After final JSON-RPC result frame, SSE connection closes cleanly | Assert stream EOF after result frame, no hanging connection | N/A |
| REQ-H07 | FEAT-002 | Uses rmcp transport-streamable-http-server | Unit | Cargo.toml includes the feature flag, daemon compiles with it | `cargo check -p triumvirate` with feature enabled | Feature flag not in Cargo.toml |
| REQ-H08 | FEAT-002 | Works with claude mcp add --transport http | Manual | Register daemon as HTTP MCP server in Claude Code, call a tool, assert it works | Run `claude mcp add --transport http triumvirate-test http://127.0.0.1:8080/mcp`, call ping, assert response | Only stdio transport configured |
| REQ-H09 | FEAT-002 | Bearer token auth on /mcp | Integration | POST to /mcp without token returns 401; with valid token returns 200 | Send unauthenticated request, assert 401. Send authenticated, assert success. | /mcp endpoint does not exist |
| REQ-H10 | FEAT-002 | Integration tests verify SSE streaming | Integration | Test file exists with >=5 tests covering SSE, sessions, auth, streaming | Run integration_streaming tests, all pass | Test file does not exist |
| REQ-P01 | FEAT-003 | Proxy bridges stdio↔HTTP | Integration | Write JSON-RPC to proxy stdin, assert response on stdout matches daemon response | Start daemon + proxy subprocess, pipe tools/list request, compare with direct HTTP response | Proxy command does not exist |
| REQ-P02 | FEAT-003 | Auto-reconnect with bounded backoff | Integration | Kill daemon, proxy fails in-flight call, restart daemon, next proxy call succeeds | Sequence: call → kill daemon → call (expect error) → restart daemon → call (expect success) | Proxy command does not exist |
| REQ-P03 | FEAT-003 | Exit with clear error if daemon unreachable at startup | Unit | Proxy exits within 6s with error message containing "daemon not reachable" | Start proxy without daemon running, assert exit code non-zero, assert stderr contains error message | Proxy command does not exist |
| REQ-P04 | FEAT-003 | Unit tests for proxy | Unit | Tests exist for forwarding, reconnect, clean exit | Run cargo test, proxy tests pass | No proxy tests exist |
| REQ-W01 | FEAT-004 | Watch connects to /ws and pretty-prints | Integration | Watch stdout shows "→ {agent}: {action}" for published events | Start daemon, publish mock event to WS broadcast, assert watch stdout contains formatted line | Watch command does not exist |
| REQ-W02 | FEAT-004 | Default filter: agent_stream only | Integration | Watch shows agent_stream events, does NOT show token_update events unless --all | Publish both event types, assert only agent_stream appears without --all flag | Watch command does not exist |
| REQ-W03 | FEAT-004 | --session filter | Integration | Watch with --session research shows only events where session_name == "research" | Publish events for two sessions, assert filter works | Watch command does not exist |
| REQ-W04 | FEAT-004 | Heartbeat during long generation | Integration | During 10s gap between TurnStarted and TurnCompleted, watch shows elapsed timer | Send TurnStarted, wait 5s, assert stdout contains "elapsed" text | Watch command does not exist |
| REQ-W05 | FEAT-004 | Handles daemon-not-running | Unit | Watch retries with message, not crash | Start watch without daemon, assert no panic, assert stderr contains retry message | Watch command does not exist |
| REQ-W06 | FEAT-004 | Sequence gap detection | Integration | Skip seq number, watch prints gap warning | Publish events with seq 1, 2, 5 (skip 3,4). Assert warning about skipped events. | Watch command does not exist |
| REQ-K01 | FEAT-005 | Spike test documents Claude Code SSE behavior | Manual | SPIKE_RESULTS.md exists with YES or NO and evidence | Read the document, verify it has a definitive answer | No spike test exists |

## Summary

| Category | REQ Count | Test Count | Unit | Integration | Manual |
|----------|-----------|------------|------|-------------|--------|
| Event Schema (E) | 4 | 4 | 3 | 1 | 0 |
| HTTP Transport (H) | 10 | 10 | 1 | 8 | 1 |
| Proxy (P) | 4 | 4 | 2 | 2 | 0 |
| Watch CLI (W) | 6 | 6 | 1 | 5 | 0 |
| Spike (K) | 1 | 1 | 0 | 0 | 1 |
| **Total** | **25** | **25** | **7** | **16** | **2** |

Zero orphan REQs. Every requirement has a test.
