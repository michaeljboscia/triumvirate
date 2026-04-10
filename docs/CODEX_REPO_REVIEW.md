# Triumvirate Codebase Quality Review
**Reviewer:** Claude (standing in for Codex after timeout)
**Date:** 2026-04-10

## 1. Cargo.toml Metadata — FAIL
Missing fields that crates.io and GitHub expect:
```toml
# Currently set:
version = "3.2.0"
edition = "2024"
license = "MIT"

# MISSING — add to [workspace.package]:
description = "Multi-agent AI coordination daemon for Claude, Gemini, and Codex"
repository = "https://github.com/michaeljboscia/triumvirate"
homepage = "https://github.com/michaeljboscia/triumvirate"
keywords = ["ai", "mcp", "agents", "llm", "orchestration"]
categories = ["command-line-utilities", "development-tools"]
```

## 2. CI/CD — FAIL
No `.github/workflows/` directory exists. For a public Rust project this is the bare minimum:
- `rust.yml`: `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt --check`
- `release.yml`: Build cross-platform binaries on tag push (aarch64-apple-darwin, x86_64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu)
- Badge in README won't work without the workflow file

## 3. LICENSE — PASS
MIT license file exists at repo root. Cargo.toml declares `license = "MIT"`. Consistent.

## 4. CONTRIBUTING.md — PASS (needs update)
File exists. Should mention the `/goatrodeo` methodology and the ABE fleet dispatch system for large contributions.

## 5. Public API Documentation — WARN
The public-facing types in `shared-types/src/lib.rs` and `shared-types/src/abe.rs` are the contract surface for MCP tools. Most structs have `#[derive(JsonSchema)]` but no `///` doc comments. Top 10 undocumented items that external users would encounter:
1. `MemoryWriteRequest` — what is namespace vs key vs value?
2. `DispatchCodexWorktreeRequest` — the main ABE entry point, zero docs
3. `ContractFields` — the ABE contract schema, 12 fields, none documented
4. `TaskStatus` enum — what triggers each state transition?
5. `FleetSpawnRequest` — how does fleet dispatch work?
6. `GetTaskStatusResponse` — what do elapsed_sec and exit_code mean?
7. `LessonListRequest` — what are tags and stale_days?
8. `HealthStatus` (ledger) — what makes status "healthy" vs not?
9. `OutboxEvent` — what events end up here?
10. `TokenRecord` — the token economics data model

## 6. Architecture Documentation — WARN
`docs/how-it-all-fits-together.md` and `docs/plain-english-guide.md` exist but neither mentions the 13-crate workspace structure. A developer doing `tree daemon/crates/` would see 13 directories with no guide to what each one does. Need a crate map:

| Crate | Purpose |
|-------|---------|
| triumvirate | Binary + CLI + MCP server + HTTP router |
| daemon-core | DaemonMetrics, ObservabilityBus, VERSION |
| daemon-http | Axum HTTP route handlers |
| mcp-tools | MCP tool implementations (abe, fleet, knowledge, review) |
| mcp-bridge | rmcp McpBridge delegator |
| shared-types | All request/response structs + JsonSchema |
| token-economics | SQLite storage + scanner + attribution |
| agent-adapter | Agent subprocess spawn + stream parsing |
| agent-worker | Worker process lifecycle |
| fleet | Fleet orchestrator + task store |
| ledger | SQLite event ledger + lessons |
| peer-review | Cross-agent code review queue |
| fallback-outbox | Offline message queueing |

## 7. Open Issues Cleanup — ACTION NEEDED
- **#21:** Close immediately (v3.0.1 shipped)
- **#12:** Relabel from `v3.1` to `backlog` or `v3.3`
- **#16:** Add `good first issue` label
- **#15:** Add blocking context comment
