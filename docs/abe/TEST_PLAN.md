# Autonomous Build Enforcement — Test Plan

**Version:** Triumvirate v3.0
**Spec:** `specs/AUTONOMOUS_BUILD_ENFORCEMENT.md`
**PRD:** `docs/abe/PRD.md`
**Implementation Plan:** `docs/abe/IMPLEMENTATION_PLAN.md`

---

## REQ-to-Test Matrix

| REQ-ID | FEAT-ID | Acceptance Criteria | Test Type | Pass Condition | Reality Test | Pre-Implementation Baseline |
|--------|---------|-------------------|-----------|---------------|-------------|---------------------------|
| REQ-A1.1 | FEAT-001 | 7 MCP tools registered and callable | Integration | All 7 tools return structured responses (not errors) when called with valid params | Call dispatch_codex_worktree → get task_id. Call get_task_status → get status. Call query_gemini → get response string. | Daemon has spawn_session/ask_session only. No dispatch tools exist. |
| REQ-A1.1 | FEAT-001 | Daemon-down returns DAEMON_UNAVAILABLE | Integration | Stop daemon. Call any ABE tool. Get DAEMON_UNAVAILABLE error with structured message. | Auto-restart fires via shell. If daemon recovers, next call succeeds. If not, structured error for human. | No daemon-down handling exists. MCP calls hang or crash. |
| REQ-A1.2 | FEAT-002 | Atomic worktree setup with rollback | Integration | Dispatch creates worktree with all .triumvirate/ artifacts. Inject failure at step 5 → worktree is completely cleaned up. | After setup: ls .triumvirate/ shows BRIEFING.md, contract.json, validate-task.sh, hooks/pre-commit. After rollback: worktree dir does not exist. | No worktree dispatch exists. |
| REQ-A1.2 | FEAT-002 | Non-blocking dispatch with task_id | Integration | dispatch_codex_worktree returns immediately with task_id. Codex process is running. get_task_status returns "working". | Time the call: returns in <1 second. Codex process PID is alive. | No non-blocking dispatch exists. |
| REQ-A1.2 | FEAT-002 | Timeout enforcement | Integration | Set task_timeout_sec=5. Worker sleeps for 60s. Worker is SIGTERM'd at 5s, SIGKILL'd at 15s. | After timeout: get_task_status returns "timeout". No .git/index.lock in worktree. | No timeout enforcement exists. |
| REQ-A1.3 | FEAT-003 | Gemini review returns structured verdict | Integration | Call query_gemini_review with a diff. Get back verdict: "clean", "concerns", or "regression". | Diff with obvious bug → "concerns" with specific concern text. Clean diff → "clean". | Gemini queries exist via ask_session but not as structured review tools. |
| REQ-A2.1 | FEAT-004 | contract.json validates all 12 fields | Unit | Pass valid contract → true. Omit task_id → false with "task_id required". Omit reality_test → false with "reality_test required". | validateContract({}) returns false with 12 error messages. validateContract(valid) returns true with 0 errors. | No contract schema exists. |
| REQ-A2.2 | FEAT-005 | PreToolUse file scope guard blocks writes | Unit | Hook receives tool_input with forbidden file → exit 2 with BLOCKED message. Hook receives allowed file → exit 0. | Forbidden file: exit code is 2, stderr contains "BLOCKED". Allowed file: exit code is 0. | No Claude Code enforcement hooks exist for ABE. |
| REQ-A2.3 | FEAT-006 | Red team: 4 violation types blocked | E2E | Dispatch worker instructed to: write forbidden file, run forbidden command, wrong commit format, leave stub. All 4 blocked. | Each violation produces a specific error message parseable by an AI agent. Worker cannot commit non-compliant code. | No worker enforcement exists. |
| REQ-A2.4 | FEAT-007 | Pre-commit hook reads contract.json | Unit | Stage allowed file → hook passes. Stage forbidden file → hook blocks with BLOCKED message. Wrong commit format → blocks. Stub marker → blocks. | Each check is independent — failing one doesn't skip others. .triumvirate/ files are excluded from file scope check. | No pre-commit hook exists. |
| REQ-A2.5 | FEAT-008 | validate-task.sh writes VALIDATION_LOG.md | Unit | Run validate-task.sh → .triumvirate/VALIDATION_LOG.md contains structured output with CLASS: line. | PASS: CLASS: none. Stub found: CLASS: worker-error. | Current validate-task.sh writes to stdout only, no VALIDATION_LOG.md. |
| REQ-A2.6 | FEAT-009 | No-overlap invariant enforced | Unit | Two tasks with overlapping allowed_files → validateNoOverlap returns false with conflicting files listed. No overlap → returns true. | Overlapping files: ["src/a.ts"] in both T-003 and T-004 → error names both tasks and the file. | No wave validation exists. |
| REQ-A2.6 | FEAT-009 | Wave gate blocks on failure | Integration | Wave with one BLOCKED task → gate returns FAIL. Wave with all PASS → gate returns PASS with summary. | FAIL gate: no next wave starts. PASS gate: BUILD_MANIFEST has wave summary. | No wave boundary gate exists. |
| REQ-A3.1 | FEAT-010 | Orchestration loop completes multi-wave build | E2E | 4-task, 2-wave plan. All tasks complete. BUILD_MANIFEST has 4 entries. DEVIATION_LOG has 4 entries. BUILD_STATE shows 0 remaining. | Waves execute in order. max_parallel respected (no more than 2 concurrent). Wave gate runs between waves. | No orchestration loop exists. |
| REQ-A3.1 | FEAT-010 | Resume protocol works | Integration | Kill session mid-build. Start new session. Say "resume". Claude reads BUILD_STATE.json and continues at correct task. | After resume: no duplicate tasks. Completed tasks not re-executed. Running tasks reconciled with daemon. | No resume protocol exists. |
| REQ-A3.2 | FEAT-011 | BUILD_STATE.json survives crashes | Integration | Write BUILD_STATE.json. Kill process. Read it back. All fields intact. | tasks_running field reflects in-flight tasks. build_started_at is ISO-8601. max_parallel is present. | No BUILD_STATE.json exists. |
| REQ-A3.3 | FEAT-012 | Briefing is advisory, not embedded content | Integration | Briefing contains "Files to Read First" section with file paths. Does NOT contain file contents. Worker reads files from worktree FS. | Grep briefing for actual source code lines → not found. Grep for file paths → found. | No briefing generation exists. |
| REQ-A3.4 | FEAT-013 | Repair briefing follows rigid template | Integration | Generate repair briefing from failure. Verify sections: What Failed, Root Cause, The Fix, Do NOT. No raw log paste. | Grep for validate-task.sh raw output → not found. Grep for section headers → all 4 present. | No repair briefing generation exists. |
| REQ-A5 | FEAT-014 | Mechanical classification by evidence | Unit | Feed VALIDATION_LOG with stub markers → worker-error. Feed log with "BLOCKED: Write to" → contract-error. Feed log with "command not found" → environment-error. Feed ambiguous log → orchestrator-briefing-error. | Each class triggers the correct retry strategy. environment-error triggers immediate HALT (no retries). | No failure classification exists. |
| REQ-A5 | FEAT-014 | Retry caps enforced | Unit | Exhaust 3 worker-errors → escalate. Exhaust 2 contract-errors → escalate. Hit 5 total → escalate regardless of class distribution. | Escalation payload includes all briefings. DEVIATION_LOG has entry for each attempt. | No retry caps exist. |
| REQ-A3.6 | FEAT-015 | Escalation includes all briefings | Integration | Trigger escalation after 5 retries. Verify payload contains every briefing and repair briefing Claude generated. | Parse escalation payload → briefing count matches attempt count. Each briefing is the full markdown content, not a summary. | No escalation protocol exists. |
| REQ-A3.6 | FEAT-015 | Escalation priority order | Unit | Trigger environment-error AND worker-error simultaneously → environment-error wins (HALT). Trigger contract-error AND Gemini concern → contract-error wins. | Higher priority condition is the one acted on. Lower priority is logged as annotation. | No priority ordering exists. |

---

## Test Categories

| Category | Count | Coverage |
|----------|-------|----------|
| Unit | 9 | Contract validation, hook logic, classification, retry caps, overlap detection |
| Integration | 10 | MCP tool wiring, worktree lifecycle, daemon health, resume, build artifacts |
| E2E | 3 | Red team, multi-wave build, Phase 1 acceptance |
| **Total** | **22** | All 16 REQs covered. 0 orphan REQs. |

---

## Manual Tests (require user sign-off)

| Test | Why manual | What to verify |
|------|-----------|---------------|
| Gemini review quality | Gemini's judgment can't be mechanically verified | Review output is relevant to the diff, not generic boilerplate |
| Briefing usefulness | Advisory content quality is subjective | Worker actually uses briefing guidance (reads recommended files first) |
| Escalation readability | Human must judge if the payload is diagnostic | Can the human determine root cause from the escalation payload alone? |
