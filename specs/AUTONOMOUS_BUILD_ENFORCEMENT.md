# Autonomous Build Enforcement — Specification

**Date:** 2026-04-07
**Author:** Mike Boscia + Claude
**Status:** Draft — Goat Rodeo Round 1 complete, Round 2 in progress
**Depends on:** Triumvirate v2 SPEC.md (agent pool, message fabric, fleet coordinator)
**Origin:** Codex 10-failure post-mortem → goat rodeo execution contract crystallization → deep research (102 sources) → architecture session

---

## What This Is

A three-phase system that lets the human walk away after approving the spec. Phase 1 builds the communication infrastructure. Phase 2 adds mechanical enforcement that doesn't depend on agents choosing to comply. Phase 3 implements the atomic dispatch loop where stateless workers execute tasks while stateful supervisors maintain project visibility.

**The problem in one sentence:** Agents drift from execution contracts because they accumulate context noise that eventually buries the contract signal.

**The fix in one sentence:** Kill context accumulation by making workers stateless — one fresh session per task — while supervisors curate what each worker needs to know.

---

## The Five Requirements

### REQ-A1: Conversational Parity
Claude can programmatically dispatch tasks to Codex and query Gemini without human intermediation. The daemon MCP adapter exposes these capabilities as tool calls within a Claude Code session. This is the prerequisite for everything else.

### REQ-A2: Mechanical Enforcement
No enforcement rule depends on the agent choosing to comply. Enforcement is split by agent type: Claude Code PreToolUse hooks guard the orchestrator (Claude). Codex workers are constrained by OS-level sandbox (Seatbelt/Landlock) + git pre-commit hooks + post-commit validate-task.sh. Git pre-commit hooks reject non-compliant commits with error messages designed for AI self-correction. Three layers per agent, all mechanical.

### REQ-A3: Atomic Dispatch
Each task in an IMPLEMENTATION_PLAN.md is executed by a fresh agent session that receives only a briefing document, the repo state, and the execution contract. No accumulated context from prior tasks. Context dies with the session. State lives in the orchestrator.

### REQ-A4: Supervised Execution
Claude (orchestrator) and Gemini (auditor) maintain full project visibility throughout the build. Claude writes briefings and after-action reports. Gemini reviews committed code between tasks. Workers (Codex) see only their briefing. The supervisors see everything.

### REQ-A5: Self-Correcting Failure Loops
When a worker fails validation, the orchestrator classifies the failure into one of four types: `worker-error` (bad code — repair briefing), `contract-error` (wrong contract — orchestrator fixes contract, redispatches), `orchestrator-briefing-error` (misleading briefing — Claude self-diagnoses with Gemini, rewrites briefing), or `environment-error` (dependency/sandbox issue — halt immediately, escalate). Each class gets a different retry strategy. The failed session's context is discarded — the repair worker inherits the solution, not the failure.

---

## Phase 1: Conversational Parity

**Goal:** Claude can talk to Gemini and dispatch to Codex without the human relaying messages.

**Depends on:** Triumvirate v2 daemon (agent pool, message fabric)

### REQ-A1.1: Daemon MCP Adapter

The daemon exposes an MCP server that Claude Code connects to. Tools available:

| Tool | What it does |
|------|--------------|
| `dispatch_codex` | Spawns a fresh Codex session with a given prompt. Returns task ID. |
| `dispatch_codex_worktree` | Same, but in a git worktree branched from a specified SHA. |
| `query_gemini` | Sends a query to Gemini and returns the response. Synchronous. |
| `query_gemini_review` | Sends code diff to Gemini for review. On failure cases, also sends briefing + contract. Returns concerns/clean. |
| `get_task_status` | Returns status of a dispatched task (working/completed/failed). |
| `get_task_output` | Returns the stdout/commit info from a completed task. |
| `cancel_task` | Kills a running task. |

**Daemon-down behavior:** If the daemon is unreachable, all dispatch/query tools return `DAEMON_UNAVAILABLE` with structured error. The orchestrator attempts one bounded auto-restart. If still down, halt the wave and escalate to human. Never dispatch locally or silently degrade.

### REQ-A1.2: Codex Dispatch Interface

