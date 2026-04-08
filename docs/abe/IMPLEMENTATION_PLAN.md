# Autonomous Build Enforcement — Implementation Plan

**Version:** Triumvirate v3.0
**Spec:** `specs/AUTONOMOUS_BUILD_ENFORCEMENT.md`
**PRD:** `docs/abe/PRD.md`
**Backend:** `docs/abe/BACKEND_STRUCTURE.md`

---

## Build Overview

- **3 Phases, 6 Waves, 22 Tasks**
- Phase 1 (Conversational Parity): built manually — hand Codex the spec
- Phase 2 (Mechanical Enforcement): can use Phase 1 tools for dispatch
- Phase 3 (Atomic Dispatch Orchestration): fully autonomous via Phase 1+2

---

## Wave 0: Interfaces and Contracts

All types, interfaces, and schemas that downstream tasks build against.

<task id="T-001" req="REQ-A1.1" wave="0" depends="">
  <description>Define MCP tool type signatures for all 7 ABE tools</description>
  <files>mcp-server/src/abe/types.ts</files>
  <scope_out>Do not modify existing MCP tools (spawn_session, ask_session). Do not implement handlers — types only.</scope_out>
  <tools>npx tsc --noEmit</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Import types from another file and instantiate a DispatchCodexWorktreeRequest with all required fields — compiler accepts it. Omit a required field — compiler rejects it.</reality_test>
  <done_when>All 7 tool request/response interfaces compile. ContractFields type matches the spec's contract.json schema exactly.</done_when>
</task>

<task id="T-002" req="REQ-A2.1" wave="0" depends="">
  <description>Define contract.json JSON schema and validation function</description>
  <files>mcp-server/src/abe/contract-schema.ts</files>
  <scope_out>Do not implement enforcement logic. Schema definition and validation only.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Pass a valid contract object — validation returns true. Pass a contract missing task_id — validation returns false with error message naming the missing field.</reality_test>
  <done_when>validateContract(obj) returns typed result with field-level error messages. Schema covers all 12 ContractFields properties.</done_when>
</task>

---

## Wave 1: Phase 1 — Dispatch Infrastructure

<task id="T-003" req="REQ-A1.2" wave="1" depends="T-001">
  <description>Implement worktree setup: create worktree, .triumvirate/ dir, git exclude, copy hook and validator</description>
  <files>daemon/src/abe/worktree-setup.ts</files>
  <scope_out>Do not implement Codex spawning. Do not modify git config outside the worktree. Do not touch existing daemon modules.</scope_out>
  <tools>npx tsc --noEmit, npm test, git worktree add, git worktree remove</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Call setupWorktree() with a valid SHA. Verify: worktree exists, .triumvirate/ contains BRIEFING.md + contract.json + validate-task.sh + hooks/pre-commit. Verify .git/info/exclude contains ".triumvirate/". Verify git config core.hooksPath is set. Then call rollbackWorktree() — verify worktree is completely removed.</reality_test>
  <done_when>setupWorktree creates a fully provisioned worktree. rollbackWorktree cleanly removes it. Both are idempotent.</done_when>
</task>

