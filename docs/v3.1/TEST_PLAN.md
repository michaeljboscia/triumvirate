# v3.1 MCP Consolidation — Test Plan

**Spec:** `specs/MCP_CONSOLIDATION.md`

---

## REQ-to-Test Matrix

| REQ | Acceptance Criteria | Test Type | Pass Condition | Reality Test |
|-----|-------------------|-----------|----------------|-------------|
| REQ-C1 | Tool handlers in mcp-tools modules | Unit + Integration | Each module compiles independently. All existing MCP tests pass. | Call each tool via MCP after extraction — identical behavior |
| REQ-C2 | HTTP routes in daemon-http | Integration | All HTTP endpoints respond correctly | curl each endpoint — identical responses |
| REQ-C3 | DaemonState in daemon-core | Unit | DaemonState constructs from daemon-core | Import and instantiate from main.rs — compiles |
| REQ-C4 | main.rs under 300 lines | Metric | `wc -l < 300` | grep for tool/route handlers returns zero |
| REQ-A1 | Alias tools registered | Integration | Each alias callable via MCP | Call spawn_daemon → session created |
| REQ-A2 | Parameter mapping works | Unit + Integration | TS schemas accepted, Rust schemas produced | spawn_daemon {target:"gemini"} maps to {agent:"gemini"} |
| REQ-A3 | Alias usage logged | Integration | tracing output includes tool_alias field | Daemon log shows alias events after alias call |
| REQ-B1 | ObservabilityBus exists | Unit | Struct compiles, cloneable via Arc | Construct in test, clone, access metrics and ws_events |
| REQ-B2 | DaemonMetrics accessible from mcp-tools | Unit | mcp-tools can increment a counter | Import DaemonMetrics, call .inc() — compiles |
| REQ-B3 | publish_ws_event on ObservabilityBus | Unit | Method exists and sends | Call publish_event, receiver gets message |
| REQ-F1 | Rust daemon serves MCP via stdio | Integration | Claude connects and lists tools | `triumvirate mcp` → tool list includes all 40+ tools |
| REQ-F2 | ~/.claude.json updated | Config | inter-agent entry removed | grep "inter-agent" ~/.claude.json returns nothing |
| REQ-F3 | Tool descriptions adequate | Lint | Every tool has description >= 20 chars | Automated check in test |
| REQ-F4 | MCP lifecycle correct | Integration | initialize, tools/list, tools/call all work | Full MCP session via test client |
| REQ-X1 | TS server archived | Filesystem | archive/mcp-server-ts/ exists, mcp-server/ gone | ls -d confirms |
| REQ-X2 | inter-agent config removed | Config | No inter-agent in ~/.claude.json | grep confirms |
| REQ-X3 | No Node.js runtime needed | Process | No node process for MCP | pgrep -f "inter-agent" returns nothing |
| REQ-J2 | send-to-codex uses ask_session | Skill | No mcp__inter-agent references | grep skill file |
| REQ-J3 | send-to-gemini uses ask_session | Skill | No mcp__inter-agent references | grep skill file |
| REQ-J4 | send-to-siblings uses ask_session | Skill | No mcp__inter-agent references | grep skill file |

---

## Test Categories

### Category 1: Extraction Parity (Wave 1-2)
For EVERY tool handler extracted from main.rs:
1. Call the tool with known inputs before extraction (capture response)
2. Extract the handler
3. Call with same inputs after extraction
4. Assert: responses are byte-identical (or structurally identical for JSON)

### Category 2: Alias Correctness (Wave 3)
For EVERY alias:
1. Call the alias with TS-schema parameters
2. Assert: response matches calling the canonical tool with Rust-schema parameters
3. Assert: daemon log contains `tool_alias` tracing event

### Category 3: Front Door Swap (Wave 4)
1. Remove inter-agent from ~/.claude.json
2. Call every tool that was previously on inter-agent
3. Assert: all respond correctly via triumvirate
4. Assert: no Node.js process running
5. Run full goat rodeo to exercise twin communication

### Category 4: Regression (All Waves)
After EVERY wave:
1. `cargo test --workspace` — all 156+ existing tests pass
2. No new warnings
3. `cargo clippy --workspace` clean (or no new warnings)

---

## Pre-Implementation Baseline

Capture before starting Wave 1:
```bash
# Tool count
triumvirate mcp --list-tools | wc -l

# Test count
cargo test --workspace 2>&1 | tail -1

# main.rs line count
wc -l daemon/crates/triumvirate/src/main.rs

# Running processes
pgrep -f "inter-agent" && echo "TS server running"
pgrep -f "triumvirate" && echo "Rust daemon running"
```
