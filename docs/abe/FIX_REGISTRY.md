# ABE v3.0 — Fix Registry

**Source:** Code review by Claude (orchestrator) + Gemini (auditor). Codex recused.
**Date:** 2026-04-08
**Instruction:** PAUSE new task development. Fix these in order before resuming T-013+.

---

## CRITICAL — Blocks all dispatch

### FIX-C1: worktree_setup.rs — `.git` is a file in worktrees, not a directory

**File:** `daemon/crates/triumvirate/src/abe/worktree_setup.rs`
**Function:** `ensure_exclude_entry`
**Line:** `let info_dir = worktree_path.join(".git").join("info");`

**Bug:** In a git worktree, `.git` is a FILE containing `gitdir: /path/to/repo/.git/worktrees/<name>`. It is NOT a directory. `fs::create_dir_all` on a file path panics with "Not a directory."

**Fix:** Read the `.git` file, parse the `gitdir:` path, then use THAT path to find `info/exclude`:
```rust
fn resolve_git_dir(worktree_path: &Path) -> PathBuf {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_file() {
        // worktree: .git is a file with "gitdir: <path>"
        let content = fs::read_to_string(&dot_git).unwrap_or_default();
        if let Some(gitdir) = content.strip_prefix("gitdir: ") {
            return PathBuf::from(gitdir.trim());
        }
    }
    dot_git // fallback: main repo
}
```
Then `ensure_exclude_entry` uses `resolve_git_dir(worktree_path).join("info")`.

**Test:** Create a worktree. Verify `.git` is a file. Call `setup_worktree`. Verify it doesn't panic and `.triumvirate/` is in the exclude list.

---

### FIX-C2: orchestrator.rs — `parse_plan` ignores contract enforcement fields

**File:** `daemon/crates/triumvirate/src/abe/orchestrator.rs`
**Function:** `parse_plan`

**Bug:** Only extracts `id`, `wave`, `req`, `description` from task XML. Ignores: `allowed_files`, `forbidden_files`, `allowed_commands`, `forbidden_commands`, `commit_format`, `test_command`, `task_timeout_sec`, `done_when`, `reality_test`, `scope_out`, `tools`. The `contract_fields` passed to worktree setup will be empty/default, rendering the entire enforcement stack inert.

**Fix:** `PlanTask` struct must include all ContractFields. `parse_plan` must extract `<files>`, `<scope_out>`, `<tools>`, `<verify>`, `<reality_test>`, `<done_when>` from between the task XML tags. Use the existing `extract_between` helper for each field. Parse `allowed_files` from `<files>` (comma-separated). Parse `allowed_commands` from `<tools>`.

**Test:** Parse a task XML with all 8 mandatory fields. Verify every field is populated in the returned PlanTask. Parse a task missing `<reality_test>` — verify it returns an error.

---

## HIGH

### FIX-H1: orchestrator.rs — Hardcoded timestamps

**File:** `daemon/crates/triumvirate/src/abe/orchestrator.rs`
**Lines:** ~105, ~113

**Bug:** `append_manifest` and `append_deviation` calls use `"2026-04-08T00:00:00Z"`. Every entry gets the same timestamp.

**Fix:** Use `chrono::Utc::now().to_rfc3339()` or accept a timestamp parameter. Add `chrono` to dependencies if not present.

---

### FIX-H2: codex_spawn.rs — SIGKILL instead of SIGTERM

**File:** `daemon/crates/triumvirate/src/abe/codex_spawn.rs`
**Function:** `enforce_timeout`
**Line:** `let _ = child.start_kill();`

**Bug:** `tokio::process::Child::start_kill()` sends SIGKILL on Unix. Spec requires SIGTERM first, 10s grace, then SIGKILL.

**Fix:**
```rust
// Send SIGTERM first
unsafe { libc::kill(child.id().unwrap() as i32, libc::SIGTERM); }
tokio::time::sleep(Duration::from_secs(10)).await;
// If still alive, SIGKILL
if child.try_wait()?.is_none() {
    let _ = child.kill().await;
}
```
Add `libc` to dependencies. Or use `nix::sys::signal::kill(Pid, Signal::SIGTERM)`.

---

### FIX-H3: failure_handler.rs — Wrong default classification

**File:** `daemon/crates/triumvirate/src/abe/failure_handler.rs`
**Lines:** 36-39

**Bug:** Default fallback is `WorkerError`. Spec says `OrchestratorBriefingError` (conservative — forces Gemini review).

**Fix:** Change the final return to:
```rust
Classification {
    class: FailureClass::OrchestratorBriefingError,
    reason: "unclassified failure — conservative default, send to Gemini".to_string(),
}
```

