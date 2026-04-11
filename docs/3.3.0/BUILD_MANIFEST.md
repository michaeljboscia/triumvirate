# BUILD_MANIFEST — v3.3.0 Live Agent Streaming

**Started:** 2026-04-11
**Branch:** v3.3.0
**Base SHA:** 82c3bec4d4d7dcab0266548f503de475254f5631

## Wave 0: Contracts + Types

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-300 | e1f0936 | shared-types/src/streaming.rs (new), shared-types/src/lib.rs, daemon-core/src/sequencer.rs (new), daemon-core/src/lib.rs | 17/17 (shared-types + daemon-core) | DONE |
| T-301 | 5c47f9f | triumvirate/src/streaming.rs (new), triumvirate/src/main.rs | 30/30 (shared-types + daemon-core) | DONE |

**Wave 0 deviations:** None. AskAgentResponse lacks token_usage field — T-301 TurnCompleted emits zeros. Wave 1 parsers provide real data.

## Wave 1: Event Pipeline

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-302 | a9c24b8 | agent-adapter/src/gemini.rs, agent-adapter/Cargo.toml | 15/15 (agent-adapter) | DONE |
| T-303 | 1c4a4e5 | agent-adapter/src/codex.rs | 15/15 (agent-adapter) | DONE |
| T-304 | cf58e5e | daemon-core/src/observability.rs | 15/15 (daemon-core) | DONE |

**Wave 1 deviations:** Added shared-types as dependency to agent-adapter (not in original task files list for T-302 — required for AgentStreamEvent import). Collateral: Cargo.toml change.

## Wave 2: Streamable HTTP Transport

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-305 | 2618151 | http_mcp.rs (new), main.rs, Cargo.toml x2 | 67/67 (4 crates) | DONE |
| T-306 | dd951ff | http_mcp.rs (auth middleware + router), main.rs | 67/67 (no regressions) | DONE |
| T-307 | b9425ca + 0d86162 (fix) | tests/integration_streaming.rs (5 tests), http_mcp.rs (fallback_service fix) | 5/5 pass against live v3.3.0 daemon | DONE |

## Wave 3: Proxy + Watch CLI

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-308 | — | — | — | PENDING |
| T-309 | — | — | — | PENDING |

## Wave 4: Spike + Polish

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-310 | — | — | — | PENDING |
| T-312 | — | — | — | IN PROGRESS (this file) |
| T-311 | — | — | — | PENDING |
