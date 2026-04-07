# 030 -- Gemini CLI Deep Dive: Programmatic Control for Go Daemon Integration

**Date:** 2026-04-03
**Source:** github.com/google-gemini/gemini-cli (main branch), geminicli.com docs, source code analysis
**Purpose:** Determine exactly what Gemini CLI can and cannot do as a persistent subprocess controlled by our Go triumvirate daemon.

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Output Formats](#output-formats)
3. [Headless Mode (-p)](#headless-mode--p)
4. [ACP Mode (--acp) -- The Real Answer](#acp-mode---acp----the-real-answer)
5. [GEMINI_SYSTEM_MD](#gemini_system_md)
6. [Interruption / Cancellation](#interruption--cancellation)
7. [Stdin Piping](#stdin-piping)
8. [Session State & Resume](#session-state--resume)
9. [Sandboxing](#sandboxing)
10. [Known Bugs & Limitations](#known-bugs--limitations)
11. [Daemon Mode (Community PR)](#daemon-mode-community-pr)
12. [Integration Strategies for Our Go Daemon](#integration-strategies-for-our-go-daemon)
13. [Complete CLI Flag Reference](#complete-cli-flag-reference)
14. [Complete Environment Variable Reference](#complete-environment-variable-reference)

---

## Executive Summary

Gemini CLI has **three** operational modes relevant to us:

| Mode | Multi-Turn | Session State | Structured Output | Tool Calls | Persistent Process |
|------|-----------|---------------|-------------------|------------|-------------------|
| **Headless (`-p`)** | NO (single-shot) | NO (exits after response) | YES (json, stream-json) | YES (auto-approved in yolo) | NO |
| **ACP (`--acp`)** | YES | YES (per-session) | YES (JSON-RPC over stdio) | YES (with approval flow) | YES |
| **Interactive (TUI)** | YES | YES | NO (terminal UI) | YES | YES |

**Bottom line:** For our Go daemon, **ACP mode is the correct integration path**. It provides persistent multi-turn sessions over stdio JSON-RPC, supports cancellation, session loading/saving, model switching, and approval mode changes -- all programmatically. Headless mode is single-shot only and exits after each invocation.

---

## Output Formats

### `--output-format text` (default)
Plain text response to stdout. Errors/warnings go to stderr. ANSI stripped unless `--raw-output` is used.

### `--output-format json`
Single JSON object after full completion. **Not streaming -- waits until entire response is done.**

```json
{
  "session_id": "uuid-here",
  "response": "The model's final answer as a string",
  "stats": {
    "models": {
      "gemini-3-pro": {
        "api": { "totalRequests": 1, "totalErrors": 0, "totalLatencyMs": 1234 },
        "tokens": { "input": 50, "prompt": 50, "candidates": 30, "total": 80, "cached": 0, "thoughts": 0, "tool": 0 }
      }
    },
    "tools": { "totalCalls": 2, "totalSuccess": 2, "totalFail": 0, "totalDurationMs": 500 },
    "files": { "totalLinesAdded": 0, "totalLinesRemoved": 0 }
  }
}
```

On error:
```json
{
  "session_id": "uuid-here",
  "error": {
    "type": "FatalAuthenticationError",
    "message": "Authentication required",
    "code": 41
  }
}
```

### `--output-format stream-json` (JSONL)
**Newline-delimited JSON events emitted in real-time to stdout.** This is what we'd use for streaming in headless mode. Each line is one JSON object.

#### Event Type Schema (from `packages/core/src/output/types.ts`)

**All events share a base:**
```typescript
interface BaseJsonStreamEvent {
  type: JsonStreamEventType;   // enum string
  timestamp: string;           // ISO 8601
}
```

**1. `init` -- First event, emitted once at start:**
```json
{"type":"init","timestamp":"2026-04-03T12:00:00.000Z","session_id":"uuid","model":"gemini-3-pro"}
```

**2. `message` -- User input echo and assistant response chunks:**
```json
{"type":"message","timestamp":"...","role":"user","content":"What is 2+2?"}
{"type":"message","timestamp":"...","role":"assistant","content":"The answer","delta":true}
```
- `role`: `"user"` or `"assistant"`
- `delta`: `true` when this is a streaming chunk (assistant responses). Absent/undefined for complete messages (user input).
- Assistant messages arrive as **multiple delta events** -- you must concatenate them.

**3. `tool_use` -- Model requests a tool call:**
```json
{"type":"tool_use","timestamp":"...","tool_name":"Read","tool_id":"read-123","parameters":{"file_path":"/path/to/file.txt"}}
```
- `parameters`: `Record<string, unknown>` -- arbitrary JSON matching the tool's schema.

**4. `tool_result` -- Tool execution result:**
```json
{"type":"tool_result","timestamp":"...","tool_id":"read-123","status":"success","output":"File contents here"}
{"type":"tool_result","timestamp":"...","tool_id":"read-123","status":"error","error":{"type":"FILE_NOT_FOUND","message":"File not found"}}
```

**5. `error` -- Non-fatal diagnostic events:**
```json
{"type":"error","timestamp":"...","severity":"warning","message":"Loop detected, stopping execution"}
{"type":"error","timestamp":"...","severity":"error","message":"Maximum session turns exceeded"}
```

**6. `result` -- Final event, emitted once at end:**
```json
{
  "type": "result",
  "timestamp": "...",
  "status": "success",
  "stats": {
    "total_tokens": 100,
    "input_tokens": 50,
    "output_tokens": 50,
    "cached": 0,
    "input": 50,
    "duration_ms": 1200,
    "tool_calls": 2,
    "models": {
      "gemini-3-pro": {
        "total_tokens": 100,
        "input_tokens": 50,
        "output_tokens": 50,
        "cached": 0,
        "input": 50
      }
    }
  }
}
```

On error:
```json
{
  "type": "result",
  "timestamp": "...",
  "status": "error",
  "error": { "type": "MaxSessionTurnsError", "message": "Maximum session turns exceeded" },
  "stats": { ... }
}
```

**Complete TypeScript union type:**
```typescript
type JsonStreamEvent = InitEvent | MessageEvent | ToolUseEvent | ToolResultEvent | ErrorEvent | ResultEvent;
```

---

## Headless Mode (-p)

### Trigger Conditions (from `packages/core/src/utils/headless.ts`)

Headless mode activates when ANY of:
1. `-p` / `--prompt` flag is provided
2. `process.stdout.isTTY` is false (piped output)
3. `process.stdin.isTTY` is false (piped input)
4. `CI=true` environment variable
5. `GITHUB_ACTIONS=true` environment variable
6. Positional query argument provided

### Single-Shot Only -- NO Multi-Turn

**This is the critical limitation.** From source code analysis of `nonInteractiveCli.ts`:

```typescript
// The main loop in nonInteractiveCli.ts
while (true) {
  // ... send message, get response, handle tool calls ...
  if (toolCallRequests.length > 0) {
    // Execute tools, send results back to model, continue loop
    currentMessages = [{ role: 'user', parts: toolResponseParts }];
  } else {
    // NO MORE TOOL CALLS = DONE. Emit result and return.
    return;
  }
}
```

The "multi-turn" loop in headless mode is **only for tool call rounds within a single prompt**. Once the model produces a final text response with no tool calls, the process exits. There is no mechanism to send a follow-up prompt.

### Stdin Behavior

From `readStdin.ts`: When stdin is not a TTY, the CLI reads **all available data** from stdin (up to 8MB, with a 500ms timeout for data availability). This data is **prepended** to the `-p` prompt. This is one-shot -- it reads once and stops.

```bash
# stdin content becomes context, -p prompt becomes the question
echo "context data here" | gemini -p "analyze this" --output-format stream-json
```

**Critical:** If stdin is not a TTY and no data arrives within 500ms, the CLI errors: "No input provided via stdin" and exits with code 1. You cannot keep stdin open for continuous prompting.

### `--prompt-interactive` (-i) Flag

There IS a `-i`/`--prompt-interactive` flag that sends an initial prompt but **stays in interactive TUI mode**. However:
- It cannot be used when stdin is piped (explicitly checks `process.stdin.isTTY`)
- It opens the full interactive TUI -- not suitable for programmatic control
- Cannot be combined with `--output-format`

---

## ACP Mode (--acp) -- The Real Answer

### What It Is

ACP (Agent Client Protocol) is an open protocol for IDE-to-agent communication over **JSON-RPC 2.0 on stdio**. Gemini CLI implements the agent side. Our Go daemon would implement the client side.

### How to Start
```bash
gemini --acp
```

The process stays alive, listening on stdin for JSON-RPC messages and responding on stdout.

### Protocol: JSON-RPC 2.0 over stdio

Uses the `@agentclientprotocol/sdk` npm package. Communication is newline-delimited JSON-RPC (nd-json).

### Available Methods

From `acpClient.ts` source code:

| Method | Purpose |
|--------|---------|
| `initialize` | Handshake. Returns capabilities, auth methods, agent info. |
| `authenticate` | Set auth method (OAuth, API key, Vertex AI, Gateway). |
| `newSession` | Create a new chat session. Returns session ID, available models/modes. |
| `loadSession` | Resume a previous session by ID. Restores full conversation history. |
| `prompt` | Send a prompt to the agent within a session. Returns streaming events. |
| `cancel` | Cancel an in-progress prompt. |
| `setSessionMode` | Change approval mode (default, auto_edit, yolo, plan). |
| `unstable_setSessionModel` | Switch the model for the session. |

### Multi-Turn Conversations

**YES.** ACP mode fully supports multi-turn. You call `newSession` once, then send multiple `prompt` calls. The session maintains full conversation history on the Gemini CLI side.

### Session Persistence

Sessions can be saved and loaded. `loadSession` accepts a session ID and restores the full conversation, including tool history. The ACP client code explicitly uses `SessionSelector` and `convertSessionToClientHistory` to rebuild state.

### Tool Call Approval

In ACP mode, tool calls go through the standard approval flow. The agent sends a notification to the client asking for permission. The client responds with approval/rejection. In `yolo` mode (set via `setSessionMode`), tools are auto-approved.

### MCP Server Integration

ACP supports passing MCP server definitions during `newSession`. The client can expose its own MCP servers, and Gemini CLI will discover and use their tools.

### File System Proxy

ACP implements a proxied file system -- file operations go through the client. This is how IDEs maintain control over file access.

### Agent Capabilities (from initialize response)

```typescript
{
  agentCapabilities: {
    loadSession: true,
    promptCapabilities: {
      image: true,
      audio: true,
      embeddedContext: true,
    },
    mcpCapabilities: {
      http: true,
      sse: true,
    },
  }
}
```

### Known Issues with ACP

**Issue #22647 (OPEN):** `--acp` mode outputs plain text to stdout (like "Loaded cached credentials."), corrupting the JSON-RPC stream. This is a known bug that can cause parse failures in clients. Workaround: filter non-JSON lines from stdout.

---

## GEMINI_SYSTEM_MD

### How It Works

`GEMINI_SYSTEM_MD` is an environment variable that **completely replaces** the built-in system prompt. It is NOT additive -- it is a full replacement.

### Configuration Options

| Value | Behavior |
|-------|----------|
| `true` or `1` | Loads `./.gemini/system.md` from the current working directory |
| `/path/to/file.md` | Loads the specified file (absolute or relative, supports tilde expansion) |
| `false` or `0` | Disables the override (uses built-in prompt) |

### Dynamic Variable Substitution in Custom System Prompts

Custom system prompt files support placeholders:
- `${AgentSkills}` -- injects agent skills section
- `${SubAgents}` -- injects sub-agents section
- `${AvailableTools}` -- injects bulleted tool list
- `${toolName_ToolName}` -- dynamic tool name references

### Can We Change It Between Turns?

**NO -- not within a single process.** The system prompt is loaded once during startup (in `loadCliConfig`). It is baked into the `Config` object and used for all subsequent API calls.

**However**, in ACP mode, each `newSession` call creates a fresh `Config` with its own system prompt load. So if you:
1. Set `GEMINI_SYSTEM_MD` to point to a file
2. Modify that file between sessions
3. Call `newSession`

...the new session will use the updated system prompt. But you cannot change it mid-session.

**In headless mode**, since each invocation is a separate process, you can change `GEMINI_SYSTEM_MD` between invocations trivially.

### GEMINI_WRITE_SYSTEM_MD

Setting `GEMINI_WRITE_SYSTEM_MD=1` before running Gemini will export the current built-in system prompt to `./.gemini/system.md`. Useful for seeing what you're replacing.

### GEMINI.md vs System Prompt

They serve different purposes:
- **GEMINI.md** (context file): Persona, goals, project context. Hierarchical (global -> workspace -> JIT). Additive.
- **system.md** (via GEMINI_SYSTEM_MD): Non-negotiable operational rules, safety, tool protocols. Full replacement.

Both are loaded. GEMINI.md content is concatenated and sent alongside the system prompt, not replacing it.

---

## Interruption / Cancellation

### Headless Mode

From `nonInteractiveCli.ts`: The CLI sets up an `AbortController` and listens for Ctrl+C (character code `\u0003`) via raw stdin keypress events.

```typescript
// Simplified from source
const abortController = new AbortController();
process.stdin.on('keypress', (str, key) => {
  if ((key && key.ctrl && key.name === 'c') || str === '\u0003') {
    abortController.abort();
  }
});
```

**Can we cancel without killing the process?** In headless mode, **effectively no** -- once the abort fires, the process emits a result event and exits with code 130 (FATAL_CANCELLATION_ERROR). The abort is one-way.

**SIGINT/SIGTERM:** Sending SIGINT (Ctrl+C) to the process will trigger the abort flow. SIGTERM triggers cleanup handlers.

### ACP Mode

**YES.** ACP exposes a `cancel` method that can abort an in-progress prompt without terminating the process. The session remains alive for subsequent prompts.

### Can We Cancel Mid-Stream Without Killing?

| Mode | Cancel Without Kill | Mechanism |
|------|-------------------|-----------|
| Headless | NO | Abort -> exit(130) |
| ACP | YES | `cancel` JSON-RPC method |

---

## Stdin Piping

### Headless Mode Behavior

1. **Piped data + `-p` flag:** stdin data is prepended to the prompt. Both are sent as a single user message.
2. **Piped data, no `-p` flag:** stdin data becomes the entire prompt.
3. **No piped data, no `-p` flag:** Error, exit 1.
4. **Stdin size limit:** 8MB (truncated with warning if exceeded).
5. **Timeout:** If stdin is not a TTY and no data arrives within 500ms, the CLI errors out.

### Continuous Piping (Feeding Multiple Prompts)

**NOT SUPPORTED in headless mode.** The `readStdin()` function reads once and resolves. There is no mechanism to read multiple delimited prompts from stdin. The process runs one prompt and exits.

### ACP Mode Piping

ACP reads from stdin **continuously** using `Readable.toWeb(process.stdin)` and processes JSON-RPC messages as they arrive. This IS the persistent subprocess pattern we need.

---

## Session State & Resume

### How Sessions Are Stored

- Location: `~/.gemini/tmp/<project_hash>/chats/`
- Project hash: derived from the project root directory
- Content: Complete conversation history including prompts, responses, tool executions, token stats, reasoning traces

### Resume Capabilities

```bash
gemini --resume              # Most recent session
gemini --resume 1            # By index
gemini --resume <uuid>       # By session ID
```

### Headless Mode + Resume

**YES, you can combine:** `gemini -p "continue the work" --resume <uuid> --output-format stream-json`

From source code, `runNonInteractive` receives `resumedSessionData` and calls `geminiClient.resumeChat()` with the restored history. The new prompt is sent in the context of the restored session.

**But it's still single-shot.** After the response completes, the process exits. Each invocation with `--resume` pays the full startup cost.

### ACP Mode + Sessions

ACP has `loadSession` which restores a previous session into a persistent process. You can then send multiple prompts. This avoids the startup penalty.

### Session State Between Headless Invocations

Sessions ARE saved even in headless mode (via `ChatRecordingService`). So you could theoretically:
1. Run `gemini -p "do step 1" --output-format json` -> get session_id from response
2. Run `gemini -p "do step 2" --resume <session_id> --output-format json`

But each invocation is a cold start (Node.js process boot, config load, auth, MCP server startup).

### Startup Penalty

This is significant. From issue #15338 comments: the startup penalty is the #1 reason people want a daemon mode. Each headless invocation pays:
- Node.js process startup
- Settings/config loading
- Authentication (even if cached)
- MCP server initialization
- Context file discovery and loading

---

## Sandboxing

### Overview

Sandboxing isolates shell commands and file modifications from the host system. Multiple backends available.

### Sandbox Methods

| Method | Platform | Isolation Level | Command |
|--------|----------|----------------|---------|
| **Seatbelt** (`sandbox-exec`) | macOS only | Process-level (lightweight) | Built-in, default on macOS |
| **Docker** | Cross-platform | Container (full) | `GEMINI_SANDBOX=docker` |
| **Podman** | Cross-platform | Container (full) | `GEMINI_SANDBOX=podman` |
| **gVisor** (`runsc`) | Linux only | User-space kernel (strongest) | `GEMINI_SANDBOX=runsc` |
| **LXC/LXD** | Linux only | Full-system container | `GEMINI_SANDBOX=lxc` |
| **Windows Native** | Windows only | Integrity levels | Built-in |

### Activation (precedence order)

1. **CLI flag:** `-s` / `--sandbox`
2. **Environment variable:** `GEMINI_SANDBOX=true|docker|podman|sandbox-exec|runsc|lxc`
3. **Settings file:** `settings.json` -> `tools.sandbox`

### Seatbelt Profiles (macOS)

Set via `SEATBELT_PROFILE` environment variable:

| Profile | Writes | Network |
|---------|--------|---------|
| `permissive-open` (default) | Restricted outside project | Allowed |
| `permissive-proxied` | Restricted outside project | Via proxy |
| `restrictive-open` | Strict | Allowed |
| `restrictive-proxied` | Strict | Via proxy |
| `strict-open` | Read+write restricted | Allowed |
| `strict-proxied` | Read+write restricted | Via proxy |

### Tool-Level Sandboxing

Separate from process-level. Provides granular isolation for individual tool executions (`shell_exec`, `write_file`). Toggle via:
```json
{ "security": { "toolSandboxing": true } }
```

### For Our Go Daemon

We probably want `--sandbox` or `GEMINI_SANDBOX=true` to restrict Gemini's file/shell operations. In Docker environments, use container sandboxing. On macOS dev machines, Seatbelt is automatic.

**Important:** Sandbox + stdin has a known issue. From the stdin integration test comments: "This test currently fails in sandbox mode (Docker/Podman) because stdin content is not properly forwarded to the container when used together with a --prompt argument."

---

## Known Bugs & Limitations

### Critical for Our Use Case

| Issue | Status | Impact |
|-------|--------|--------|
| **#22647**: ACP mode outputs plain text to stdout, corrupting JSON-RPC stream | OPEN | Must filter non-JSON from stdout in our Go parser |
| **#18292**: stream-json with trivial prompts enters infinite tool_use loop | OPEN | Need to implement turn limits or loop detection on our side |
| **#20859**: AgentExecutionBlocked produces no output in STREAM_JSON mode | OPEN | Blocked events are silently swallowed -- we won't know execution was blocked |
| **#20183**: Auth errors in stream-json produce no structured RESULT event | OPEN | Auth failures may produce no parseable output |
| **#20453**: AgentExecutionStopped produces no output in JSON mode | OPEN | Stop events may be invisible |
| **#24432**: Session logs store full base64 inline binary data, causing multi-GB disk and OOM | OPEN | Session resume can OOM on large sessions with images |

### Feature Gaps

| Issue | Status | Description |
|-------|--------|-------------|
| **#15338**: Feature request for stateful headless daemon mode | OPEN | Community wants exactly what we're building |
| **#24058**: Reasoning traces not exposed in stream-json | OPEN | Can't see model thinking in headless mode |
| **#22083**: Model thinking events not in stream-json | OPEN | Same as above |
| **#21833**: stream-json should include per-model usage stats like json format | OPEN | Stats schema differs between json and stream-json |

### PR #20700: Community Daemon Mode Implementation

**Status: OPEN (not merged).** Adds `--daemon`, `--daemon-status`, `--daemon-stop` flags with Unix socket communication and named sessions. Not yet in mainline. This is exactly the pattern we'd use -- but since it's not merged, we should build our own Go wrapper around ACP mode instead.

---

## Daemon Mode (Community PR)

PR #20700 proposes:

```bash
# Start daemon
gemini --daemon

# Send prompt to named session
gemini --client --session test "what is 2+2"

# Multi-turn with context
gemini --client --session test "multiply that by 4"

# Stop daemon
gemini --daemon-stop
```

Uses Unix sockets for IPC. Not merged as of 2026-04-03. The PR is actively being reviewed but has been open for some time. We should not depend on this.

---

## Integration Strategies for Our Go Daemon

### Strategy A: ACP Mode (Recommended)

**Architecture:**
```
Go Daemon -> spawn `gemini --acp` as child process
          -> communicate via JSON-RPC 2.0 over stdin/stdout
          -> manage sessions, prompts, cancellation via ACP protocol
```

**Pros:**
- Official, stable protocol (used by VS Code, JetBrains, Zed)
- Full multi-turn support
- Session persistence and resume
- Cancellation without process kill
- MCP server passthrough
- Model and mode switching mid-session
- Approval flow support

**Cons:**
- JSON-RPC is more complex than simple JSONL parsing
- Known stdout corruption bug (#22647) -- need to handle gracefully
- No streaming content deltas (ACP sends complete response events, not token-by-token streaming)
- Requires implementing ACP client protocol in Go

**Implementation:**
1. Implement ACP client in Go (JSON-RPC 2.0 over stdio, using nd-json framing)
2. Use `@agentclientprotocol/sdk` as reference for message schemas
3. Handle stdout corruption by parsing lines: skip non-JSON, parse JSON-RPC only
4. Manage sessions via `newSession`/`loadSession`
5. Use `setSessionMode` to enable yolo mode for automated operation

### Strategy B: Headless Mode with Session Resume (Fallback)

**Architecture:**
```
Go Daemon -> spawn `gemini -p "prompt" --resume <id> --output-format stream-json --yolo`
          -> parse JSONL events from stdout
          -> kill and restart for each turn
```

**Pros:**
- Simpler parsing (JSONL vs JSON-RPC)
- Well-understood output schema (6 event types)
- Each invocation is isolated

**Cons:**
- Cold start penalty per turn (several seconds)
- No mid-stream cancellation (must SIGKILL)
- Session resume adds overhead
- No model/mode switching between turns without env var changes
- Tool calls may trigger infinite loops (#18292)
- Multiple missing event types in edge cases (#20859, #20183, #20453)

### Strategy C: Hybrid (Pragmatic)

Use ACP for the persistent Gemini subprocess. Use headless mode for one-off queries that don't need session state (e.g., quick classifications, summaries).

---

## Complete CLI Flag Reference

| Flag | Alias | Type | Description |
|------|-------|------|-------------|
| `--prompt` | `-p` | string | Non-interactive mode with given prompt |
| `--prompt-interactive` | `-i` | string | Execute prompt, stay in interactive mode |
| `--model` | `-m` | string | Specify Gemini model |
| `--output-format` | `-o` | text/json/stream-json | Output format |
| `--sandbox` | `-s` | boolean | Enable sandboxing |
| `--yolo` | `-y` | boolean | Auto-approve all tool calls |
| `--approval-mode` | | default/auto_edit/yolo/plan | Set approval level |
| `--acp` | | boolean | Start in ACP mode |
| `--experimental-acp` | | boolean | Deprecated alias for --acp |
| `--resume` | `-r` | string/empty | Resume session (latest, index, or UUID) |
| `--list-sessions` | | boolean | List all sessions |
| `--delete-session` | | string | Delete a session by index or UUID |
| `--debug` | `-d` | boolean | Enable debug mode |
| `--include-directories` | | string[] | Additional workspace directories |
| `--worktree` | `-w` | string | Git worktree name |
| `--policy` | | string[] | Additional policy files |
| `--admin-policy` | | string[] | Admin policy files |
| `--allowed-mcp-server-names` | | string[] | MCP server allowlist (deprecated) |
| `--allowed-tools` | | string[] | Tool allowlist (deprecated) |
| `--extensions` | | string[] | Load extensions |
| `--list-extensions` | | boolean | List installed extensions |
| `--screen-reader` | | boolean | Accessibility mode |
| `--raw-output` | | boolean | Don't strip ANSI from output |
| `--accept-raw-output-risk` | | boolean | Suppress raw output warning |
| `--use-write-todos` | | boolean | Enable write_todos tool |
| `--fake-responses` | | string | Path to fake responses (testing) |
| `--record-responses` | | string | Record responses to file (testing) |

---

## Complete Environment Variable Reference

### Authentication
| Variable | Description |
|----------|-------------|
| `GEMINI_API_KEY` | Gemini API key |
| `GOOGLE_API_KEY` | Google Cloud API key (Vertex AI) |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to service account JSON |
| `GOOGLE_CLOUD_PROJECT` | GCP project ID |
| `GOOGLE_CLOUD_LOCATION` | GCP region (e.g., us-central1) |
| `GOOGLE_GENAI_USE_VERTEXAI` | Enable Vertex AI mode (true) |

### Model & Behavior
| Variable | Description |
|----------|-------------|
| `GEMINI_MODEL` | Override default model |
| `GOOGLE_GENAI_API_VERSION` | API version |

### System Prompt
| Variable | Description |
|----------|-------------|
| `GEMINI_SYSTEM_MD` | Custom system prompt: `true`/`1` -> `./.gemini/system.md`, path -> file |
| `GEMINI_WRITE_SYSTEM_MD` | Export built-in prompt: `true`/`1` -> `./.gemini/system.md`, path -> file |

### Sandboxing
| Variable | Description |
|----------|-------------|
| `GEMINI_SANDBOX` | Enable: `true`, `docker`, `podman`, `sandbox-exec`, `runsc`, `lxc` |
| `SEATBELT_PROFILE` | macOS profile: permissive-open, restrictive-open, strict-open, etc. |
| `SANDBOX_FLAGS` | Custom Docker/Podman flags |
| `SANDBOX_SET_UID_GID` | Control UID/GID mapping in containers |

### CLI Configuration
| Variable | Description |
|----------|-------------|
| `GEMINI_CLI_HOME` | Override config root directory |
| `GEMINI_CLI_IDE_PID` | IDE process PID for integration |
| `GEMINI_CLI_SURFACE` | Custom User-Agent label |
| `NO_COLOR` | Disable all color output |
| `CLI_TITLE` | Custom CLI title |
| `CODE_ASSIST_ENDPOINT` | Code assist server endpoint |

### Telemetry
| Variable | Description |
|----------|-------------|
| `GEMINI_TELEMETRY_ENABLED` | Enable telemetry (true/1) |
| `GEMINI_TELEMETRY_TARGET` | Target: local or gcp |
| `GEMINI_TELEMETRY_OTLP_ENDPOINT` | OTLP endpoint |
| `GEMINI_TELEMETRY_OTLP_PROTOCOL` | grpc or http |
| `GEMINI_TELEMETRY_LOG_PROMPTS` | Log user prompts |
| `GEMINI_TELEMETRY_OUTFILE` | Local telemetry output file |
| `GEMINI_TELEMETRY_USE_COLLECTOR` | Use external OTLP collector |

### Debug
| Variable | Description |
|----------|-------------|
| `DEBUG` / `DEBUG_MODE` | Enable verbose debug logging |

### Headless Detection
| Variable | Description |
|----------|-------------|
| `CI` | Set to "true" forces headless mode |
| `GITHUB_ACTIONS` | Set to "true" forces headless mode |

---

## Exit Codes

| Code | Name | Meaning |
|------|------|---------|
| 0 | SUCCESS | Normal exit |
| 1 | (default) | General/API error |
| 41 | FATAL_AUTHENTICATION_ERROR | Auth failure |
| 42 | FATAL_INPUT_ERROR | Invalid prompt/arguments |
| 44 | FATAL_SANDBOX_ERROR | Sandbox failure |
| 52 | FATAL_CONFIG_ERROR | Configuration error |
| 53 | FATAL_TURN_LIMITED_ERROR | Max session turns exceeded |
| 54 | FATAL_TOOL_EXECUTION_ERROR | Fatal tool error |
| 130 | FATAL_CANCELLATION_ERROR | User cancelled (Ctrl+C / abort) |

---

## Key Decisions for Our Architecture

1. **Use ACP mode** for the persistent Gemini subprocess. It's the only mode that gives us multi-turn, cancellation, and session management without process restart.

2. **Implement a Go ACP client** following the `@agentclientprotocol/sdk` protocol. The protocol is JSON-RPC 2.0 over nd-json stdio.

3. **Handle stdout corruption** (issue #22647) by implementing a resilient JSON-RPC parser that skips non-JSON lines.

4. **Use `setSessionMode("yolo")`** for automated operation where tool calls should be auto-approved.

5. **Use `GEMINI_SYSTEM_MD`** pointing to a file we control. Update the file between sessions (via `newSession`) to inject dynamic context.

6. **Fall back to headless mode** for simple one-shot queries where session state is unnecessary and startup cost is acceptable.

7. **Implement turn limits** on our side to prevent infinite loops (issue #18292).

8. **Monitor for missing events** in stream-json (issues #20859, #20183, #20453) and handle timeouts/hangs gracefully.