---

### FIX-H4: orchestrator.rs — FuturesUnordered achieves zero concurrency

**File:** `daemon/crates/triumvirate/src/abe/orchestrator.rs`
**Lines:** 78-93

**Bug:** The future pushed to `FuturesUnordered` is `async move { (task, ticket) }` which resolves instantly. `wait_task` is called sequentially after each pop. No concurrent monitoring.

**Fix:** Push the FULL dispatch+wait cycle as the future:
```rust
running.push(async move {
    let ticket = backend.dispatch_task(&task).await?;
    let result = backend.wait_task(&ticket).await?;
    Ok::<_, anyhow::Error>((task, result))
});
```
Then `running.next().await` actually waits for real completion concurrently.

---

### FIX-H5: Pre-commit hook reads wrong commit message

**File:** `daemon/crates/triumvirate/src/abe/worktree_setup.rs` (embedded hook)
**Line:** `msg=$(git log -1 --pretty=%B 2>/dev/null || true)`

**Bug:** `pre-commit` hook fires BEFORE the commit exists. `git log -1` returns the PREVIOUS commit's message. Commit message validation must happen in a `commit-msg` hook where `$1` is the message file.

**Fix:** Split into two hooks:
- `pre-commit`: file scope + stub markers (no message check)
- `commit-msg`: message format check using `cat "$1"`

Or move entirely to a `commit-msg` hook that does all checks.

---

### FIX-H6: codex_spawn.rs — False commit data on failure

**File:** `daemon/crates/triumvirate/src/abe/codex_spawn.rs`
**Function:** `resolve_commit_outputs`

**Bug:** If the worker never committed, this returns the base commit SHA and its files. False data enters BUILD_MANIFEST.

**Fix:** Compare HEAD against the worktree's starting SHA. If they're the same, the worker made no commit — return empty/error, not the base data.

---

### FIX-H7: Pre-commit hook missing compilation check

**File:** `daemon/crates/triumvirate/src/abe/worktree_setup.rs` (embedded hook)

**Bug:** Hook checks format, file scope, stubs. Missing: fast test command (compilation check) from `contract.json.test_command`.

**Fix:** Add after stub check:
```bash
test_cmd=$(jq -r '.test_command // empty' "$contract")
if [[ -n "$test_cmd" ]]; then
  if ! eval "$test_cmd" >/dev/null 2>&1; then
    echo "BLOCKED: test command failed: $test_cmd"
    exit 1
  fi
fi
```

---

## MEDIUM

### FIX-M1: wave_gate.rs — Missing test suite + Gemini review

**Bug:** Only validates overlap + status. REQ-A2.6 requires: collect validations, Gemini review of wave code, full test suite on merged state, block on failure, write wave summary.

**Fix:** `gate_wave` needs additional parameters: test_command (string), gemini_review callback. Run test suite. Call Gemini. Return structured result with summary.

---

### FIX-M2: task_tracker.rs — Cancel doesn't clean git locks

**Bug:** `cancel` calls `start_kill()` but doesn't remove `.git/index.lock`.

**Fix:** After killing, call the same lock cleanup as `enforce_timeout`.

---

### FIX-M3: Already covered by FIX-C2.

---

## LOW

### FIX-L1: Pre-commit hook uses `rg` not `grep`

**Fix:** Replace `rg -n` with `grep -rn` or check `command -v rg` with fallback.

---

### FIX-L2: Manifest table missing columns

**Fix:** Add Wave, Files Modified, Attempts columns to `append_manifest` header and row format.

---

## Execution Order

Fix in this order (dependencies):
1. **FIX-C1** (worktree .git file) — unblocks all dispatch
2. **FIX-C2** (parse contract fields) — unblocks all enforcement
3. **FIX-H5** (commit-msg hook split) — unblocks commit validation
4. **FIX-H2** (SIGTERM before SIGKILL) — timeout safety
5. **FIX-H3** (default classification) — failure handling correctness
6. **FIX-H4** (concurrent dispatch) — wave parallelism
7. **FIX-H6** (false commit data) — manifest integrity
8. **FIX-H7** (compilation check in hook) — enforcement completeness
9. **FIX-H1** (timestamps) — audit trail accuracy
10. **FIX-M1** (wave gate) — boundary enforcement
11. **FIX-M2** (cancel locks) — cleanup safety
12. **FIX-L1** (rg→grep) — portability
13. **FIX-L2** (manifest columns) — format compliance

After all fixes: resume T-013, T-014, T-016, T-021, T-022.
