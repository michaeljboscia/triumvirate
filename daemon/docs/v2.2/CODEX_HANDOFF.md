# Codex Handoff — Triumvirate v2.2 Build

**Branch:** `feat/mcp-first` at commit `63579d8`
**Date:** 2026-04-07

---

## What You're Building

Triumvirate v2.2 "The Accountability Release." 9 features, 4 new Rust crates, 102 REQs. The spec survived an 8-round goat rodeo with twin review every round.

## Read These First (In Order)

1. `daemon/docs/v2.2/SPEC.md` — Source of truth. 102 REQs. Read the whole thing.
2. `daemon/docs/v2.2/IMPLEMENTATION_PLAN.md` — 71 tasks with XML `<task>` blocks. Work through them IN ORDER.
3. `daemon/docs/v2.2/BACKEND_STRUCTURE.md` — SQLite schema, traits, HTTP endpoints.
4. `daemon/docs/v2.2/TECH_STACK.md` — Dependencies, env vars, build pipeline.
5. `daemon/docs/v2.2/TEST_PLAN.md` — Reality tests per REQ. Every task has one.

## How To Work

### Phase Order (STRICT)

```
Phase 0.5 → Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6 → Phase 7
```

Within each phase, work wave by wave. Wave 0 is always contracts/interfaces. Wave N+1 cannot start until Wave N is complete.

### Per-Task Process

For every task in IMPLEMENTATION_PLAN.md:

1. Read the `<task>` XML block
2. Implement it
3. Run the `<verify>` command (compilation check)
4. Run the `<reality_test>` (behavioral check — a stub CANNOT pass this)
5. If reality test fails, the task is NOT done. Fix it.
6. `git commit` with task ID in message: `feat(ledger): T-104 implement LedgerStore::open with WAL mode`
7. Move to next task

### Rules You Must Follow

- **`ledger` crate takes absolute PathBuf only.** Never resolve project roots inside the ledger crate (REQ-002a).
- **`fleet` crate uses GitOps trait.** Never shell out to `git` directly (REQ-003). The real GitOps impl goes in `triumvirate/src/git_ops_impl.rs`.
- **All MCP tool DTOs in `shared-types`.** Crates provide logic, binary provides wiring (REQ-005).
- **All logging via `tracing` macros.** Zero `println!` or `eprintln!` (REQ-062).
- **Hooks use POSIX builtins only.** `date`, `$$`, `$RANDOM`, `curl`. No `uuidgen`, `sqlite3`, `python3` (REQ-010).
- **Spool is a directory, not a file.** Atomic rename pattern: `.tmp` → `.ndjson` (REQ-010).
- **Dashboard requires DESIGN_SYSTEM.md approval before dev.** It's at `dashboard/DESIGN_SYSTEM.md` (REQ-048).

### New Crates to Create

| Crate | Path | Depends On |
|-------|------|-----------|
| `ledger` | `daemon/crates/ledger/` | shared-types, agent-adapter |
| `fleet` | `daemon/crates/fleet/` | shared-types, daemon-core, agent-worker, ledger |
| `peer-review` | `daemon/crates/peer-review/` | shared-types, ledger |
| `dashboard` | `dashboard/` (Svelte, not a Rust crate) | shared-types (build-time) |

Add new crates to the workspace `Cargo.toml` members list.

### Key Architecture Decisions

These were made during the goat rodeo. Do not deviate without documenting in DEVIATION_LOG.md.

- **Spool-first ingestion.** Hooks write to `<project>/.triumvirate/spool/` directory. Daemon drains async. The hook cannot fail.
- **Daemon is the single SQLite writer.** No bash hook touches the DB.
- **Parallel reviews, sequential merge.** Reviews start when agents finish. Merge checks approval before each step.
- **Fleet task file is untracked.** `.triumvirate/fleet-task.md` is runtime metadata. `.triumvirate/` is in `.gitignore` (Ledger Phase 1 owns this).
- **dry_run=true is the default for fleet_spawn.** Safety first.
- **No local execution in MCP bridge.** Daemon-proxy only. Kill the local fallback path.
- **Confidence decay at query time.** Not background mutation.
- **Codex auto-approve via --full-auto flag.** Not JSON-RPC response (broken as of early 2026).

### Phase Gates

After each phase, verify the gate before proceeding:

| Phase | Gate |
|-------|------|
| 0.5 | `curl /metrics` returns all 9 metric names. `RUST_LOG=debug` produces JSON spans. Zero println. |
| 1 | `triumvirate doctor` green. Hook → spool → drain → SQLite → health = healthy. |
| 2 | `ledger_query("test")` returns results. Lesson confidence decay correct. |
| 3 | 2-agent fleet spawns, claims, works, merges. Crash recovery works. |
| 4 | Self-review rejected. Fleet merge blocked without approval. Skip flag works. |
| 5 | Dashboard at localhost:8080. All 6 views. Health indicator green. |
| 6 | Codex app-server connects. Auto-approve fires. OutboxEvent has token_usage. |
| 7 | GC deletes stale events. Active fleet blocks GC. |

### If You Get Stuck

- Read the SPEC.md REQ for the task you're on
- Read the TEST_PLAN.md reality test for that REQ
- If you need to deviate from the plan, write it in DEVIATION_LOG.md with WHY
- Do not skip reality tests. A passing `cargo test` with no reality test is not done.

### When You're Done

Run `cargo test --workspace` + `cargo clippy --workspace`. All green = ready for postrodeo audit.
