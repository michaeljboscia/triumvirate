# ABE v3.0 — Stress Test Plan

**Date:** 2026-04-08
**Goal:** Find where the system breaks before users do. Scale from 1→2→4→8 concurrent Codex workers. Test happy paths, failure paths, race conditions, resource exhaustion, and recovery.
**Philosophy:** Failing now is good. Every edge case found here is a production incident avoided.

---

## Test Repository

Create a dedicated test repo — do NOT run these against the triumvirate repo itself. A test failure that corrupts the main repo would be catastrophic.

```bash
mkdir -p ~/projects/abe-stress-test
cd ~/projects/abe-stress-test
git init
git config extensions.worktreeConfig true
echo "# ABE Stress Test Repo" > README.md
mkdir -p src
echo "fn main() { println!(\"hello\"); }" > src/main.rs
echo '[package]\nname = "abe-stress"\nversion = "0.1.0"\nedition = "2021"' > Cargo.toml
git add -A && git commit -m "init: stress test repo"
```

---

## Phase 1: Single Worker (1 Codex + 1 Gemini)

### Test 1.1: Happy Path — Create + Commit + Review
**Purpose:** Baseline. Prove the full loop works with one worker.

```
dispatch_codex_worktree:
  task_id: STRESS-001
  briefing: "Create src/hello.rs with fn hello() -> &str { \"world\" }. Commit."
  allowed_files: [src/hello.rs]
  test_command: "cargo check"
  task_timeout_sec: 120
```

**Verify:**
- [ ] get_task_status → completed
- [ ] get_task_output → commit SHA + modified files = [src/hello.rs]
- [ ] query_gemini_review(diff, mode=pass) → returns verdict
- [ ] File content correct in worktree
- [ ] Worktree can be merged to main

### Test 1.2: Forbidden File — Worker Tries to Write Outside Scope
**Purpose:** Test pre-commit hook enforcement.

```
dispatch_codex_worktree:
  task_id: STRESS-002
  briefing: "Create src/hello.rs AND modify README.md to say 'modified by worker'."
  allowed_files: [src/hello.rs]  # README.md NOT allowed
  forbidden_files: [README.md]
  test_command: "cargo check"
  task_timeout_sec: 120
```

**Expected:** Worker creates both files but pre-commit hook BLOCKS the commit because README.md is not in allowed_files.
**Verify:**
- [ ] get_task_status → failed (commit rejected by hook)
- [ ] README.md is NOT modified in the worktree's git history
- [ ] Error message contains "BLOCKED" and names README.md

### Test 1.3: Stub Marker — Worker Leaves TODO
**Purpose:** Test stub detection in pre-commit hook.

```
dispatch_codex_worktree:
  task_id: STRESS-003
  briefing: "Create src/stub.rs with a function that has a TODO comment inside."
  allowed_files: [src/stub.rs]
  test_command: "echo PASS"
  task_timeout_sec: 120
```

**Expected:** Pre-commit hook blocks commit due to stub marker.
**Verify:**
- [ ] get_task_status → failed
- [ ] Error contains "stub marker" or "BLOCKED"

### Test 1.4: Wrong Commit Format
**Purpose:** Test commit message enforcement.

```
dispatch_codex_worktree:
  task_id: STRESS-004
  briefing: "Create src/format.rs. Commit with message 'added format file' (deliberately wrong format)."
  allowed_files: [src/format.rs]
  commit_format: "^STRESS-004:"
  test_command: "echo PASS"
  task_timeout_sec: 120
```

**Expected:** Commit-msg hook rejects the commit because message doesn't start with STRESS-004:.
**Verify:**
- [ ] get_task_status → failed
- [ ] Error mentions commit format

### Test 1.5: Timeout — Worker Takes Too Long
**Purpose:** Test SIGTERM → grace → SIGKILL timeout enforcement.

```
dispatch_codex_worktree:
  task_id: STRESS-005
  briefing: "This is a very complex task. Think deeply about the architecture for several minutes before writing any code. Take your time."
  allowed_files: [src/slow.rs]
  test_command: "echo PASS"
  task_timeout_sec: 15  # Very short — designed to trigger timeout
```

