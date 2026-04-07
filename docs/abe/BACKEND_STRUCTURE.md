# Autonomous Build Enforcement — Backend Structure

**Version:** Triumvirate v3.0
**Spec:** `specs/AUTONOMOUS_BUILD_ENFORCEMENT.md`
**Tech Stack:** `docs/abe/TECH_STACK.md`

---

## Daemon Architecture

The ABE features extend the existing Triumvirate daemon. New dispatch tools coexist with existing session tools.

### Existing Tools (unchanged)

| Tool | Lifecycle | Purpose |
|------|-----------|---------|
| `spawn_session` | Long-lived, stateful | Create persistent named session for an agent |
| `ask_session` | Long-lived, stateful | Query within a named persistent session |
| `list_sessions` | — | List active sessions |
| `dismiss_session` | — | End a session |

### New Tools (ABE v3.0)

All new tools are registered in the unified MCP server alongside existing tools.

---

## MCP Tool Schemas

### dispatch_codex

Spawns a fresh Codex session with a prompt. No worktree isolation.

```typescript
interface DispatchCodexRequest {
  prompt: string;                    // The full prompt for Codex
  cwd?: string;                     // Working directory (default: project root)
  timeout_sec?: number;             // Task timeout (default: 600)
  sandbox?: "workspace-write" | "read-only" | "danger-full-access";  // Default: workspace-write
}

interface DispatchCodexResponse {
  task_id: string;                  // Unique task identifier
  status: "dispatched";
}
```

### dispatch_codex_worktree

Spawns a fresh Codex session in an isolated git worktree with enforcement artifacts.

```typescript
interface DispatchCodexWorktreeRequest {
  sha: string;                      // Git commit SHA to branch from
  briefing_content: string;         // Markdown string — the full BRIEFING.md content
  contract_fields: ContractFields;  // Structured contract data (see schema below)
  keep_failed_worktree?: boolean;   // Debug flag — preserve worktree on setup failure (default: false)
}

interface ContractFields {
  task_id: string;                  // e.g., "T-003"
  req_ids: string[];                // e.g., ["REQ-002"]
  wave: number;
  file_policy: "default-deny";      // Always default-deny
  allowed_files: string[];          // Files the worker MAY write
  forbidden_files: string[];        // Files the worker MUST NOT write
  allowed_commands: string[][];     // Token prefix arrays, e.g., [["npm", "test"]]
  forbidden_commands: string[][];   // Token prefix arrays
  commit_format: string;            // Regex, e.g., "^T-003:"
  test_command: string;             // Shell command, e.g., "npm test"
  task_timeout_sec: number;         // Seconds before SIGTERM
  done_when: string;                // Semantic completion description
  reality_test: string;             // Behavioral test description
}

interface DispatchCodexWorktreeResponse {
  task_id: string;
  worktree_path: string;
  status: "dispatched";
}

// Error response (on daemon-down or setup failure):
interface DispatchErrorResponse {
  error: "DAEMON_UNAVAILABLE" | "SETUP_FAILED" | "INVALID_SHA";
  message: string;
  details?: string;
}
```

### query_gemini

Synchronous query to Gemini. Returns response inline.

```typescript
interface QueryGeminiRequest {
  query: string;                    // The question or analysis request
  context?: string;                 // Optional context (file contents, code snippets)
}

interface QueryGeminiResponse {
  response: string;                 // Gemini's full response
}
```

### query_gemini_review

Code review query. On failure cases, also accepts briefing + contract.

```typescript
interface QueryGeminiReviewRequest {
  diff: string;                     // Git diff to review
  mode: "pass" | "failure";         // pass = blind review; failure = full context
  briefing?: string;                // Only sent when mode = "failure"
  contract?: ContractFields;        // Only sent when mode = "failure"
  failure_details?: string;         // validate-task.sh output, error messages
}

interface QueryGeminiReviewResponse {
  verdict: "clean" | "concerns" | "regression";
  concerns?: string[];              // List of specific concerns
  suggestions?: string[];           // Non-blocking suggestions
}
```

### get_task_status

Poll task completion status.

```typescript
interface GetTaskStatusRequest {
  task_id: string;
}

interface GetTaskStatusResponse {
  task_id: string;
  status: "working" | "completed" | "failed" | "timeout" | "setup_failed";
  elapsed_sec?: number;
  commit_sha?: string;              // Only when completed
  exit_code?: number;               // Only when failed/timeout
  error_message?: string;           // Only when failed/setup_failed
}
```

### get_task_output

Retrieve results from a completed task.

```typescript
interface GetTaskOutputRequest {
  task_id: string;
}

interface GetTaskOutputResponse {
  task_id: string;
  commit_sha: string;
  modified_files: string[];
  stdout: string;                   // Last N lines of Codex stdout
  validation_log?: string;          // Contents of .triumvirate/VALIDATION_LOG.md
  test_output?: string;             // Test suite output
}
```

### cancel_task

Kill a running task.

```typescript
interface CancelTaskRequest {
  task_id: string;
}

interface CancelTaskResponse {
  task_id: string;
  status: "cancelled";
  worktree_path?: string;           // Path if worktree was preserved
}
```

---

## Daemon Internal: Dispatch Lifecycle

