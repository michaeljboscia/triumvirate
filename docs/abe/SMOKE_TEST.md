# ABE v3.0 — Manual Smoke Test Plan

**Binary:** `daemon/target/release/triumvirate` (Apr 8 00:51)
**Prerequisite:** Binary compiled, MCP bridge restarted (new Claude Code session)
**Tools to test:** 7 new MCP tools visible in session

---

## Pre-Flight

- [ ] Confirm all 7 ABE tools appear in Claude Code's deferred tool list
- [ ] Confirm existing tools still work: `ping`, `list_sessions`, `ask_session`
- [ ] Confirm daemon is responsive: `mcp__triumvirate__ping`

---

## Test 1: dispatch_codex (simple, no worktree)

**Goal:** Prove basic Codex dispatch works through MCP.

```
Call: dispatch_codex
  prompt: "Create a file called /tmp/abe-smoke-test.txt containing 'ABE v3.0 smoke test passed'. Then exit."
  timeout_sec: 60
```

**Expected:**
- [ ] Returns a `task_id` immediately (non-blocking)
- [ ] No errors

---

## Test 2: get_task_status (poll)

**Goal:** Prove status polling works.

```
Call: get_task_status
  task_id: (from Test 1)
```

**Expected:**
- [ ] Returns status: "working" or "completed"
- [ ] Returns elapsed_sec
- [ ] If completed: returns commit_sha (or empty if no git context)

---

## Test 3: get_task_output

**Goal:** Prove output retrieval works.

```
Call: get_task_output
  task_id: (from Test 1, after completion)
```

**Expected:**
- [ ] Returns stdout from the Codex session
- [ ] File /tmp/abe-smoke-test.txt exists with correct content

---

## Test 4: cancel_task

**Goal:** Prove task cancellation works.

```
Call: dispatch_codex
  prompt: "Sleep for 300 seconds."
  timeout_sec: 600

Then immediately:
Call: cancel_task
  task_id: (from above)
```

**Expected:**
- [ ] dispatch returns task_id
- [ ] cancel returns status: "cancelled"
- [ ] Subsequent get_task_status shows "cancelled"

---

## Test 5: query_gemini

**Goal:** Prove Gemini query path works.

```
Call: query_gemini
  query: "What is the capital of France? Answer in one word."
```

**Expected:**
- [ ] Returns response string containing "Paris"
- [ ] No timeout, no error

---

## Test 6: query_gemini_review (pass mode)

**Goal:** Prove Gemini code review works in blind mode.

```
Call: query_gemini_review
  diff: "diff --git a/test.rs b/test.rs\n--- a/test.rs\n+++ b/test.rs\n@@ -1 +1,3 @@\n fn main() {\n+    println!(\"hello\");\n }"
  mode: "pass"
```

**Expected:**
- [ ] Returns verdict: "clean" (trivial diff)
- [ ] No errors

---

## Test 7: dispatch_codex_worktree (the big one)

**Goal:** Prove the full atomic dispatch with enforcement artifacts.

**Prerequisite:** Need a git repo with at least one commit. Use the triumvirate repo itself.

```
Call: dispatch_codex_worktree
  sha: (current HEAD of triumvirate repo)
  briefing_content: "# Smoke Test Briefing\n\n## Your Assignment\nCreate a file called smoke-test-result.txt containing 'ABE dispatch works'.\n\n## Files You Own\nsmoke-test-result.txt\n\n## Commit Rules\n- Message format: SMOKE-001: smoke test\n- No other files modified"
  contract_fields: {
    "task_id": "SMOKE-001",
    "req_ids": ["SMOKE"],
    "wave": 0,
    "file_policy": "default-deny",
    "allowed_files": ["smoke-test-result.txt"],
    "forbidden_files": [],
    "allowed_commands": [["echo"], ["cat"], ["git", "add"], ["git", "commit"]],
    "forbidden_commands": [["rm", "-rf"]],
    "commit_format": "^SMOKE-001:",
    "test_command": "echo PASS",
    "task_timeout_sec": 120,
    "done_when": "smoke-test-result.txt exists with correct content",
    "reality_test": "cat smoke-test-result.txt contains 'ABE dispatch works'"
  }
```

**Expected:**
- [ ] Returns task_id and worktree_path
- [ ] Worktree exists at returned path
- [ ] `.triumvirate/` dir exists in worktree with:
  - [ ] BRIEFING.md (matches briefing_content)
  - [ ] contract.json (matches contract_fields)
  - [ ] validate-task.sh (executable)
  - [ ] hooks/pre-commit (executable)
- [ ] `.git/info/exclude` contains `.triumvirate/`
- [ ] `core.hooksPath` is set to `.triumvirate/hooks`
- [ ] Poll get_task_status until completed or failed
- [ ] If completed: get_task_output returns commit SHA + modified files
- [ ] Worker only modified `smoke-test-result.txt` (enforcement worked)

---

## Test 8: query_gemini_review (failure mode)

**Goal:** Prove Gemini receives briefing + contract on failure.

```
Call: query_gemini_review
  diff: "diff --git a/test.rs b/test.rs\n--- a/test.rs\n+++ b/test.rs\n@@ -1 +1 @@\n-fn process() { todo!() }\n+fn process() { todo!() }"
  mode: "failure"
  briefing: "Task was to implement process() but worker left a stub."
  contract: { "task_id": "T-FAIL", "done_when": "process() does real work" }
  failure_details: "BLOCKED: stub marker todo!() found"
```

**Expected:**
- [ ] Returns verdict: "concerns" (stub in diff)
- [ ] Concerns mention the stub or incomplete implementation
- [ ] Response references the briefing context (not just blind diff review)

---

## Test 9: Daemon-down handling

**Goal:** Prove DAEMON_UNAVAILABLE error when daemon is unreachable.

```
1. Stop the daemon process manually
2. Call: dispatch_codex with any prompt
```

**Expected:**
- [ ] Returns structured error with "DAEMON_UNAVAILABLE"
- [ ] Does not hang or crash Claude Code session
- [ ] After restarting daemon: next call succeeds

---

## Post-Test Cleanup

- [ ] Remove /tmp/abe-smoke-test.txt
- [ ] Remove any smoke test worktrees
- [ ] Verify no orphaned Codex processes: `ps aux | grep codex`

---

## Verdict

| Test | Status |
|------|--------|
| 1. dispatch_codex | |
| 2. get_task_status | |
| 3. get_task_output | |
| 4. cancel_task | |
| 5. query_gemini | |
| 6. query_gemini_review (pass) | |
| 7. dispatch_codex_worktree | |
| 8. query_gemini_review (failure) | |
| 9. Daemon-down handling | |

**All 9 pass → push to origin/main.**
**Any failure → fix before push.**
