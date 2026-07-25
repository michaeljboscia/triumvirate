# ABE v3.0 — Fix Registry v2 (Smoke Test Failures)

**Source:** Smoke test against production daemon (Apr 8)
**Root Cause:** `dispatch_codex` and `dispatch_codex_worktree` handlers use `std::env::current_dir()` for project_root. The daemon's cwd is `/Users/you` (home dir), not a git repo. All existing tools (fleet, peer review, ledger) have a `project_root` field in their request structs with fallback to `current_dir()`. The ABE tools don't follow this pattern.

---

## FIX-S1: dispatch_codex_worktree uses current_dir() instead of caller-provided project_root

**File:** `daemon/crates/triumvirate/src/main.rs`
**Line:** 872

**Bug:**
```rust
let project_root = std::env::current_dir()
    .map_err(|e| format!("failed to resolve project_root: {e}"))?;
```

The daemon runs from `/Users/you`. This is not a git repo. `git worktree add` fails because there's no `.git` in the home directory. Manual `git worktree add` from the correct directory succeeds.

**Fix:** Follow the existing pattern used by fleet/peer-review tools (lines 1112-1115):
1. Add `project_root: Option<String>` to `DispatchCodexWorktreeRequest` in shared-types
2. In the handler, resolve project_root from the request first, fall back to current_dir:
```rust
let project_root = req.project_root
    .map(PathBuf::from)
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| "no project_root provided and current_dir failed".to_string())?;
```

**Test:** Call `dispatch_codex_worktree` with `project_root: "/Users/you/projects/triumvirate"` — worktree creation succeeds.

---

## FIX-S2: dispatch_codex has no cwd resolution

**File:** `daemon/crates/triumvirate/src/main.rs`
**Scope:** The dispatch_codex handler (non-worktree variant)

**Bug:** Same root cause — Codex subprocess spawns with daemon's cwd, not the target project. The `DispatchCodexRequest` schema already has an optional `cwd` field but the handler may not be using it for the git context / subprocess working directory.

**Fix:** Ensure the handler uses `req.cwd` (if provided) as the subprocess working directory. If `req.cwd` is None, fall back to `current_dir()`. Verify the Codex subprocess is spawned with `current_dir(req.cwd)`.

**Test:** Call `dispatch_codex` with `cwd: "/Users/you/projects/triumvirate"` and a simple prompt — task completes successfully.

---

## FIX-S3: MCP schema for dispatch_codex_worktree missing project_root parameter

**File:** `daemon/crates/shared-types/src/abe.rs`
**Struct:** `DispatchCodexWorktreeRequest`

**Bug:** The MCP schema (generated from the struct) doesn't include a `project_root` field. The caller (Claude) has no way to tell the daemon which repo to target.

**Fix:** Add to the struct:
```rust
pub project_root: Option<String>,
```

This makes it optional in the MCP schema (backward compatible). When provided, it overrides current_dir(). When omitted, falls back to current_dir().

---

## FIX-S4: get_task_output untested on successful task

**Severity:** LOW — blocked by FIX-S1/S2 (can't get a successful task without working dispatch)

**Action:** After FIX-S1 and FIX-S2 are applied, re-run smoke test and verify get_task_output returns commit_sha + modified_files on a completed task.

---

## Execution Order

1. FIX-S3 (add project_root to shared-types schema)
2. FIX-S1 (use it in dispatch_codex_worktree handler)
3. FIX-S2 (verify dispatch_codex uses req.cwd)
4. `cargo build --release`
5. Restart Claude Code session (MCP bridge picks up new binary)
6. Re-run full smoke test (9 tests)

After fixes: FIX-S4 resolves automatically when dispatch works.

---

## Implementation Update (Apr 8, 2026)

The following follow-up issues were fixed in `daemon/crates/triumvirate/src/abe/` and validated with full workspace tests:

### FIX-S5: Failure classification now explicitly recognizes worker failures

**File:** `daemon/crates/triumvirate/src/abe/failure_handler.rs`

**Change:** Added explicit `WorkerError` matches before fallback for:
- `"stub marker"`
- `"test command failed"`
- `"validation failed"`

Default fallback remains `OrchestratorBriefingError` for conservative escalation.

### FIX-S6: Git lock cleanup now resolves worktree gitdir

**Files:**
- `daemon/crates/triumvirate/src/abe/codex_spawn.rs`
- `daemon/crates/triumvirate/src/abe/task_tracker.rs`

**Change:** `index.lock` cleanup now resolves the real gitdir when `.git` is a file (`gitdir: ...`) in linked worktrees, matching worktree semantics.

### FIX-S7: Plan parser enforces required task metadata

**File:** `daemon/crates/triumvirate/src/abe/orchestrator.rs`

**Change:** `parse_task_block` now fails fast if required fields are missing/invalid:
- `wave` attribute (must exist and parse to `u32`)
- `req` attribute (non-empty)
- `<description>` (non-empty)
- `<scope_out>` (non-empty)

### FIX-S8: Build state finalized at 22/22 complete

**File:** `BUILD_STATE.json`

**Change:** Confirmed completion state:
- `tasks_completed`: `T-001` through `T-022`
- `tasks_remaining`: `[]`

### Validation

- Command: `cargo test --workspace` (from `daemon/`)
- Result: all tests passed, including new ABE acceptance coverage.
