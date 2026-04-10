# BUILD MANIFEST — v3.1.0 MCP Consolidation

**Reconstructed from git history (manifest not produced during build)**

## chore(3.1.0): bump workspace version, wire version reporting, add drift hook script
- **Commit:** 882a113
- **Date:** 2026-04-09
- **Files:** ROADMAP.md,daemon/Cargo.lock,daemon/Cargo.toml,daemon/crates/daemon-core/src/lib.rs,daemon/crates/daemon-core/src/version.rs,daemon/crates/triumvirate/src/cli_ops.rs,daemon/crates/triumvirate/src/main.rs,scripts/install-git-hooks.sh,scripts/version-drift-check.sh

## chore(3.1.0): T-001 move DaemonMetrics to daemon-core, define ObservabilityBus
- **Commit:** 51daf5c
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/daemon-core/Cargo.toml,daemon/crates/daemon-core/src/lib.rs,daemon/crates/daemon-core/src/metrics.rs,daemon/crates/daemon-core/src/observability.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-002 add alias parameter-mapping types and conversion functions
- **Commit:** 0104cc4
- **Date:** 2026-04-09
- **Files:** daemon/crates/mcp-tools/src/aliases.rs,daemon/crates/mcp-tools/src/lib.rs

## chore(3.1.0): T-007 extract review and gemini_query handlers to mcp-tools
- **Commit:** 2f8373c
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/mcp-tools/Cargo.toml,daemon/crates/mcp-tools/src/gemini_query.rs,daemon/crates/mcp-tools/src/lib.rs,daemon/crates/mcp-tools/src/review.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-005 extract fleet handlers to mcp-tools/fleet.rs
- **Commit:** e98f347
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/mcp-tools/Cargo.toml,daemon/crates/mcp-tools/src/fleet.rs,daemon/crates/mcp-tools/src/lib.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-003 extract inter-agent handlers to mcp-tools/inter_agent.rs
- **Commit:** 3925f31
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/mcp-tools/Cargo.toml,daemon/crates/mcp-tools/src/inter_agent.rs,daemon/crates/mcp-tools/src/lib.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-006 extract knowledge handlers to mcp-tools/knowledge.rs
- **Commit:** 813a7fd
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/mcp-tools/Cargo.toml,daemon/crates/mcp-tools/src/knowledge.rs,daemon/crates/mcp-tools/src/lib.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-004 extract ABE handlers to mcp-tools/abe.rs
- **Commit:** 4afe753
- **Date:** 2026-04-09
- **Files:** daemon/crates/mcp-tools/src/abe.rs,daemon/crates/mcp-tools/src/lib.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-004B add multi-channel worker completion detection
- **Commit:** 4f21bb1
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/mcp-tools/Cargo.toml,daemon/crates/mcp-tools/src/abe.rs,daemon/crates/shared-types/src/abe.rs,daemon/crates/triumvirate/src/abe/task_tracker.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-008 extract HTTP route handlers to daemon-http
- **Commit:** 3821b9c
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/daemon-http/Cargo.toml,daemon/crates/daemon-http/src/lib.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-009 extract WebSocket, metrics, dashboard routes and DaemonState
- **Commit:** 39f0a6a
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/daemon-core/src/lib.rs,daemon/crates/daemon-http/Cargo.toml,daemon/crates/daemon-http/src/lib.rs,daemon/crates/triumvirate/src/main.rs

## chore(3.1.0): T-011 register 10 alias tools in tool_router with parameter mapping
- **Commit:** 0e12c44
- **Date:** 2026-04-09
- **Files:** daemon/Cargo.lock,daemon/crates/mcp-tools/Cargo.toml,daemon/crates/mcp-tools/src/aliases.rs,daemon/crates/triumvirate/src/main.rs