Codex sessions are ephemeral. Each dispatch:
1. Creates a git worktree from a specified commit SHA
2. Creates `.triumvirate/` directory in the worktree, adds it to `.git/info/exclude`
3. Writes `BRIEFING.md` and `contract.json` to `.triumvirate/` (not worktree root — prevents accidental commits)
4. Copies `validate-task.sh` into `.triumvirate/` (Codex sandbox can't reach `~/.claude/scripts/`)
5. Configures per-worktree hooks: `git config --worktree core.hooksPath .triumvirate/hooks/` and installs the contract-aware pre-commit hook there
6. Spawns `codex -p @.triumvirate/BRIEFING.md --approval-policy full-auto --sandbox workspace-write`
7. Monitors for completion (commit detected) or timeout (`task_timeout_sec` from contract.json — enforced by killing the subprocess)
8. Returns: commit SHA, modified files, test output, or failure details

### REQ-A1.3: Gemini Query Interface

Gemini is never dispatched — it's queried. Two modes:
- **Review mode:** Receives a git diff and returns structured review (concerns, suggestions, or "clean")
- **Diagnosis mode:** Receives a failure description and returns root cause analysis + recommended fix

Both use the existing Triumvirate daemon → Gemini CLI pathway. No new infrastructure needed beyond the MCP tool wrappers.

---

## Phase 2: Mechanical Enforcement Stack

**Goal:** Five layers of enforcement, none of which depend on agent compliance.

### REQ-A2.1: contract.json Per Task

Goat rodeo Phase 4.3 produces the execution contract as a markdown appendix in IMPLEMENTATION_PLAN.md. For mechanical enforcement, Phase 4.3 ALSO generates a `contract.json` per task:

```json
{
  "task_id": "T-003",
  "req_ids": ["REQ-002"],
  "wave": 1,
  "file_policy": "default-deny",
  "allowed_files": ["src/engine.rs", "src/engine/mod.rs"],
  "forbidden_files": ["src/cli.rs", "Cargo.toml"],
  "allowed_commands": [["cargo", "check", "-p", "my-crate"], ["cargo", "test", "-p", "my-crate"]],
  "forbidden_commands": [["cargo", "add"], ["rm", "-rf"]],
  "commit_format": "^T-003:",
  "test_command": "cargo test --workspace",
  "task_timeout_sec": 600,
  "done_when": "Engine processes input X and produces output Y with property Z",
  "reality_test": "Specific behavioral test description"
}
```

Generated by the orchestrator before each Codex dispatch. Written to the worktree root.

### REQ-A2.2: Enforcement — Orchestrator (Claude Code Hooks)

Claude Code PreToolUse hooks protect the ORCHESTRATOR session. These hooks fire for Claude, NOT for Codex workers (Codex has its own OS-level sandbox).

**File Scope Guard:**
```json
{
  "hooks": [{
    "event": "PreToolUse",
    "tools": ["Write", "Edit"],
    "handler": {
      "type": "command",
      "command": "~/.claude/hooks/enforce-file-scope.sh"
    }
  }]
}
```

The hook reads `contract.json` from the working directory, inspects the tool input's `file_path`, and returns `exit 2` (deny) if the file is not in `allowed_files`. File policy is **default-deny** — if a file is not in `allowed_files`, it is blocked.

**Command Guard:** Same pattern for Bash tool calls. Reads `contract.json`, checks against `allowed_commands` (token prefix matching — trailing args permitted) and `forbidden_commands`. Blocks dangerous operations unless explicitly allowed for this task.

Error message format (designed for AI self-correction):
```
BLOCKED: Write to src/cli.rs denied by contract T-003.
Allowed files: src/engine.rs, src/engine/mod.rs
Rule: <scope_out> says "Do not modify the CLI parser."
Fix: Only write to files listed in your task's <files> field.
```

### REQ-A2.3: Enforcement — Workers (Codex Sandbox + Git Hooks)

Codex workers are NOT protected by Claude Code hooks. They are constrained by three independent mechanisms:

1. **Codex OS-level sandbox** (Seatbelt on macOS, Landlock on Linux): `--sandbox workspace-write` restricts filesystem access to the worktree. Cannot read or write outside. This is a hard OS boundary, not a guideline.
2. **Git pre-commit hook** (installed per-worktree via `core.hooksPath`): Validates commit message format, file scope against `allowed_files`, and stub markers before any commit is accepted.
3. **Post-commit validate-task.sh** (copied into `.triumvirate/` during dispatch): Runs after commit. Checks the same things as the pre-commit hook plus the full test suite. Exit codes: 0=PASS, 1=BLOCKED, 2=WARN.

The Codex sandbox and git hooks are the mechanical enforcement. validate-task.sh is the verification layer. All three are independent — any one of them catching a violation is sufficient to block the task.

### REQ-A2.4: Git Pre-Commit Hook

Installed per-worktree via `git config --worktree core.hooksPath .triumvirate/hooks/` during dispatch setup (REQ-A1.2 step 5). The hook reads contract.json from `.triumvirate/contract.json`. Checks:
1. Commit message matches `commit_format` from contract.json
2. Modified files (`git diff --cached --name-only`) are within `allowed_files`
3. No stub markers in modified files (TODO, FIXME, unimplemented!, placeholder)
4. Fast test command passes (compilation check)

Error messages follow Fleek's pattern — state the rule, show the violation, provide the fix code. The agent reads the error, self-corrects on first retry.

### REQ-A2.5: Post-Commit Validation (validate-task.sh)

Already built (`~/.claude/scripts/validate-task.sh`). Runs after each commit. Checks commit message, stub markers, file scope, and full test suite. Exit codes: 0=PASS, 1=BLOCKED, 2=WARN.

Enhancement: validate-task.sh also writes results to `.triumvirate/VALIDATION_LOG.md` in the worktree. The orchestrator reads this when building the after-action report.

Note: validate-task.sh is copied into `.triumvirate/` during dispatch (REQ-A1.2 step 4) because Codex's workspace-write sandbox cannot reach `~/.claude/scripts/`.

### REQ-A2.6: Wave Boundary Gate

**No-overlap invariant:** Before dispatching any wave, the orchestrator statically validates that no two tasks in the same wave share `allowed_files` entries. If overlap is detected, the later task is moved to the next wave. This prevents merge conflicts and eliminates the need for conflict resolution logic.

Between waves, the orchestrator:
1. Collects all validation results from completed wave tasks
2. Dispatches Gemini review of all committed code in the wave
3. Runs full test suite against merged state
4. Blocks next wave if any task failed validation or Gemini raised concerns
5. Writes wave summary to BUILD_MANIFEST.md
6. Updates BUILD_STATE.json (current wave, completed tasks, validation pass rate, last commit SHA)

---

## Phase 3: Atomic Dispatch Orchestration

**Goal:** Claude orchestrates the entire build, dispatching stateless workers, maintaining full project state, with Gemini as independent auditor.

### REQ-A3.1: The Orchestration Loop

```
Claude (orchestrator — stateful, full project visibility)
  │
  ├─ Reads IMPLEMENTATION_PLAN.md + Execution Contract
  ├─ Holds: all task XML, all prior validation results,
  │         all deviation history, cumulative build state
  ├─ Persists: BUILD_STATE.json (quick-resume checkpoint),
  │            BUILD_MANIFEST.md + DEVIATION_LOG.md (full audit trail)
  │            All written per-task, all on disk, neither depends on context survival
  │
  ├─ For each task in wave order:
  │   │
  │   ├─ 1. BRIEFING GENERATION
  │   │      Claude writes BRIEFING.md containing:
  │   │      - Task XML block (all 8 fields)
  │   │      - Wave 0 contracts (interfaces/types)
  │   │      - Which files to read first (NOT file contents — worker reads full files from worktree FS)
  │   │      - Synthesized context from prior tasks (10-20 lines max)
  │   │      - Known hazards from prior failures
  │   │      - Execution contract (commit format, done definition)
  │   │
  │   ├─ 2. CONTRACT GENERATION
  │   │      Claude generates contract.json from task XML
  │   │      Installs pre-commit hook in worktree
  │   │
  │   ├─ 3. DISPATCH
  │   │      dispatch_codex_worktree(sha, briefing, contract)
  │   │      Codex session is born. Receives only BRIEFING.md + repo.
  │   │      Zero accumulated context. Zero memory of prior tasks.
  │   │
  │   ├─ 4. MONITOR
  │   │      Wait for completion, timeout, or failure
  │   │      get_task_status() polling or event subscription
  │   │
  │   ├─ 5. VALIDATE
  │   │      validate-task.sh runs (mechanical)
  │   │      Claude reads VALIDATION_LOG.md
  │   │
  │   ├─ 6. IF BLOCKED:
  │   │   │
  │   │   ├─ 6a. Claude reads failure details and CLASSIFIES the failure:
  │   │   │    - worker-error: bad code → repair briefing, new worker
  │   │   │    - contract-error: wrong contract → fix contract, redispatch
  │   │   │    - orchestrator-briefing-error: bad briefing → Claude rewrites with Gemini help
  │   │   │    - environment-error: sandbox/dependency issue → HALT, escalate immediately
  │   │   │
  │   │   ├─ 6b. Claude APPENDS to DEVIATION_LOG.md (MANDATORY):
  │   │   │    Log: task_id, attempt number, failure CLASS, failure details,
  │   │   │    validate-task.sh output, root cause diagnosis
  │   │   │
  │   │   ├─ 6c. query_gemini_review(diff + failure + briefing + contract)
  │   │   │    (Gemini sees the briefing on failure to diagnose orchestrator errors)
  │   │   │
  │   │   ├─ 6d. Claude writes REPAIR_BRIEFING.md (rigid template, size-capped):
  │   │   │    Sections: What Failed → Root Cause → The Fix → Do NOT
  │   │   │    No raw log paste. No conversation history. Diff-focused only.
  │   │   │
  │   │   ├─ 6e. dispatch_codex_worktree(sha, repair_briefing, contract)
  │   │   │    NEW session — no memory of prior failure
  │   │   │
  │   │   └─ Loop: max 3 attempts per failure class, then escalate to human
  │   │        Escalation payload MUST include all briefings Claude generated
  │   │        (enables human to diagnose orchestrator fault vs worker fault)
  │   │        Each attempt gets its own DEVIATION_LOG entry
  │   │
  │   ├─ 7. IF PASS:
  │   │   │
  │   │   ├─ 7a. Claude writes AFTER_ACTION.md:
  │   │   │    task_id, commit_sha, files_modified,
  │   │   │    validation_result, notes_for_next_task
  │   │   │
  │   │   ├─ 7b. Claude APPENDS to BUILD_MANIFEST.md (MANDATORY):
  │   │   │    One row per task. See format below. This is the
  │   │   │    permanent record of what was built. If it's not in
  │   │   │    the manifest, it didn't happen.
  │   │   │
  │   │   ├─ 7c. Claude APPENDS to DEVIATION_LOG.md IF any of:
  │   │   │    - Collateral files were modified outside <files>
  │   │   │    - Task required >1 attempt (repair dispatch)
  │   │   │    - Gemini flagged concerns (even if advisory)
  │   │   │    - <scope_out> was touched with justification
  │   │   │    - Reality test passed but with unexpected behavior
  │   │   │    If NONE of the above: append "T-{ID}: clean — no deviations"
  │   │   │
  │   │   ├─ 7d. query_gemini_review(diff) — async code review
  │   │   │    (On PASS: Gemini reviews diff BLIND — no briefing, preserving independence)
  │   │   │    Returns: concerns, suggestions, or "clean"
  │   │   │    IF concerns → append to DEVIATION_LOG.md AND
  │   │   │    incorporate into next task's briefing
  │   │   │
  │   │   └─ 7e. Claude updates BUILD_STATE.json:
  │   │        tasks_completed, tasks_remaining, current_wave,
  │   │        validation_pass_rate, collateral_fix_count, last_commit_sha
  │   │        (BUILD_STATE.json is the quick-resume checkpoint — survives session crashes)
  │   │
  │   └─ 8. CONTEXT DIES
  │          Codex session is gone. Worktree may be kept (if more
  │          tasks touch same files) or deleted. The only surviving
  │          artifacts are: git commits and Claude's after-action report.
  │
  ├─ End of wave:
  │   ├─ Wave boundary gate (REQ-A2.6)
  │   ├─ Merge all worktree branches into main build branch
  │   ├─ Full test suite on merged state
  │   └─ Wave summary written. Next wave starts.
  │
  └─ End of build:
      ├─ All tasks complete (backlog_status: 0)
      ├─ End-of-execution report in contract format
      ├─ Invoke /postrodeo for full retrospective
      └─ Notify human: "Build complete. Review at your convenience."
```

### REQ-A3.2: Build Artifact Formats

Every atomic work product populates these artifacts. They are append-only during the build. The orchestrator writes them — workers never touch them.

**BUILD_MANIFEST.md** — The permanent record of what was built. One entry per completed task.

```markdown
## BUILD_MANIFEST

| Task | REQ | Wave | Commit | Files Modified | Attempts | Validation | Gemini Review | Timestamp |
|------|-----|------|--------|---------------|----------|------------|---------------|-----------|
| T-001 | REQ-001 | 0 | a1b2c3d | src/traits.rs | 1 | PASS | clean | 2026-04-07T14:23:00Z |
| T-002 | REQ-002 | 1 | e4f5g6h | src/store.rs, src/store/sqlite.rs | 2 | PASS (attempt 2) | concern: missing index | 2026-04-07T14:41:00Z |
| T-003 | REQ-002 | 1 | i7j8k9l | src/engine.rs, src/engine/mod.rs | 1 | PASS | clean | 2026-04-07T14:55:00Z |

### Wave 0 Summary
- Tasks: 1/1 completed
- Validation: 1/1 PASS
- Deviations: 0

### Wave 1 Summary
- Tasks: 2/2 completed
- Validation: 2/2 PASS (1 required repair)
- Deviations: 1 (T-002 first attempt failed stub scan)
- Gemini concerns: 1 (T-002 missing index — noted, not blocking)
```

**Trigger:** Orchestrator appends after step 7b (task PASS). Wave summaries appended at wave boundary gate.

**DEVIATION_LOG.md** — Every departure from the plan, successful or failed. Append-only.

```markdown
## DEVIATION_LOG

### T-002 — Attempt 1 FAILED (2026-04-07T14:30:00Z)
- **Type:** stub_marker
- **validate-task.sh output:** BLOCKED — todo!() at src/store/sqlite.rs:45
- **Root cause:** Worker implemented connection pool but left query methods as stubs
- **Action:** Repair briefing dispatched. Attempt 2.

### T-002 — Attempt 2 PASS (2026-04-07T14:41:00Z)
- **Type:** repair_success
- **Attempts total:** 2
- **What changed:** query methods implemented with actual SQLite calls

### T-002 — Gemini Review Concern (2026-04-07T14:42:00Z)
- **Type:** gemini_advisory
- **Concern:** Missing index on `memories.embedding_id` — will be slow at >10K rows
- **Action:** Noted in next task briefing. Not blocking.

### T-003 — clean — no deviations
```

**Triggers:**
- Step 6b: Every failed validation attempt
- Step 7c: Every task completion (even if clean — "no deviations" entry)
- Step 7d: Every Gemini concern
- Collateral fixes, scope_out violations, unexpected reality test behavior

**Rule:** If a task completed but has no DEVIATION_LOG entry, the orchestrator failed its obligation. Every task gets at minimum a "clean — no deviations" line. Absence of an entry is a bug in the orchestrator, not evidence that nothing happened.

### REQ-A3.3: Briefing Document Format

```markdown
# Task Briefing: T-003

## Your Assignment
[Task description from XML]

## Files You Own
[From <files> — ONLY these files]

## Do NOT Touch
[From <scope_out>]

## Interfaces You Build Against
[Wave 0 contracts — types, traits, function signatures]

## Files to Read First
[From <files> — read these from the worktree filesystem. Full files, not excerpts.]

## What Happened Before You
[Synthesized by Claude — 10-20 lines max. NOT raw prior output.]
- T-001 completed: SqliteKnowledgeStore implemented. Connection pool is lazy-init.
- T-002 completed: EmbeddingBackend trait has 3 implementations. Use embed() not embed_batch().

## Known Hazards
[From prior failures or Gemini review concerns]
- The test fixtures expect UTF-8 input. Don't test with binary.

## When You're Done
[From <done_when>]

## Commit Rules
- Message format: T-003: [description]
- Run: cargo test --workspace
- All tests must pass. Not just yours.
```

### REQ-A3.4: Repair Briefing Format

```markdown
# Repair Briefing: T-003 (Attempt 2 of 3)

## What Failed
validate-task.sh returned BLOCKED:
- Stub marker found: src/engine.rs:87 — `todo!()`
- Reality test failed: cosine distance was 0.0 (hardcoded vector)

## Root Cause
[Claude's diagnosis, informed by Gemini review]
The first attempt implemented embed() as a byte-hasher. It produces
vectors but they have no semantic meaning.

## The Fix
Replace the byte-hash implementation with actual llama.cpp inference.
The model file is at models/code-embed.gguf. Use llama_cpp::Model::load().

## Do NOT
- Do not change the function signature (EmbeddingBackend trait is frozen)
- Do not modify tests — they are correct. Your code must pass them.
- Do not touch any file except src/engine.rs

## Everything Else
[Same as original briefing: interfaces, file state, commit rules]
```

### REQ-A3.5: Supervisor Roles

| Agent | Role | State | What it sees |
|-------|------|-------|-------------|
| **Claude** | Orchestrator | Stateful — holds full project context for entire build | Everything: all task XML, all after-actions, all validation results, all Gemini reviews, cumulative BUILD_MANIFEST |
| **Gemini** | Auditor | Stateful — holds review history for this build | On PASS: code diffs only (independent blind review). On FAIL: code diffs + briefing + contract (to diagnose orchestrator errors). Its own prior review comments. |
| **Codex** | Worker | Stateless — one session per task | ONLY: BRIEFING.md + contract.json + repo state. Nothing else. Born, executes, commits, dies. |

### REQ-A3.6: Escalation Protocol

Escalation is failure-class-aware (see REQ-A5 failure taxonomy):

| Condition | Failure Class | Action |
|-----------|--------------|--------|
| Worker fails validation 1st time | `worker-error` | Repair briefing, new session |
| Worker fails validation 2nd time | `worker-error` | Repair briefing with Gemini diagnosis |
| Worker fails validation 3rd time | `worker-error` | **STOP. Escalate to human.** Payload includes ALL briefings. |
| Contract is wrong (missing file, bad test cmd) | `contract-error` | Orchestrator fixes contract, redispatches. No retry count consumed. |
| Briefing is misleading (Gemini diagnoses) | `orchestrator-briefing-error` | Claude rewrites briefing with Gemini help, redispatches. Counts as 1 retry. |
| Sandbox/dependency/build tool issue | `environment-error` | **HALT immediately.** No retries. Escalate to human. |
| Gemini flags regression concern | — | Claude evaluates. If confirmed: halt and repair before next task. |
| Wave boundary gate fails | — | **STOP. Escalate to human.** Full wave summary in BUILD_MANIFEST. |
| Daemon unreachable | `environment-error` | One auto-restart attempt, then **HALT.** |
| All tasks complete | — | End-of-execution report + /postrodeo |

---

## What This Does NOT Cover

- **Dashboard integration** — Triumvirate v2 dashboard (REQ-6) is separate. This spec's artifacts (BUILD_MANIFEST, validation logs, after-actions) should feed into the dashboard but the integration is out of scope.
- **Fleet scaling beyond 3 agents** — This spec is for the Claude+Gemini+Codex trio. REQ-7 (dynamic multi-agent fleet) extends this pattern but is a separate spec.
- **Conversational workflows** — REQ-1 (N-agent conversation) is for interactive collaboration. This spec is for autonomous build execution where the human is absent.

---

## Traceability

| This spec REQ | Triumvirate v2 REQ | Goat Rodeo Phase |
|---|---|---|
| REQ-A1 (Conversational Parity) | REQ-1 (N-Agent Conversation), REQ-5 (Use All Three) | — |
| REQ-A2 (Mechanical Enforcement) | REQ-4 (It All Works Together) | Phase 4.3, Phase 5.3, Phase 7 |
| REQ-A3 (Atomic Dispatch) | REQ-7 (Dynamic Multi-Agent Fleet) | Phase 5 (Build Execution) |
| REQ-A4 (Supervised Execution) | REQ-5 (Use All Three) | Phase 8 (Post Rodeo Audit) |
| REQ-A5 (Self-Correcting Failures) | REQ-4 (It All Works Together) | Phase 7 (Reality Gate) |

---

## Evidence Base

| Source | What it proves |
|---|---|
| Codex 10-failure post-mortem (2026-04-07) | Agents drift when instructions are advisory. 10 specific failure modes. |
| Deep research report (102 sources, 2026-04-07) | Claude Code hooks are non-bypassable. Git hooks achieve 100% compliance vs 90% prompt-only. Planner/Worker/Judge wins at 2,000-agent scale. |
| Cursor FastRender (2,000 agents) | Flat peer coordination fails. Hierarchical dispatch with stateless workers succeeds. |
| Fleek (YC W22) | Hook error messages = prompt engineering. Agent self-corrects on first retry. |
| ABC framework (arXiv:2602.22302) | Behavioral drift follows Ornstein-Uhlenbeck process. Context accumulation is the mechanism. Stateless sessions reset drift to zero. |
| Agent Contracts (arXiv:2601.08815) | Formal contract tuple with conservation laws. Budget delegation respects parent constraints. Maps to wave-based execution. |
| Triumvirate v1 failure (2026-04-05) | Built engine without steering wheel. Daemon has no MCP interface. UX-before-architecture lesson. |
