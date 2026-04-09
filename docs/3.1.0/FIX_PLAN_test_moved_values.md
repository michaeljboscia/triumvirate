# Fix Plan: Test Compilation Errors (Pre-Wave 0 Cleanup)

**Task ID:** FIX-TEST-MOVED-VALUES
**Target agent:** Codex
**Type:** Pre-existing bug fix (not part of v3.1 sprint scope — unblocks Wave 0)
**Estimated effort:** 5-15 lines changed, 1 file
**Must pass before:** Wave 0 of v3.1 MCP Consolidation sprint

---

## Problem

`cargo test --workspace` fails to compile with 3 `E0382` errors in the `abe_red_team_enforcement_blocks_non_compliant_worker` test function at `daemon/crates/triumvirate/src/main.rs:6036`.

The errors:
```
error[E0382]: use of moved value: `dispatch_and_expect_failed`
  --> crates/triumvirate/src/main.rs:6166:18
error[E0382]: use of moved value: `dispatch_and_expect_failed`
  --> crates/triumvirate/src/main.rs:6168:18
error[E0382]: borrow of moved value: `project_root`
  --> crates/triumvirate/src/main.rs:6114:40
```

## Root Cause

At `main.rs:6103`, a closure `dispatch_and_expect_failed` is defined:

```rust
let dispatch_and_expect_failed = |script_path: PathBuf, task_id: String| {
    let bridge = bridge.clone();
    let head_sha = head_sha.clone();
    async move {
        // ... uses project_root by move (line 6114) ...
        let dispatched = bridge
            .dispatch_codex_worktree(Parameters(DispatchCodexWorktreeRequest {
                project_root: Some(project_root.display().to_string()),
                // ...
            }))
            // ...
    }
};
```

The closure captures `project_root: PathBuf` from the enclosing scope. Because `project_root.display().to_string()` borrows `project_root` inside an `async move` block, the closure becomes `FnOnce` (the `async move` moves `project_root` into the future).

Then at lines 6164, 6166, 6168, the closure is called three times:

```rust
let s1 = dispatch_and_expect_failed(forbidden_file_script.clone(), "T-016A".to_string()).await?;
let s2 = dispatch_and_expect_failed(bad_commit_script.clone(), "T-016B".to_string()).await?;
let s3 = dispatch_and_expect_failed(stub_script.clone(), "T-016C".to_string()).await?;
```

The first call consumes the closure (and `project_root` via the `async move`). The second call fails: the closure has already been moved. Hence `use of moved value: dispatch_and_expect_failed` on lines 6166 and 6168.

The `borrow of moved value: project_root` error at 6114 is the compiler pointing at the line inside the closure where the move-borrow conflict originates.

Additionally, at line 6212 after the three closure calls:

```rust
let _ = fs::remove_dir_all(project_root);
```

`project_root` is used again by value — this is fine in isolation but the compiler's analysis chains the errors together.

## Fix

Clone `project_root` into the closure scope so each call gets its own owned copy, matching the existing pattern already used for `bridge` and `head_sha`:

### Change 1: Line 6103-6106 region

**Before:**
```rust
let dispatch_and_expect_failed = |script_path: PathBuf, task_id: String| {
    let bridge = bridge.clone();
    let head_sha = head_sha.clone();
    async move {
```

**After:**
```rust
let dispatch_and_expect_failed = |script_path: PathBuf, task_id: String| {
    let bridge = bridge.clone();
    let head_sha = head_sha.clone();
    let project_root = project_root.clone();
    async move {
```

That single added line (`let project_root = project_root.clone();`) is the entire fix. The closure still captures `project_root` by reference from the outer scope, but now clones it on every invocation before moving the clone into the async block. The outer `project_root` remains valid for the `fs::remove_dir_all(project_root)` call at line 6212.

## Verification Steps

1. Apply the one-line change above
2. Run `cargo check --workspace` — should report zero errors
3. Run `cargo test --workspace` — should compile successfully (test may still take time to run)
4. Run the specific test to verify it works: `cargo test --package triumvirate abe_red_team_enforcement_blocks_non_compliant_worker -- --ignored` (check if `#[ignore]` is needed based on test attribute)

## Scope Discipline

- **DO** change exactly the one region in the closure definition
- **DO NOT** refactor the test to use a different pattern
- **DO NOT** touch any other test functions
- **DO NOT** modify the `dispatch_codex_worktree` call
- **DO NOT** change the test logic or assertions
- **DO NOT** address any other warnings or issues you may notice

## Commit Format

```
fix(tests): resolve moved-value errors in abe_red_team test

The dispatch_and_expect_failed closure captured project_root by move
via the inner async block, causing FnOnce semantics. The closure is
called 3 times in sequence, so clone project_root inside the closure
body alongside the existing bridge and head_sha clones.

Unblocks v3.1 MCP Consolidation Wave 0 which requires cargo test
--workspace to pass as a baseline.
```

## Files Changed

- `daemon/crates/triumvirate/src/main.rs` — one line added in the closure definition

No other files should be modified.

## Acceptance

- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` compiles (test suite runs to completion)
- [ ] The specific test `abe_red_team_enforcement_blocks_non_compliant_worker` compiles
- [ ] Git diff shows only the one-line change in `main.rs`
- [ ] Commit message matches the format above