**Expected:** Worker is killed after 15 seconds.
**Verify:**
- [ ] get_task_status → failed with status "timeout"
- [ ] No .git/index.lock left behind
- [ ] Worktree is in a usable state (not corrupted)

### Test 1.6: Cancel — Kill a Running Worker
**Purpose:** Test cancel_task mid-execution.

```
dispatch_codex_worktree:
  task_id: STRESS-006
  briefing: "Implement a comprehensive test suite for a sorting library. Be thorough."
  allowed_files: [src/sort.rs, src/sort_test.rs]
  test_command: "echo PASS"
  task_timeout_sec: 300
```

After 10 seconds: `cancel_task(STRESS-006)`

**Verify:**
- [ ] cancel returns status "cancelled"
- [ ] get_task_status → cancelled
- [ ] No .git/index.lock left behind
- [ ] No orphaned Codex process: `ps aux | grep codex | grep STRESS-006`

### Test 1.7: Gemini Failure Review — Send Failure Context
**Purpose:** Test Gemini gets briefing + contract on failure mode.

```
After STRESS-002 fails:
query_gemini_review:
  diff: (the diff from STRESS-002's worktree)
  mode: failure
  briefing: (STRESS-002's briefing)
  contract: (STRESS-002's contract fields)
  failure_details: "BLOCKED: Write to README.md denied by contract"
```

**Verify:**
- [ ] Returns verdict with diagnostic context
- [ ] Response references the contract violation (not just generic feedback)

### Test 1.8: Setup Failure — Invalid SHA
**Purpose:** Test daemon rollback on bad dispatch parameters.

```
dispatch_codex_worktree:
  sha: "0000000000000000000000000000000000000000"  # Nonexistent
  briefing: "This should never run."
  contract_fields: (valid)
```

**Expected:** SETUP_FAILED error, no leaked worktree.
**Verify:**
- [ ] Returns SETUP_FAILED
- [ ] No worktree directory created
- [ ] No orphaned processes

---

## Phase 2: Two Workers (2 Codex + N Gemini)

### Test 2.1: Parallel Happy Path — Two Independent Tasks
**Purpose:** Prove two workers can run simultaneously without interference.

```
Dispatch simultaneously:
  STRESS-201: Create src/alpha.rs with fn alpha() -> i32 { 1 }
    allowed_files: [src/alpha.rs]
  STRESS-202: Create src/beta.rs with fn beta() -> i32 { 2 }
    allowed_files: [src/beta.rs]
```

**Verify:**
- [ ] Both dispatched within 1 second of each other
- [ ] Both complete independently
- [ ] No file conflicts (different allowed_files)
- [ ] Both commits exist in their respective worktrees
- [ ] Gemini review on both diffs succeeds

### Test 2.2: Parallel With Shared Dependency — Same Base SHA
**Purpose:** Both workers branch from the same commit. Tests worktree isolation.

```
Dispatch from same SHA:
  STRESS-203: Create src/left.rs importing src/shared.rs
    allowed_files: [src/left.rs]
  STRESS-204: Create src/right.rs importing src/shared.rs
    allowed_files: [src/right.rs]
```

**Verify:**
- [ ] Both worktrees have identical starting state
- [ ] Neither worker can see the other's changes
- [ ] Both complete independently
- [ ] No git lock conflicts between worktrees

### Test 2.3: One Passes, One Fails — Mixed Results
**Purpose:** Prove failure in one worker doesn't affect the other.

```
Dispatch simultaneously:
  STRESS-205: Happy path (valid task, should succeed)
    allowed_files: [src/good.rs]
  STRESS-206: Designed to fail (allowed_files doesn't include the file it needs)
    briefing: "Create src/bad.rs AND src/sneaky.rs"
    allowed_files: [src/bad.rs]  # sneaky.rs not allowed
```