```typescript
// Pseudocode for dispatch_codex_worktree handler
async function handleDispatchWorktree(req: DispatchCodexWorktreeRequest): Promise<DispatchCodexWorktreeResponse> {
  const worktreePath = createWorktreePath(req.contract_fields.task_id);

  try {
    // Step 1: Create worktree
    await exec(`git worktree add ${worktreePath} ${req.sha}`);

    // Step 2: Create .triumvirate/ and exclude it
    await mkdir(`${worktreePath}/.triumvirate`);
    await appendFile(`${worktreePath}/.git/info/exclude`, '.triumvirate/\n');

    // Step 3: Write briefing and contract
    await writeFile(`${worktreePath}/.triumvirate/BRIEFING.md`, req.briefing_content);
    await writeFile(`${worktreePath}/.triumvirate/contract.json`, JSON.stringify(req.contract_fields, null, 2));

    // Step 4: Copy validate-task.sh
    await copyFile(VALIDATE_TASK_SRC, `${worktreePath}/.triumvirate/validate-task.sh`);
    await chmod(`${worktreePath}/.triumvirate/validate-task.sh`, '755');

    // Step 5: Install pre-commit hook
    await mkdir(`${worktreePath}/.triumvirate/hooks`);
    await copyFile(PRECOMMIT_HOOK_SRC, `${worktreePath}/.triumvirate/hooks/pre-commit`);
    await chmod(`${worktreePath}/.triumvirate/hooks/pre-commit`, '755');
    await exec(`git -C ${worktreePath} config --worktree core.hooksPath .triumvirate/hooks/`);

    // Step 6: Spawn Codex (non-blocking)
    const proc = spawn('codex', [
      '-p', `@.triumvirate/BRIEFING.md`,
      '--approval-policy', 'full-auto',
      '--sandbox', 'workspace-write'
    ], { cwd: worktreePath });

    // Step 7: Set build env
    proc.env.CARGO_TARGET_DIR = `${worktreePath}/.triumvirate/target/${req.contract_fields.task_id}`;

    // Step 8: Register for monitoring
    const taskId = registerTask(proc, worktreePath, req.contract_fields);

    // Start timeout watchdog
    startTimeoutWatchdog(taskId, req.contract_fields.task_timeout_sec);

    return { task_id: taskId, worktree_path: worktreePath, status: 'dispatched' };

  } catch (error) {
    // Atomic rollback
    await exec(`git worktree remove --force ${worktreePath}`).catch(() => {});
    await rm(worktreePath, { recursive: true, force: true }).catch(() => {});
    throw { error: 'SETUP_FAILED', message: error.message };
  }
}
```

---

## Timeout Enforcement

```typescript
async function startTimeoutWatchdog(taskId: string, timeoutSec: number) {
  await sleep(timeoutSec * 1000);

  if (getTaskState(taskId) === 'working') {
    // SIGTERM with 10s grace
    sendSignal(taskId, 'SIGTERM');
    await sleep(10_000);

    if (getTaskState(taskId) === 'working') {
      // SIGKILL + git lock cleanup
      sendSignal(taskId, 'SIGKILL');
      const worktree = getWorktreePath(taskId);
      await rm(`${worktree}/.git/index.lock`, { force: true });
    }

    setTaskState(taskId, 'timeout');
  }
}
```

---

## Pre-Commit Hook Logic

The static generic pre-commit hook reads `.triumvirate/contract.json` at runtime:

```bash
#!/usr/bin/env bash
# Pre-commit hook for ABE enforcement
# Static script — reads contract.json at runtime
set -euo pipefail

CONTRACT=".triumvirate/contract.json"
if [ ! -f "$CONTRACT" ]; then
  echo "ERROR: No contract.json found. Are you in an ABE worktree?"
  exit 1
fi

TASK_ID=$(jq -r '.task_id' "$CONTRACT")
COMMIT_FORMAT=$(jq -r '.commit_format' "$CONTRACT")
readarray -t ALLOWED_FILES < <(jq -r '.allowed_files[]' "$CONTRACT")

# Check 1: Commit message format
MSG=$(cat "$1" 2>/dev/null || git log -1 --format=%B)
if ! echo "$MSG" | grep -qE "$COMMIT_FORMAT"; then
  echo "BLOCKED: Commit message does not match format: $COMMIT_FORMAT"
  echo "Fix: Start your message with '$TASK_ID: '"
  exit 1
fi

# Check 2: File scope (default-deny)
STAGED=$(git diff --cached --name-only)
for file in $STAGED; do
  ALLOWED=0
  for af in "${ALLOWED_FILES[@]}"; do
    [ "$file" = "$af" ] && ALLOWED=1 && break
  done
  if [ "$ALLOWED" -eq 0 ] && [[ ! "$file" =~ ^\.triumvirate/ ]]; then
    echo "BLOCKED: Write to $file denied by contract $TASK_ID."
    echo "Allowed files: ${ALLOWED_FILES[*]}"
    echo "Fix: Only modify files listed in your task's <files> field."
    exit 1
  fi
done

# Check 3: Stub markers
STUB_PATTERNS=("todo!()" "unimplemented!()" "TODO" "FIXME" "XXX" "HACK"
               "NotImplementedError" "placeholder" "not implemented" "implement me")
for file in $STAGED; do
  [ -f "$file" ] || continue
  for pat in "${STUB_PATTERNS[@]}"; do
    if git diff --cached -- "$file" | grep -q "+.*$pat"; then
      echo "BLOCKED: Stub marker '$pat' found in staged changes for $file"
      echo "Fix: Replace the stub with a real implementation."
      exit 1
    fi
  done
done

exit 0
```

---

## State Persistence

All state files live in the project root (not in worktrees):

| File | Location | Lifecycle |
|------|----------|-----------|
| BUILD_STATE.json | Project root | Created at build start, updated per-task, read on resume |
| BUILD_MANIFEST.md | Project root | Created at build start, append-only |
| DEVIATION_LOG.md | Project root | Created at build start, append-only |
| AFTER_ACTION.md | Project root (per-task) | Created after each task pass |

These files are written by the orchestrator (Claude) via the Write/Edit tools, NOT by the daemon.
