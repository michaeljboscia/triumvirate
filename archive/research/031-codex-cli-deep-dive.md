# 031 — Codex CLI Deep Dive: Programmatic Control Surface

**Date:** 2026-04-03
**Source:** openai/codex GitHub repo (Rust source), OpenAI developer docs, local codex-cli 0.116.0
**Purpose:** Exhaustive analysis of Codex CLI capabilities for use as a persistent subprocess in the Triumvirate Go daemon

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Architecture Overview](#architecture-overview)
3. [codex exec --json: JSONL Event Schema](#codex-exec---json-jsonl-event-schema)
4. [Session Persistence and Multi-Turn](#session-persistence-and-multi-turn)
5. [AGENTS.md: Custom System Prompt Injection](#agentsmd-custom-system-prompt-injection)
6. [Sandbox Modes](#sandbox-modes)
7. [Approval Policies and --full-auto](#approval-policies-and---full-auto)
8. [--output-schema: Structured Output](#--output-schema-structured-output)
9. [Interruption and Cancellation](#interruption-and-cancellation)
10. [codex mcp-server: Alternative Integration Path](#codex-mcp-server-alternative-integration-path)
11. [codex app-server: WebSocket Integration](#codex-app-server-websocket-integration)
12. [Known Issues and Regressions](#known-issues-and-regressions)
13. [Integration Recommendations for Go Daemon](#integration-recommendations-for-go-daemon)
14. [Missing --quiet Flag](#missing---quiet-flag)

---

## Executive Summary

Codex CLI v0.116.0 provides three programmatic integration surfaces:

| Surface | Protocol | Multi-Turn | Structured Events | Maturity |
|---------|----------|------------|-------------------|----------|
| `codex exec --json` | JSONL on stdout | Yes (via `resume`) | Yes (10 event types) | Stable with known bugs |
| `codex mcp-server` | MCP stdio (JSON-RPC) | Yes (`codex-reply` tool) | Partial (tool output) | Stable |
| `codex app-server` | WebSocket or stdio | Yes (full thread API) | Yes (full protocol) | Experimental |

**Bottom line:** `codex exec --json` is the primary headless integration path. It supports single-shot and multi-turn (via `exec resume`), emits well-typed JSONL events, supports `--output-schema` for structured responses, and handles Ctrl+C interruption. However, each invocation is a separate OS process — there is no persistent subprocess mode where you keep writing prompts to stdin. Multi-turn requires re-invoking with `codex exec resume <session_id> "follow-up"`.

The `codex mcp-server` mode IS a persistent subprocess (stdio JSON-RPC) with multi-turn via the `codex-reply` tool, but it uses MCP protocol rather than raw JSONL events.

---

## Architecture Overview

Codex CLI is written in Rust (`codex-rs/`). The exec binary spawns an in-process app-server client that manages the full agent loop:

```
codex exec --json "prompt"
  └── InProcessAppServerClient
       ├── thread/start → thread_id (UUID)
       ├── turn/start → sends prompt
       ├── event loop (notifications → JSONL stdout)
       │   ├── Ctrl+C → turn/interrupt
       │   └── turn.completed → thread/unsubscribe → shutdown
       └── client.shutdown()
```

Key invariants from source:
- **stdout is sacred.** In default mode, ONLY the final agent message goes to stdout. In `--json` mode, ONLY valid JSONL goes to stdout. All other output (progress, warnings) goes to stderr. The crate enforces `#![deny(clippy::print_stdout)]`.
- **Approval requests are auto-rejected in exec mode.** Command execution approval, file change approval, `request_user_input`, dynamic tool calls, and apply-patch approval all get rejected with an error message. MCP elicitations are auto-cancelled.
- **Default approval policy is `Never`** in exec mode (line ~310 of lib.rs: `approval_policy: Some(AskForApproval::Never)`).

---

## codex exec --json: JSONL Event Schema

### Top-Level Event Types (from `exec_events.rs`)

Every line is a JSON object with a `"type"` field. The complete enum:

```rust
enum ThreadEvent {
    "thread.started"   // First event, contains thread_id
    "turn.started"     // New turn begins
    "turn.completed"   // Turn ends with usage stats
    "turn.failed"      // Turn failed with error
    "item.started"     // Item begins (tool call, command, etc.)
    "item.updated"     // Item state changed (todo list updates)
    "item.completed"   // Item finished
    "error"            // Unrecoverable stream error
}
```

### Item Types (from `ThreadItemDetails` enum)

Items carry a `"type"` field identifying their payload:

| Item Type | Emitted On | Key Fields |
|-----------|-----------|------------|
| `agent_message` | completed only | `text: string` |
| `reasoning` | completed only | `text: string` |
| `command_execution` | started + completed | `command`, `aggregated_output`, `exit_code`, `status` |
| `file_change` | completed only | `changes[{path, kind}]`, `status` |
| `mcp_tool_call` | started + completed | `server`, `tool`, `arguments`, `result`, `error`, `status` |
| `collab_tool_call` | started + completed | `tool`, `sender_thread_id`, `receiver_thread_ids`, `agents_states`, `status` |
| `web_search` | started + completed | `id`, `query`, `action` |
| `todo_list` | started + updated + completed | `items[{text, completed}]` |
| `error` | completed only | `message: string` |

### Concrete JSONL Examples

**Thread start:**
```json
{"type":"thread.started","thread_id":"550e8400-e29b-41d4-a716-446655440000"}
```

**Turn start:**
```json
{"type":"turn.started"}
```

**Command execution started:**
```json
{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"ls -la","aggregated_output":"","exit_code":null,"status":"in_progress"}}
```

**Command execution completed:**
```json
{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"ls -la","aggregated_output":"total 48\ndrwxr-xr-x  12 user  staff  384 Apr  3 10:00 .\n","exit_code":0,"status":"completed"}}
```

**File change:**
```json
{"type":"item.completed","item":{"id":"item_1","type":"file_change","changes":[{"path":"src/main.rs","kind":"update"},{"path":"src/new.rs","kind":"add"}],"status":"completed"}}
```

**MCP tool call started:**
```json
{"type":"item.started","item":{"id":"item_2","type":"mcp_tool_call","server":"myserver","tool":"search","arguments":{"query":"test"},"result":null,"error":null,"status":"in_progress"}}
```

**MCP tool call completed (success):**
```json
{"type":"item.completed","item":{"id":"item_2","type":"mcp_tool_call","server":"myserver","tool":"search","arguments":{"query":"test"},"result":{"content":[{"type":"text","text":"found 3 results"}],"structured_content":null},"error":null,"status":"completed"}}
```

**MCP tool call completed (failure):**
```json
{"type":"item.completed","item":{"id":"item_2","type":"mcp_tool_call","server":"myserver","tool":"search","arguments":{},"result":null,"error":{"message":"user cancelled MCP tool call"},"status":"failed"}}
```

**Agent message:**
```json
{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"I've updated the file. Here's what I changed..."}}
```

**Todo list update:**
```json
{"type":"item.updated","item":{"id":"item_4","type":"todo_list","items":[{"text":"Read the codebase","completed":true},{"text":"Implement the fix","completed":false}]}}
```

**Turn completed:**
```json
{"type":"turn.completed","usage":{"input_tokens":24763,"cached_input_tokens":24448,"output_tokens":122}}
```

**Turn failed:**
```json
{"type":"turn.failed","error":{"message":"rate limit exceeded (retry in 30s)"}}
```

### Status Enums

**CommandExecutionStatus:** `in_progress`, `completed`, `failed`, `declined`
**PatchApplyStatus:** `in_progress`, `completed`, `failed`
**McpToolCallStatus:** `in_progress`, `completed`, `failed`
**CollabToolCallStatus:** `in_progress`, `completed`, `failed`
**PatchChangeKind:** `add`, `delete`, `update`

### Item ID Assignment

Items get sequential IDs: `item_0`, `item_1`, `item_2`, etc. The exec processor maintains its own ID sequence (separate from the internal app-server IDs). `item.started` and `item.completed` for the same operation share the same ID. `agent_message` and `reasoning` items ONLY emit `item.completed` (no started event).

---

## Session Persistence and Multi-Turn

### Single-Shot Mode (Default)
```bash
codex exec --json "do something" → runs one turn, exits
```
The process starts, sends one prompt, waits for the turn to complete, then shuts down. Session state is persisted to disk (unless `--ephemeral`).

### Multi-Turn via Resume
```bash
# First invocation
codex exec --json "analyze this repo" -o /tmp/result.txt
# Parse thread_id from thread.started event

# Second invocation
codex exec --json resume <thread_id> "now fix the bug you found"

# Or resume the most recent session
codex exec --json resume --last "follow up"
```

**How it works internally:**
1. `exec resume` calls `thread/list` to find the session, then `thread/resume` to reload it.
2. The resumed thread retains its full conversation history.
3. A new `turn/start` is sent with the follow-up prompt.
4. The process exits after the new turn completes.

**Critical limitation:** Each invocation is a separate OS process. There is NO way to keep a single `codex exec` process alive and send multiple prompts. You must re-invoke the binary each time. This means:
- MCP servers are re-initialized on each resume (startup cost).
- AGENTS.md is re-read on each invocation.
- The process startup overhead applies each time.

### Session Storage

Sessions are persisted to `~/.codex/sessions/` (SQLite state DB). The `--ephemeral` flag prevents writing session files. Session IDs are UUIDs. Sessions can also be resumed by thread name (non-UUID string).

### State Between Invocations

The conversation history IS preserved across `exec resume` calls — the model sees the full prior context. However, runtime state (running MCP servers, in-memory caches, sandbox state) is NOT preserved. Each invocation is a cold start with warm conversation history.

---

## AGENTS.md: Custom System Prompt Injection

### Discovery Order (from `project_doc.rs`)

1. **Global scope:** `~/.codex/AGENTS.override.md` > `~/.codex/AGENTS.md` (first non-empty wins)
2. **Project scope:** Walk from git repo root down to cwd, checking each directory for:
   - `AGENTS.override.md` (highest priority)
   - `AGENTS.md`
   - Any filename in `project_doc_fallback_filenames` config

Files are concatenated root-to-cwd with blank lines as separators. Files closer to cwd appear LATER in the prompt (higher effective priority since LLMs weight recency).

### Size Limit

`project_doc_max_bytes` = **32,768 bytes (32 KiB)** by default. Configurable up to 65,536 in config.toml:
```toml
project_doc_max_bytes = 65536
```

When the budget is exhausted, remaining files are silently truncated with a warning in the log.

### Injection Point

AGENTS.md content is injected as **user instructions** in the system prompt. From `get_user_instructions()`:

```
[Config.instructions (if any)]
--- project-doc ---
[AGENTS.md content, concatenated root-to-cwd]
[JS REPL instructions (if enabled)]
[Hierarchical agents message (if child_agents_md feature enabled)]
```

### Can We Inject Our Markdown Keyword Protocol?

**Yes, absolutely.** AGENTS.md accepts arbitrary Markdown. We can put our keyword protocol, structured response format requirements, or any instruction payload here. The content goes into the user instructions message sent to the model.

**Three injection methods:**
1. **AGENTS.md file** — Write our protocol to `AGENTS.md` in the working directory
2. **AGENTS.override.md** — Higher priority override (global at `~/.codex/AGENTS.override.md` or per-directory)
3. **`developer_instructions` config key** — Injected as a separate developer role message (via config.toml or CLI override: `-c developer_instructions="..."`)
4. **`base_instructions` config key** — Replaces the default system instructions entirely

For the Go daemon, the cleanest approach: write a temporary `AGENTS.md` to the workspace directory before invoking `codex exec`, or use `-c developer_instructions="<protocol>"`.

### Fallback Filenames

Configurable alternative filenames that Codex will scan:
```toml
project_doc_fallback_filenames = ["TEAM_GUIDE.md", ".agents.md", "CLAUDE.md"]
```

### child_agents_md Feature Flag

When `features.child_agents_md = true`, Codex appends a hierarchical scope/precedence guide to the user instructions. This tells the model how AGENTS.md files in different directories relate to each other.

---

## Sandbox Modes

### Three Modes

| Mode | CLI Flag | File Access | Network | Use Case |
|------|----------|-------------|---------|----------|
| `read-only` | `--sandbox read-only` (default) | Read only | Blocked | Safe analysis |
| `workspace-write` | `--sandbox workspace-write` | Write to cwd + writable_roots | Configurable | Normal dev work |
| `danger-full-access` | `--sandbox danger-full-access` | Full filesystem | Full | CI with external sandbox |

### workspace-write Details

```toml
[sandbox_workspace_write]
writable_roots = ["/Users/YOU/.pyenv/shims", "/tmp/extra"]
network_access = false        # default: blocked
exclude_slash_tmp = false     # keep /tmp writable
exclude_tmpdir_env_var = false
```

Protected paths remain read-only even in workspace-write: `.git/`, `.codex/`.

### macOS Implementation

On macOS, sandboxing uses Apple's `sandbox-exec` (Seatbelt) framework. The Rust code generates a Seatbelt profile that:
- Allows/denies file paths based on the sandbox policy
- Controls network access via the Seatbelt network filter
- Sets `CODEX_SANDBOX=seatbelt` env var for child processes
- Sets `CODEX_SANDBOX_NETWORK_DISABLED=1` when network is blocked

### Linux Implementation

Uses `bwrap` (bubblewrap) or Landlock for sandboxing. The `codex-linux-sandbox` binary handles the actual containment.

### For Our Go Daemon

**Recommendation:** Use `--sandbox workspace-write` with explicit `writable_roots` for the project directory. Add `network_access = true` if MCP servers need network. Avoid `danger-full-access` unless the daemon itself provides sandboxing.

---

## Approval Policies and --full-auto

### Approval Policy Levels

| Policy | Behavior in Exec Mode |
|--------|-----------------------|
| `untrusted` | Only "trusted" commands (ls, cat, sed) run without approval. Others get **rejected** (not prompted — exec can't prompt). |
| `on-request` | Model decides when to ask. Requests get **rejected** in exec mode. |
| `never` | **Default in exec mode.** All commands run without approval. |

### --full-auto Behavior

`--full-auto` is a convenience alias that sets:
- `--sandbox workspace-write`
- `-a on-request` (approval policy)

**In exec mode, this is slightly misleading.** The `on-request` policy means the model can try to request approval, but those requests will be rejected since exec has no interactive approval mechanism. In practice, `--full-auto` with exec means:
- Files can be written to the workspace
- Commands that the model wants to auto-execute will run
- Commands the model flags for approval will be **rejected**

**The safer choice for headless automation is:**
```bash
codex exec --json -a never --sandbox workspace-write "prompt"
```
This explicitly tells the model "never ask for approval" (which is what exec defaults to anyway).

### --dangerously-bypass-approvals-and-sandbox (alias: --yolo)

- Skips ALL confirmation prompts
- Disables sandboxing entirely
- Also skips the git repo check
- Conflicts with `--full-auto` (can't use both)
- **NEVER use in production without external sandboxing**

### What Gets Auto-Rejected in Exec Mode (from source)

The `handle_server_request` function explicitly rejects these request types:
- `McpServerElicitationRequest` → auto-cancelled (not rejected)
- `CommandExecutionRequestApproval` → rejected: "not supported in exec mode"
- `FileChangeRequestApproval` → rejected: "not supported in exec mode"
- `ToolRequestUserInput` → rejected: "not supported in exec mode"
- `DynamicToolCall` → rejected: "not supported in exec mode"
- `ChatgptAuthTokensRefresh` → rejected: "not supported in exec mode"
- `ApplyPatchApproval` → rejected: "not supported in exec mode"
- `ExecCommandApproval` → rejected: "not supported in exec mode"
- `PermissionsRequestApproval` → rejected: "not supported in exec mode"

---

## --output-schema: Structured Output

### Usage

```bash
# Define schema
cat > schema.json << 'EOF'
{
  "type": "object",
  "properties": {
    "project_name": { "type": "string" },
    "languages": { "type": "array", "items": { "type": "string" } },
    "issues_found": { "type": "integer" }
  },
  "required": ["project_name", "languages"],
  "additionalProperties": false
}
EOF

# Execute with schema enforcement
codex exec --json --output-schema ./schema.json -o ./result.json "Analyze this repository"
```

### How It Works

1. The schema file is read and parsed as JSON at startup (`load_output_schema()` in lib.rs).
2. The parsed schema is passed as `output_schema` in the `TurnStartParams` to the app-server.
3. The model's final response is constrained to match the schema.
4. The structured response appears as the `text` field in an `agent_message` item (as a JSON string).
5. If `-o` is specified, the final message (structured JSON) is written to that file.

### Implications for Keyword Protocol

We can use `--output-schema` to force Codex's final response into a structured format that our Go daemon can parse deterministically. This is MORE reliable than parsing Markdown keywords from free-text. Example schema for our protocol:

```json
{
  "type": "object",
  "properties": {
    "status": { "enum": ["COMPLETE", "BLOCKED", "NEED_INPUT"] },
    "reasoning": { "type": "string" },
    "files_modified": { "type": "array", "items": { "type": "string" } },
    "next_action": { "type": "string" }
  },
  "required": ["status", "reasoning"],
  "additionalProperties": false
}
```

**Note:** `--output-schema` constrains the FINAL agent message only. Intermediate tool calls, commands, and reasoning are still free-form. The schema applies at the Responses API level.

---

## Interruption and Cancellation

### Ctrl+C Handling (from source)

The exec binary installs a Ctrl+C handler via `tokio::signal::ctrl_c()`. On interrupt:

1. The signal handler sends a message to `interrupt_tx` channel.
2. The event loop sends a `turn/interrupt` request to the app-server:
   ```rust
   ClientRequest::TurnInterrupt {
       params: TurnInterruptParams {
           thread_id: primary_thread_id,
           turn_id: task_id,
       },
   }
   ```
3. The app-server interrupts the current turn.
4. The turn completion notification arrives with `TurnStatus::Interrupted`.
5. `error_seen` is set to `true`, process exits with code 1.

### From Go Daemon

To interrupt a running `codex exec` process:
- **Send SIGINT (Ctrl+C equivalent):** `process.Signal(os.Interrupt)` — this triggers the graceful interrupt path.
- **Send SIGTERM:** Should also work via the default Tokio signal handling, but SIGINT is preferred as it's explicitly handled.
- **Send SIGKILL:** Force kill — no graceful cleanup. Use as last resort.

The interrupt is graceful: the model stops generating, any in-progress tool calls are abandoned, and the turn completes with `Interrupted` status. The session state IS still saved (the conversation up to the interruption point is preserved for future `resume`).

### Interruption Timing

Interruption is asynchronous. After sending SIGINT, you should continue reading stdout for the final events:
- `turn.completed` or `turn.failed` with interrupted status
- The process then exits

---

## codex mcp-server: Alternative Integration Path

`codex mcp-server` runs Codex as a persistent stdio MCP server. This is fundamentally different from `codex exec`:

### Tools Exposed

1. **`codex`** — Start a new session
   - Input: `{ prompt, model?, cwd?, approval-policy?, sandbox?, config?, base-instructions?, developer-instructions?, compact-prompt? }`
   - Output: `{ threadId, content }`

2. **`codex-reply`** — Continue an existing session
   - Input: `{ threadId, prompt }`
   - Output: `{ threadId, content }`

### Multi-Turn Pattern

```
Client → tools/call codex { prompt: "analyze repo" }
Server → { threadId: "abc-123", content: "I found 3 issues..." }

Client → tools/call codex-reply { threadId: "abc-123", prompt: "fix issue #1" }
Server → { threadId: "abc-123", content: "Done. I modified src/main.rs..." }
```

### Advantages for Go Daemon

- **Persistent process** — one subprocess, many turns
- **Standard MCP protocol** — we already have Go MCP libraries
- **No re-initialization** between turns (MCP servers stay warm)
- **Built-in multi-turn** via `codex-reply`

### Disadvantages

- **No streaming events** — you get the final output, not intermediate JSONL events
- **No todo list / progress tracking** — just the final content
- **No per-item granularity** — can't see individual commands, file changes, MCP calls as they happen
- **Approval handling** is different — the MCP server has its own elicitation flow

### Recommendation

If we need real-time streaming visibility (which we do for the Triumvirate UI), use `codex exec --json`. If we just need fire-and-forget task execution with multi-turn, `codex mcp-server` is simpler.

---

## codex app-server: WebSocket Integration

**Status: EXPERIMENTAL.** The app-server provides the full internal protocol over WebSocket or stdio:

```bash
codex app-server --listen ws://127.0.0.1:8080 --session-source custom
```

### Protocol

Full JSON-RPC with all thread management operations:
- `thread/start`, `thread/resume`, `thread/list`, `thread/read`, `thread/unsubscribe`
- `turn/start`, `turn/interrupt`
- Server notifications: `item/started`, `item/completed`, `turn/completed`, etc.

This is what the VS Code extension uses. It gives maximum control but is:
- Undocumented publicly
- Marked experimental
- Protocol may change between versions

### Why This Matters

If the app-server stabilizes, it becomes the ideal integration point: persistent process, WebSocket transport, full streaming events, multi-turn, and the complete internal API surface. Worth monitoring.

---

## Known Issues and Regressions

### CRITICAL: MCP Tool Calls Cancelled in Exec Mode (v0.117.0+)

**Issue:** [#16685](https://github.com/openai/codex/issues/16685) — All MCP tool calls are immediately cancelled in `codex exec` mode on v0.117.0 and v0.118.0. Root cause: the `tool_call_mcp_elicitation` feature flag became default-on, which triggers an approval flow that exec mode auto-cancels.

**Status:** Open, confirmed regression by OpenAI maintainer. Works fine on v0.116.0.

**Workaround:** Stay on v0.116.0, or disable the feature: `codex exec --disable tool_call_mcp_elicitation --json "prompt"`

**Impact on us:** HIGH. Our Go daemon will use MCP servers through Codex. This bug blocks us on v0.117.0+.

### Abandoned Items Emitted with Wrong Status

**Issue:** [#14691](https://github.com/openai/codex/issues/14691) — When a turn ends with commands still running:
- Shell commands are emitted as `completed` instead of `failed` (with `exit_code: null`)
- MCP tool calls leave orphaned `item.started` events with no matching `item.completed`

**Impact on us:** MEDIUM. Our JSONL parser must handle: (1) `completed` items with `exit_code: null`, (2) unmatched `item.started` events.

### exec --json Resume Can Hang with MCP Servers

**Issue:** [#14470](https://github.com/openai/codex/issues/14470) — `codex exec --json resume` can hang indefinitely on macOS when MCP helpers fail to initialize. The process sits in a no-output state because `list_all_tools()` blocks waiting for MCP client startup.

**Impact on us:** HIGH. We need timeouts and health checks when using `resume` with MCP servers.

### --json Doesn't Output Reasoning Items with API Key Auth

**Issue:** [#10746](https://github.com/openai/codex/issues/10746) — When authenticating via API key (CODEX_API_KEY), reasoning items are not emitted in JSONL. Only appears with account-based auth.

**Impact on us:** LOW. We use ChatGPT subscription auth, not API keys.

### Broken Pipe Panic on macOS

**Issue:** [#10248](https://github.com/openai/codex/issues/10248) — Codex panics with `BrokenPipe` error when stdout closes during `--json` streaming (e.g., consumer process dies).

**Impact on us:** MEDIUM. Our Go daemon must handle the Codex process crashing if we close our reader end. Use `cmd.Wait()` to detect exit, don't just read stdout.

### exec with danger-full-access Still Hits EPERM

**Issue:** [#15696](https://github.com/openai/codex/issues/15696) — Even with `danger-full-access`, exec mode can hit permission errors that interactive mode doesn't.

**Impact on us:** LOW. We plan to use `workspace-write`, not `danger-full-access`.

---

## Missing --quiet Flag

**There is no `--quiet` flag in `codex exec`.** I searched the entire Rust source — no quiet mode exists. The user's question about `--quiet` appears to reference something that doesn't exist (or was removed/never merged).

What DOES exist for controlling output verbosity:

| Mechanism | Effect |
|-----------|--------|
| `--json` | Switches stdout from final-message to JSONL events |
| `--color never` | Disables ANSI color codes |
| `-o FILE` | Writes final message to file |
| `--ephemeral` | Suppresses session file persistence |
| `hide_agent_reasoning = true` (config) | Suppresses reasoning output in human mode |
| `model_reasoning_summary = "none"` (config) | Requests no reasoning summaries from model |

In `--json` mode, stderr still receives progress/warning output. To truly silence stderr:
```bash
codex exec --json "prompt" 2>/dev/null
```

---

## Integration Recommendations for Go Daemon

### Recommended Integration: codex exec --json as Subprocess

```go
// Spawn Codex for a task
cmd := exec.Command("codex", "exec", "--json",
    "--sandbox", "workspace-write",
    "-a", "never",
    "--skip-git-repo-check",
    "-C", workDir,
    "-o", lastMessageFile,
    prompt,
)
cmd.Env = append(os.Environ(),
    "CODEX_API_KEY="+apiKey,  // or use auth.json for subscription
    "CODEX_HOME="+codexHome,
)
cmd.Stderr = os.Stderr // or capture for logging

stdout, _ := cmd.StdoutPipe()
cmd.Start()

scanner := bufio.NewScanner(stdout)
for scanner.Scan() {
    var event ThreadEvent
    json.Unmarshal(scanner.Bytes(), &event)
    // Route event to Triumvirate event bus
    switch event.Type {
    case "thread.started":
        threadID = event.ThreadID
    case "item.completed":
        handleItem(event.Item)
    case "turn.completed":
        handleUsage(event.Usage)
    case "turn.failed":
        handleError(event.Error)
    }
}
cmd.Wait()
```

### Multi-Turn Pattern

```go
// First turn
threadID := runCodexExec(ctx, workDir, "analyze the codebase")

// Follow-up turn (separate process)
runCodexExecResume(ctx, workDir, threadID, "now fix the issues you found")
```

### System Prompt Injection

Write `AGENTS.md` to the workspace before invoking:

```go
agentsMD := fmt.Sprintf(`# Triumvirate Protocol

You are operating as part of the Triumvirate multi-agent system.

## Response Protocol
Your final response MUST be valid JSON matching this schema:
%s

## Constraints
- Do not modify files outside the workspace
- Report all file changes in the response
- If blocked, set status to "BLOCKED" with explanation
`, schemaJSON)

os.WriteFile(filepath.Join(workDir, "AGENTS.md"), []byte(agentsMD), 0644)
```

### Structured Output

For deterministic parsing, always use `--output-schema`:

```go
cmd := exec.Command("codex", "exec", "--json",
    "--output-schema", schemaFile,
    "-o", resultFile,
    "--sandbox", "workspace-write",
    "-a", "never",
    "-C", workDir,
    prompt,
)
```

### Interruption

```go
// Graceful interrupt
cmd.Process.Signal(os.Interrupt)

// Wait for clean shutdown with timeout
done := make(chan error, 1)
go func() { done <- cmd.Wait() }()
select {
case <-done:
    // Clean exit
case <-time.After(10 * time.Second):
    cmd.Process.Kill() // Force kill
}
```

### Timeout Safety

Always wrap exec calls with a timeout. Known hang conditions:
- MCP server startup failure during resume (#14470)
- Stalled turn with no completion event (#14462)

```go
ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
defer cancel()
cmd := exec.CommandContext(ctx, "codex", "exec", "--json", ...)
```

### Version Pinning

**Pin to v0.116.0** until the MCP elicitation regression (#16685) is fixed. Test each upgrade against MCP tool call functionality.

### Config Overrides via CLI

Instead of managing config.toml files, pass overrides via `-c`:

```bash
codex exec --json \
  -c 'model="gpt-5.2-codex"' \
  -c 'sandbox_workspace_write.network_access=true' \
  -c 'model_reasoning_effort="high"' \
  -c 'project_doc_max_bytes=65536' \
  "prompt here"
```

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `CODEX_API_KEY` | API key auth (alternative to subscription) |
| `CODEX_HOME` | Override config directory (default: `~/.codex`) |
| `CODEX_SQLITE_HOME` | Override SQLite state DB location |
| `CODEX_CA_CERTIFICATE` | Custom CA bundle for enterprise proxies |
| `SSL_CERT_FILE` | Fallback CA bundle |

---

## Appendix A: Full Config.toml Reference (Exec-Relevant Keys)

```toml
# Model
model = "gpt-5.2-codex"
model_provider = "openai"
model_reasoning_effort = "high"    # minimal | low | medium | high | xhigh
model_reasoning_summary = "concise" # auto | concise | detailed | none
model_verbosity = "medium"          # low | medium | high
model_context_window = 128000
model_auto_compact_token_limit = 80000

# Sandbox
sandbox_mode = "workspace-write"   # read-only | workspace-write | danger-full-access

[sandbox_workspace_write]
writable_roots = ["/extra/path"]
network_access = true
exclude_slash_tmp = false
exclude_tmpdir_env_var = false

# Approval
approval_policy = "never"          # untrusted | on-request | never

# AGENTS.md
project_doc_max_bytes = 32768
project_doc_fallback_filenames = ["CLAUDE.md"]

# Developer instructions (separate from AGENTS.md)
developer_instructions = "Always respond in JSON"

# Shell environment
[shell_environment_policy]
inherit = "all"                    # all | core | none
set = { MY_VAR = "value" }
exclude = ["AWS_*", "SECRET_*"]

# MCP servers
[mcp_servers.myserver]
command = "node"
args = ["/path/to/server.js"]
enabled = true
required = true                    # exec exits if this server fails to start
startup_timeout_sec = 30
tool_timeout_sec = 120

# Features
[features]
shell_tool = true
multi_agent = true
child_agents_md = false
codex_hooks = false
```

## Appendix B: Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success — turn completed normally |
| 1 | Error — turn failed, interrupted, or fatal error occurred |
| 1 | Not in git repo and `--skip-git-repo-check` not set |
| 1 | Config load failure |
| 1 | Required MCP server failed to initialize |

There is no differentiation between error types via exit code. Parse the JSONL stream for specifics.

## Appendix C: Process Lifecycle Summary

```
Parent (Go daemon)
  │
  ├── spawn: codex exec --json --sandbox workspace-write -a never -C /work "prompt"
  │     │
  │     ├── stdout: {"type":"thread.started","thread_id":"<uuid>"}
  │     ├── stdout: {"type":"turn.started"}
  │     ├── stdout: {"type":"item.started",...}   (commands, MCP calls)
  │     ├── stdout: {"type":"item.completed",...}
  │     ├── stdout: {"type":"item.completed","item":{"type":"agent_message","text":"..."}}
  │     ├── stdout: {"type":"turn.completed","usage":{...}}
  │     └── exit 0
  │
  ├── (later) spawn: codex exec --json resume <uuid> "follow up"
  │     │
  │     ├── stdout: {"type":"thread.started","thread_id":"<uuid>"}
  │     ├── stdout: {"type":"turn.started"}
  │     ├── ... (more events)
  │     ├── stdout: {"type":"turn.completed","usage":{...}}
  │     └── exit 0
  │
  └── (interrupt) SIGINT → graceful shutdown
        ├── stdout: {"type":"turn.completed",...} or {"type":"turn.failed",...}
        └── exit 1
```