**Verify:**
- [ ] STRESS-205 completes successfully
- [ ] STRESS-206 fails (hook blocks sneaky.rs)
- [ ] STRESS-205's worktree is NOT affected by 206's failure
- [ ] get_task_status returns correct status for each

### Test 2.4: Race Condition — Both Workers Finish Simultaneously
**Purpose:** Test daemon's task tracker under concurrent completion.

```
Dispatch simultaneously with very short tasks:
  STRESS-207: "echo 'hello' > src/race1.rs && git add . && git commit -m 'STRESS-207: race'"
    allowed_files: [src/race1.rs]
    task_timeout_sec: 30
  STRESS-208: "echo 'world' > src/race2.rs && git add . && git commit -m 'STRESS-208: race'"
    allowed_files: [src/race2.rs]
    task_timeout_sec: 30
```

**Verify:**
- [ ] Both show "completed" (not one overwriting the other's state)
- [ ] Both commit SHAs are different
- [ ] TaskTracker has correct entries for both

### Test 2.5: Cancel One While Other Runs
**Purpose:** Cancelling one task doesn't affect the other.

```
Dispatch:
  STRESS-209: Long task (300s timeout)
  STRESS-210: Long task (300s timeout)

After 10 seconds: cancel_task(STRESS-209)
Let STRESS-210 continue.
```

**Verify:**
- [ ] STRESS-209: cancelled
- [ ] STRESS-210: still working (not cancelled)
- [ ] STRESS-210 eventually completes normally

---

## Phase 3: Four Workers (4 Codex + N Gemini)

### Test 3.1: Wave Simulation — 4 Parallel Tasks
**Purpose:** Simulate a real wave with max_parallel=4.

```
Dispatch all 4 simultaneously from same SHA:
  STRESS-301: Create src/w1.rs — fn worker1() -> &str { "one" }
  STRESS-302: Create src/w2.rs — fn worker2() -> &str { "two" }
  STRESS-303: Create src/w3.rs — fn worker3() -> &str { "three" }
  STRESS-304: Create src/w4.rs — fn worker4() -> &str { "four" }
  
All have disjoint allowed_files. All should succeed.
```

**Verify:**
- [ ] All 4 dispatched within 2 seconds
- [ ] All 4 running concurrently (get_task_status shows "working" for all)
- [ ] All 4 complete with different commit SHAs
- [ ] No git lock conflicts
- [ ] No worktree corruption
- [ ] Monitor system resources during execution: `ps aux | grep codex | wc -l` = 4

### Test 3.2: Resource Exhaustion — Memory Pressure
**Purpose:** 4 Codex processes + daemon + Claude session. How much RAM?

```
Same as 3.1, but monitor:
- Total RSS of all codex processes
- Daemon RSS
- System memory available
```

**Verify:**
- [ ] System doesn't swap
- [ ] No OOM kills
- [ ] All tasks complete despite memory pressure
- [ ] Record: baseline RSS per Codex worker

### Test 3.3: Mixed Success/Failure — 2 Pass, 1 Fail, 1 Timeout
**Purpose:** Complex mixed results with 4 workers.

```
Dispatch:
  STRESS-305: Happy path → should PASS
  STRESS-306: Happy path → should PASS
  STRESS-307: Forbidden file → should FAIL (hook blocks)
  STRESS-308: Timeout in 15s → should TIMEOUT
```

**Verify:**
- [ ] Correct status for each task
- [ ] Passing tasks unaffected by failing tasks
- [ ] Timed-out task leaves no locks
- [ ] TaskTracker state is consistent (2 completed, 1 failed, 1 timeout)

### Test 3.4: Worktree Cleanup — Verify No Leaks After 4 Tasks
**Purpose:** After all tasks finish, worktrees should be manageable.

```
After 3.1 completes:
  git worktree list
```

**Verify:**
- [ ] 4 worktrees exist (or cleaned up, depending on daemon behavior)
- [ ] No prunable/orphaned worktrees
- [ ] Disk space used by .triumvirate/abe-worktrees/ is reasonable

### Test 3.5: Blocker/Unblocker — Wave Dependency Simulation
**Purpose:** Simulate Wave 1 → Wave 2 dependency. Wave 2 tasks can't start until Wave 1 completes.

```
Wave 1 (dispatch immediately):
  STRESS-309: Create src/interface.rs with a trait definition
  STRESS-310: Create src/types.rs with type aliases

Wait for both to complete. Merge results to a new commit SHA.

Wave 2 (dispatch after Wave 1 merge):
  STRESS-311: Create src/impl.rs implementing the trait from interface.rs
  STRESS-312: Create src/consumer.rs using types from types.rs
```

**Verify:**
- [ ] Wave 2 workers see Wave 1's committed code in their worktrees
- [ ] Wave 2 workers can build against Wave 1 interfaces
- [ ] No stale state from Wave 1 leaking into Wave 2

---

## Phase 4: Eight Workers (8 Codex + N Gemini)

### Test 4.1: Maximum Concurrency — 8 Parallel Dispatches
**Purpose:** Push the system to its limits on a 16-core, 30GB machine.

```
Dispatch all 8 simultaneously:
  STRESS-401 through STRESS-408
  Each creates a unique file (src/s401.rs through src/s408.rs)
  All disjoint allowed_files
  All same base SHA
```

**Verify:**
- [ ] All 8 dispatched
- [ ] System remains responsive (can still call get_task_status)
- [ ] How many complete vs timeout vs fail?
- [ ] Record: wall-clock time for all 8 to finish
- [ ] Record: peak memory usage
- [ ] Record: peak CPU usage
- [ ] No filesystem errors (too many open files, etc.)

### Test 4.2: Git Object Contention — 8 Workers Writing to Shared .git/objects/
**Purpose:** git add requires writing to the shared object store. 8 concurrent git adds may contend on .git/objects/ locks.

```
Same as 4.1 but monitor:
  - Any "unable to create temporary file" errors
  - Any "index.lock" contention
  - git fsck after all complete
```

**Verify:**
- [ ] No git corruption: `git fsck --full` passes
- [ ] No dangling objects
- [ ] All 8 commits are valid

### Test 4.3: Half Succeed, Half Fail — Chaos Test
**Purpose:** Maximum disorder. Half the tasks are designed to fail in different ways.

```
Dispatch:
  STRESS-409: Happy path → PASS
  STRESS-410: Happy path → PASS
  STRESS-411: Happy path → PASS
  STRESS-412: Happy path → PASS
  STRESS-413: Forbidden file → FAIL
  STRESS-414: Stub marker → FAIL
  STRESS-415: Wrong commit format → FAIL
  STRESS-416: Timeout (15s) → TIMEOUT
```

**Verify:**
- [ ] 4 completed, 3 failed, 1 timeout
- [ ] TaskTracker state is fully consistent
- [ ] No cross-contamination between passing and failing tasks
- [ ] BUILD_STATE.json (if used) reflects correct tallies
- [ ] System is still responsive after all 8 finish

### Test 4.4: Rapid Fire Cancel — Cancel All 8 Immediately
**Purpose:** Test cancel under maximum load.

```
Dispatch 8 tasks.
Wait 5 seconds.
Cancel all 8 in rapid succession.
```

**Verify:**
- [ ] All 8 return "cancelled"
- [ ] No orphaned Codex processes: `ps aux | grep codex | grep -c STRESS`
- [ ] No .git/index.lock files left
- [ ] Daemon is still responsive after mass cancel

### Test 4.5: Dispatch While Tasks Running — Overflow Test
**Purpose:** What happens if you dispatch task 9 while 8 are running?

```
Dispatch 8 tasks (300s timeout each).
While all 8 are "working", dispatch task 9.
```

**Verify:**
- [ ] Does task 9 queue? Fail? Run as 9th concurrent?
- [ ] System behavior is predictable (not undefined)
- [ ] Document: what IS the actual concurrency limit?

---

## Phase 5: Recovery and Edge Cases

### Test 5.1: Daemon Restart Mid-Build
**Purpose:** Kill the daemon while workers are running. What happens?

```
Dispatch 2 tasks.
Wait for "working" status.
Kill daemon: kill $(pgrep -f "triumvirate daemon")
Restart daemon.
Check task states.
```

**Verify:**
- [ ] Are the Codex workers still alive? (They were spawned as children of the daemon)
- [ ] Can get_task_status recover state?
- [ ] Do orphaned worktrees get detected?

### Test 5.2: MCP Bridge Restart Mid-Build
**Purpose:** Kill the MCP bridge (but not daemon) while tasks are running.

```
Dispatch 2 tasks.
Wait for "working".
Kill MCP bridge: pkill -f "triumvirate mcp"
Reconnect (new Claude session or auto-respawn).
Poll get_task_status.
```

**Verify:**
- [ ] Tasks continue running (daemon is still alive)
- [ ] New MCP bridge can query task states
- [ ] No data loss

### Test 5.3: Disk Full Simulation
**Purpose:** What happens when .triumvirate/ can't be written?

```
Fill the temp space or set a tiny quota.
Dispatch a task.
```

**Verify:**
- [ ] Clean error message (not a panic)
- [ ] Rollback works (no partial worktree)
- [ ] Daemon stays alive

### Test 5.4: Invalid Contract — Malformed JSON
**Purpose:** What happens with garbage contract fields?

```
dispatch_codex_worktree:
  contract_fields: { "task_id": "", "allowed_files": [], ... }  # Empty task_id
```

**Verify:**
- [ ] Rejected before spawning Codex
- [ ] Meaningful error message
- [ ] No worktree created

### Test 5.5: Worktree Path Collision — Same Task ID Twice
**Purpose:** Dispatch the same task_id while a previous dispatch with that ID is still running.

```
Dispatch STRESS-DUPE with 300s timeout.
While STRESS-DUPE is working, dispatch STRESS-DUPE again.
```

**Verify:**
- [ ] Second dispatch fails cleanly (or waits, or overwrites — document the behavior)
- [ ] First task is not corrupted
- [ ] No undefined state

### Test 5.6: Extremely Long Briefing
**Purpose:** Test with a massive briefing document (100KB+).

```
dispatch_codex_worktree:
  briefing_content: (100KB of text — repeat a paragraph 1000 times)
  contract_fields: (valid)
```

**Verify:**
- [ ] Dispatch succeeds or fails cleanly (not hang/crash)
- [ ] If succeeds: Codex can read the briefing
- [ ] File write to .triumvirate/BRIEFING.md doesn't fail

---

## Metrics to Capture

Record these for every phase:

| Metric | How to Capture |
|--------|---------------|
| Wall-clock per task | build_started_at → completion timestamp |
| Peak RSS per Codex worker | `ps -o rss -p <pid>` sampled every 5s |
| Total system memory | `vm_stat` or `top -l 1` |
| Codex process count | `ps aux \| grep codex \| grep -v grep \| wc -l` |
| Git worktree count | `git worktree list \| wc -l` |
| Disk usage (.triumvirate/) | `du -sh .triumvirate/` |
| Open file descriptors | `lsof -p <daemon_pid> \| wc -l` |
| Git fsck result | `git fsck --full` after each phase |

---

## Pass Criteria

| Phase | Pass If |
|-------|---------|
| Phase 1 | All 8 tests produce expected results (pass or designed-failure) |
| Phase 2 | Parallel workers don't interfere. Mixed results handled correctly. |
| Phase 3 | 4 concurrent workers complete. Wave dependency works. No resource issues. |
| Phase 4 | 8 concurrent workers: system stays responsive, git isn't corrupted, no orphans |
| Phase 5 | Recovery works. Edge cases produce clean errors, not panics or corruption. |

## Fail Criteria (stop and investigate)

- Git corruption (fsck fails)
- Orphaned Codex processes after cancel/timeout
- Daemon crash or panic
- Worktree leak (worktrees accumulate and aren't cleaned)
- Cross-task contamination (one worker sees another's files)
- System unresponsive (can't poll get_task_status while tasks run)
- OOM kill on any process