<task id="T-004" req="REQ-A1.2" wave="1" depends="T-001">
  <description>Implement Codex subprocess spawning with timeout watchdog</description>
  <files>daemon/src/abe/codex-spawn.ts</files>
  <scope_out>Do not implement worktree setup (that's T-003). Do not modify process management for existing sessions. Do not implement monitoring/polling.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Spawn a subprocess that sleeps for 2 seconds — verify it completes with exit 0. Spawn a subprocess that sleeps for 60 seconds with timeout_sec=3 — verify SIGTERM is sent after 3s, SIGKILL after 13s, and .git/index.lock is cleaned up if present.</reality_test>
  <done_when>spawnCodex() launches subprocess with correct flags. Timeout watchdog kills with SIGTERM→grace→SIGKILL. Git lock cleanup works.</done_when>
</task>

<task id="T-005" req="REQ-A1.2" wave="1" depends="T-001">
  <description>Implement task state tracking: register, poll status, get output, cancel</description>
  <files>daemon/src/abe/task-tracker.ts</files>
  <scope_out>Do not implement dispatch or worktree setup. Do not modify existing fleet/session state. State tracking only.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Register a mock task → getStatus returns "working". Mark it completed with a SHA → getStatus returns "completed" with the SHA. Cancel a working task → getStatus returns "cancelled". Register 3 tasks → listTasks returns all 3 with correct statuses.</reality_test>
  <done_when>TaskTracker stores task state in memory. Supports register, getStatus, getOutput, cancel, listByWave. Buffers completion events (no race condition on fast tasks).</done_when>
</task>

---

## Wave 2: Phase 1 — MCP Tool Registration + Integration

<task id="T-006" req="REQ-A1.1,REQ-A1.2" wave="2" depends="T-003,T-004,T-005">
  <description>Wire dispatch_codex_worktree MCP tool: compose worktree setup + Codex spawn + task tracking into a single atomic operation</description>
  <files>mcp-server/src/abe/tools/dispatch-worktree.ts, mcp-server/src/abe/tools/index.ts</files>
  <scope_out>Do not modify existing tool registration. Do not implement Gemini tools yet. Do not modify daemon HTTP routes — use existing patterns.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Call dispatch_codex_worktree via MCP with a valid SHA, briefing string, and contract fields. Verify: worktree created, .triumvirate/ populated, Codex process running, task_id returned. Then simulate setup failure on step 5 — verify worktree is rolled back and SETUP_FAILED error returned.</reality_test>
  <done_when>dispatch_codex_worktree is a registered MCP tool. Single call atomically sets up worktree and spawns Codex. Rollback works on any failure.</done_when>
</task>

<task id="T-007" req="REQ-A1.1" wave="2" depends="T-005">
  <description>Wire get_task_status, get_task_output, and cancel_task MCP tools</description>
  <files>mcp-server/src/abe/tools/task-status.ts, mcp-server/src/abe/tools/task-output.ts, mcp-server/src/abe/tools/cancel-task.ts</files>
  <scope_out>Do not modify task-tracker internals. These are thin MCP wrappers over TaskTracker methods.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Dispatch a task (via T-006). Call get_task_status — returns "working". Wait for completion. Call get_task_status — returns "completed". Call get_task_output — returns commit SHA and modified files. Dispatch another task, call cancel_task — returns "cancelled".</reality_test>
  <done_when>Three MCP tools registered. Each correctly wraps TaskTracker. Error handling for unknown task_id returns structured error.</done_when>
</task>

<task id="T-008" req="REQ-A1.3" wave="2" depends="T-001">
  <description>Wire query_gemini and query_gemini_review MCP tools using existing daemon→Gemini CLI pathway</description>
  <files>mcp-server/src/abe/tools/gemini-query.ts, mcp-server/src/abe/tools/gemini-review.ts</files>
  <scope_out>Do not modify existing Gemini session/CLI infrastructure. Use existing pathway. Do not implement new Gemini features.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Call query_gemini with "What is 2+2?" — returns a response string (not empty). Call query_gemini_review with a diff in "pass" mode — returns verdict (clean/concerns). Call query_gemini_review with mode="failure" and briefing+contract — returns verdict with diagnostic context.</reality_test>
  <done_when>Both Gemini MCP tools registered. Review mode supports pass/failure toggle. Failure mode includes briefing+contract in Gemini query.</done_when>
</task>

<task id="T-009" req="REQ-A1.1" wave="2" depends="T-001">
  <description>Wire dispatch_codex (non-worktree) MCP tool — simple Codex dispatch without isolation</description>
  <files>mcp-server/src/abe/tools/dispatch-codex.ts</files>
  <scope_out>Do not duplicate worktree logic. This is the simple dispatch — no .triumvirate/, no hooks, no contract.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Call dispatch_codex with a simple prompt. Verify task_id returned. Poll get_task_status until completed or failed.</reality_test>
  <done_when>dispatch_codex registered as MCP tool. Spawns Codex with prompt, returns task_id. Uses TaskTracker for state.</done_when>
</task>

<task id="T-010" req="REQ-A1.1" wave="2" depends="T-006,T-007,T-008,T-009">
  <description>Daemon-down detection and auto-restart via local shell</description>
  <files>mcp-server/src/abe/daemon-health.ts</files>
  <scope_out>Do not modify daemon startup logic. This is the MCP-side health check and shell restart — not daemon internals.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>With daemon running: health check returns OK. Stop daemon manually. Next MCP call returns DAEMON_UNAVAILABLE. Auto-restart fires via shell script. Daemon comes back. Next MCP call succeeds.</reality_test>
  <done_when>All 7 ABE MCP tools return DAEMON_UNAVAILABLE on connection failure. One bounded auto-restart attempt via local shell. If restart fails, returns structured error for human escalation.</done_when>
</task>

---

## Wave 3: Phase 2 — Enforcement Scripts

<task id="T-011" req="REQ-A2.4" wave="3" depends="T-002">
  <description>Write the static pre-commit hook script that reads .triumvirate/contract.json</description>
  <files>daemon/assets/pre-commit-hook.sh</files>
  <scope_out>Do not modify git config. Do not modify the daemon. This is a standalone bash script asset.</scope_out>
  <tools>bash, jq, git</tools>
  <verify>bash -n daemon/assets/pre-commit-hook.sh</verify>
  <reality_test>Create a temp git repo with .triumvirate/contract.json. Stage a file IN allowed_files — hook passes. Stage a file NOT in allowed_files — hook blocks with "BLOCKED: Write to X denied." Stage a commit with wrong message format — hook blocks. Stage a file with "TODO" — hook blocks with stub marker message.</reality_test>
  <done_when>Pre-commit hook script reads contract.json, enforces file scope (default-deny), commit format, and stub markers. Error messages follow Fleek pattern. All checks work independently.</done_when>
</task>

<task id="T-012" req="REQ-A2.4" wave="3" depends="T-011">
  <description>Unit tests for the pre-commit hook — contract parsing, file matching, edge cases</description>
  <files>daemon/tests/pre-commit-hook.test.sh</files>
  <scope_out>Do not modify the hook script itself. Test-only task.</scope_out>
  <tools>bash, git, jq</tools>
  <verify>bash daemon/tests/pre-commit-hook.test.sh</verify>
  <reality_test>Run the test suite — all tests pass. Intentionally break the hook (wrong jq path) — at least one test fails.</reality_test>
  <done_when>Test suite covers: valid contract, missing contract, missing fields, file in allowed_files, file not in allowed_files, .triumvirate/ files excluded from check, correct commit format, wrong commit format, stub markers found, stub markers in comments (should pass), empty staged files.</done_when>
</task>

<task id="T-013" req="REQ-A2.5" wave="3" depends="T-002">
  <description>Update validate-task.sh: add VALIDATION_LOG.md output, classification hints, .triumvirate/ awareness, Python pass pattern</description>
  <files>~/.claude/scripts/validate-task.sh</files>
  <scope_out>Do not change exit code semantics (0/1/2). Do not change the CLI interface. Additive changes only.</scope_out>
  <tools>bash</tools>
  <verify>bash -n ~/.claude/scripts/validate-task.sh</verify>
  <reality_test>Run validate-task.sh on a commit with a stub marker — output includes "CLASS: worker-error" AND .triumvirate/VALIDATION_LOG.md is written with the same content. Run on a clean commit — VALIDATION_LOG.md shows PASS with classification "CLASS: none".</reality_test>
  <done_when>validate-task.sh writes VALIDATION_LOG.md to .triumvirate/ (if dir exists, else cwd). Emits CLASS: lines for mechanical classifier. Reads contract from .triumvirate/contract.json if present. Adds Python `pass` as sole function body to stub patterns.</done_when>
</task>

<task id="T-014" req="REQ-A2.2" wave="3" depends="T-002">
  <description>Write Claude Code PreToolUse hooks for orchestrator enforcement (file scope guard + command guard)</description>
  <files>~/.claude/hooks/enforce-file-scope.sh, ~/.claude/hooks/enforce-command-scope.sh</files>
  <scope_out>Do not modify Claude Code settings.json — that's a separate configuration step. Hook scripts only.</scope_out>
  <tools>bash, jq</tools>
  <verify>bash -n ~/.claude/hooks/enforce-file-scope.sh && bash -n ~/.claude/hooks/enforce-command-scope.sh</verify>
  <reality_test>Set up a contract.json in cwd. Run enforce-file-scope.sh with a tool_input containing an allowed file path — exits 0. Run with a forbidden file path — exits 2 with BLOCKED message. Run enforce-command-scope.sh with an allowed command prefix — exits 0. Run with a forbidden command — exits 2.</reality_test>
  <done_when>Both hook scripts read contract.json from cwd, parse tool input from stdin (JSON), and exit 0 (allow) or 2 (deny) with Fleek-pattern error messages.</done_when>
</task>

---

## Wave 4: Phase 2 — Integration + Red Team

<task id="T-015" req="REQ-A2.6" wave="4" depends="T-011,T-013">
  <description>Implement wave boundary gate: no-overlap validation, merge, full test suite, wave summary</description>
  <files>mcp-server/src/abe/wave-gate.ts</files>
  <scope_out>Do not modify the orchestration loop (that's Phase 3). This is the gate logic called between waves.</scope_out>
  <tools>npx tsc --noEmit, npm test, git</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Create two mock task results for a wave — both PASS. Run wave gate — returns PASS with wave summary. Create two tasks with overlapping allowed_files — validateNoOverlap rejects before dispatch. Create a task with BLOCKED validation — wave gate returns FAIL.</reality_test>
  <done_when>WaveGate validates no-overlap invariant, collects validation results, runs full test suite on merged state, produces wave summary for BUILD_MANIFEST. Returns PASS/FAIL verdict.</done_when>
</task>

<task id="T-016" req="REQ-A2.3" wave="4" depends="T-006,T-011,T-013">
  <description>Phase 2 red team acceptance test: dispatch deliberately non-compliant worker</description>
  <files>daemon/tests/red-team.test.ts</files>
  <scope_out>Do not modify enforcement scripts. Test-only task. This validates the enforcement stack works end-to-end.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npm test -- --grep "red-team"</verify>
  <reality_test>Dispatch a Codex worker with instructions to: (1) write to a forbidden file — pre-commit hook blocks it. (2) Run a forbidden command — sandbox blocks it. (3) Commit with wrong message format — pre-commit hook blocks it. (4) Leave a stub marker — pre-commit hook blocks it. All four violations produce self-correction error messages.</reality_test>
  <done_when>All four violation types are blocked by the enforcement stack. Error messages are parseable by an AI agent. Test is repeatable and deterministic.</done_when>
</task>

---

## Wave 5: Phase 3 — Orchestration Loop

<task id="T-017" req="REQ-A3.1" wave="5" depends="T-006,T-007,T-008,T-015">
  <description>Implement the orchestration loop: briefing generation, concurrent dispatch, monitoring, validation, state updates</description>
  <files>mcp-server/src/abe/orchestrator.ts</files>
  <scope_out>Do not modify MCP tools — consume them. Do not modify enforcement scripts. Do not implement failure handling (that's T-018).</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Create a 3-task implementation plan across 2 waves. Run the orchestrator. Verify: all 3 tasks dispatched in correct wave order, max_parallel respected, BUILD_STATE.json updated after each task, BUILD_MANIFEST.md has 3 entries, DEVIATION_LOG.md has 3 entries (even if all clean).</reality_test>
  <done_when>Orchestrator reads IMPLEMENTATION_PLAN.md, generates briefings, dispatches via MCP, monitors concurrent tasks, validates results, writes all build artifacts. Happy path works end-to-end.</done_when>
</task>

<task id="T-018" req="REQ-A5,REQ-A3.6" wave="5" depends="T-017">
  <description>Implement failure classification, retry logic, repair briefing generation, and escalation protocol</description>
  <files>mcp-server/src/abe/failure-handler.ts</files>
  <scope_out>Do not modify the orchestrator loop — this is called BY the orchestrator. Do not modify validate-task.sh. Do not modify Gemini tools.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Feed a VALIDATION_LOG.md with stub markers → classifies as worker-error, generates repair briefing. Feed a log with "BLOCKED: Write to forbidden file" → classifies as contract-error, fixes contract. Feed a log with "command not found" → classifies as environment-error, returns HALT. Exhaust 5 retries → returns ESCALATE with all briefings attached.</reality_test>
  <done_when>Mechanical classifier parses VALIDATION_LOG.md output. Four failure classes handled with correct retry strategies. Per-class caps enforced (3/2/2/0, total 5). Repair briefings follow rigid template. Escalation payload includes all briefings.</done_when>
</task>

<task id="T-019" req="REQ-A3.2" wave="5" depends="T-017">
  <description>Implement BUILD_STATE.json, BUILD_MANIFEST.md, and DEVIATION_LOG.md writers</description>
  <files>mcp-server/src/abe/build-artifacts.ts</files>
  <scope_out>Do not implement the orchestrator loop. These are utility functions called by the orchestrator. Do not modify file formats — they match the spec exactly.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Call appendManifest with task result → BUILD_MANIFEST.md has a new row with correct columns. Call appendDeviation with a failure → DEVIATION_LOG.md has structured entry. Call updateState → BUILD_STATE.json reflects new task counts. Read BUILD_STATE.json back → all fields parse correctly.</reality_test>
  <done_when>Three writer functions: appendManifest, appendDeviation, updateState. Append-only for markdown files. Full replace for JSON. Formats match spec exactly.</done_when>
</task>

<task id="T-020" req="REQ-A3.1" wave="5" depends="T-017">
  <description>Implement resume protocol: read BUILD_STATE.json, reconcile with daemon, continue at first incomplete task</description>
  <files>mcp-server/src/abe/resume.ts</files>
  <scope_out>Do not modify BUILD_STATE.json schema. Do not modify the orchestrator loop — this feeds into it.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npx tsc --noEmit</verify>
  <reality_test>Create a BUILD_STATE.json with 2 completed and 3 remaining tasks. Call resume() → returns the state with first incomplete task identified. Create a state with 1 task in tasks_running but daemon says it's completed → reconcile moves it to tasks_completed. Create a state with 1 task in tasks_running but daemon has no process → triggers crash recovery (quarantine + redispatch).</reality_test>
  <done_when>resume() reads BUILD_STATE.json, calls get_task_status for running tasks, reconciles state, returns ResumeResult with first_incomplete_task and any crash recovery actions taken.</done_when>
</task>

---

## Wave 6: Phase 3 — End-to-End Integration

<task id="T-021" req="REQ-A1,REQ-A2,REQ-A3" wave="6" depends="T-017,T-018,T-019,T-020">
  <description>Phase 1 end-to-end acceptance test: dispatch + poll + Gemini review</description>
  <files>daemon/tests/e2e-phase1.test.ts</files>
  <scope_out>Test-only. Do not modify any implementation files.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npm test -- --grep "e2e-phase1"</verify>
  <reality_test>Dispatch a Codex worker that creates a file and commits it. Poll get_task_status until completed. Call get_task_output — verify commit SHA exists. Run query_gemini_review on the diff — verify verdict returned. Cancel a long-running task — verify cancellation.</reality_test>
  <done_when>Full Phase 1 acceptance test passes: dispatch → poll → output → review → cancel. All MCP tools function end-to-end.</done_when>
</task>

<task id="T-022" req="REQ-A3.1,REQ-A5" wave="6" depends="T-017,T-018,T-019,T-020">
  <description>Full orchestration integration test: multi-wave build with failure + recovery</description>
  <files>daemon/tests/e2e-orchestration.test.ts</files>
  <scope_out>Test-only. Do not modify any implementation files.</scope_out>
  <tools>npx tsc --noEmit, npm test</tools>
  <verify>npm test -- --grep "e2e-orchestration"</verify>
  <reality_test>Create a 4-task plan (2 waves). Task 1 and 2 pass cleanly. Task 3 fails first attempt (inject a stub), repair briefing dispatched, passes on attempt 2. Task 4 passes. Verify: BUILD_MANIFEST has 4 entries. DEVIATION_LOG has entries for T-003 failure + repair + all clean entries. BUILD_STATE.json shows 0 remaining. Wave boundary gate ran between waves.</reality_test>
  <done_when>Multi-wave build with at least one failure and recovery completes end-to-end. All build artifacts are correct. Orchestration loop handles happy path and failure path in a single run.</done_when>
</task>

---

## Execution Contract

### Backlog Freeze
This document contains 22 tasks across 6 waves. This is the COMPLETE backlog.
- Do NOT accept new tasks until all tasks are complete (backlog_status: 0).
- If new requirements arrive mid-execution, respond: `blocked_on: scope-change — [describe new requirement]` and STOP.
- Only the human can add, remove, or reorder tasks in this backlog.

### Execution Order
- Wave order is strict: complete ALL tasks in Wave N before starting Wave N+1.
- Within a wave: tasks are parallel-safe (no dependencies on each other). Execute concurrently or in any order.
- Within a sequential group: strict FIFO. Do not start T(N+1) before T(N) is committed and reported.

### Definition of Done (Per Task)
A task is DONE when ALL of these are true:
1. Code is written (not stubbed — see reality test)
2. `<verify>` passes (compilation/type check)
3. `<reality_test>` passes (behavioral check that a stub cannot fake)
4. `<done_when>` condition is met (semantic completion check)
5. FULL test suite passes (`npm test`) — not just this task's tests
6. Git commit is created with message referencing task ID

A task that passes its own tests but breaks other tests is NOT done. Fix the regression first.

### Commit Report Format
After each task commit, respond with EXACTLY this format and nothing else:
```
task: T-{ID}
commit: {hash}
changed: {1-5 bullets, one per file or logical change}
tests: npm test → {pass count}/{total count} passed
remaining: {N} tasks in current wave, {M} total
```
No interim progress updates. No explanations between tasks. No summaries until backlog_status: 0.

### Collateral Fix Protocol
If completing a task REQUIRES touching files outside that task's `<files>` list:
1. Label the commit: `collateral-fix: T-{ID} — {one-line justification}`
2. List extra files in the commit report under a `collateral:` field
3. Re-run full test suite after the collateral fix

If you WANT to touch adjacent code but don't NEED to, don't. Scope discipline > local improvement.

### Blocked Protocol
If blocked on any task, respond with EXACTLY:
```
blocked_on: {single concrete blocker}
task: T-{ID}
evidence: {command + output summary, max 5 lines}
proposed_fix: {single action you would take}
```
Then STOP. Do not proceed to the next task. Do not attempt workarounds without reporting.

### Context-Switch Refusal
If you receive instructions not in this backlog during execution:
- Respond: "Outside current execution contract. Backlog has {N} remaining tasks. Complete backlog first, or explicitly cancel it."
- Do NOT start the new work.
- Do NOT interleave new work with backlog tasks.
- Only an explicit "cancel backlog" or "pause backlog" from the human allows context-switching.

### Self-Validation (MANDATORY)
After each task commit, run the validation script:
```
.triumvirate/validate-task.sh T-{ID} "npm test" {files from <files> list}
```
- If BLOCKED (exit 1): fix the failure before proceeding. Do NOT skip to next task.
- If WARN (exit 2): proceed, but include warnings in commit report.
- If PASS (exit 0): proceed to next task.

Between waves: run validation on ALL tasks in the wave before starting the next wave. Print the cumulative validation report.

### End-of-Execution Report
When all tasks are complete, respond with:
```
backlog_status: 0 remaining
completed_tasks: [T-001, T-002, ..., T-022]
total_commits: {N}
collateral_fixes: {N} ({list if any})
validation: {N}/{N} tasks passed validate-task.sh
test_suite: npm test → {pass/fail with counts}
```
