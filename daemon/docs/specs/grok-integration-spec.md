<!--
Provenance: supplied by the owner 2026-08-30 as GROK_ADAPTER_IMPLEMENTATION_GUIDE.md.
Landed here unmodified below the divider, matching the docs/specs/{agent}-integration-spec.md
convention already used by agy and deepseek.

VERIFIED AGAINST THE REPO 2026-08-30 before landing. Every file it names exists (15/15).
Nothing is started: no grok.rs anywhere in the tree. Cited line numbers are accurate within
the "~" the guide uses (run_named_agent_with_session_and_model 1516 vs ~1529,
run_agent_process_with_session 2596 vs ~2620, cli_ops 223 exact, main.rs 2175 and 4059 exact,
inter_agent 313 exact). schedule_len_for exists at agent_exec.rs:2977 as a test-local closure,
and already asserts deepseek == 1, so the guide's retry requirement has a home to slot into.

ONE FINDING BEYOND WHAT THE GUIDE STATES. It calls cli_ops.rs "currently stale (gemini,codex
only)". The drift is wider than that and affects a list the guide does not flag:

  allowlist (is_supported_agent_name, the truth) : gemini codex deepseek claude
  cli_ops.rs:203                                  : gemini codex
  cli_ops.rs:223                                  : gemini codex
  mcp-tools/inter_agent.rs:313                    : gemini codex deepseek
  triumvirate/main.rs:2175  (HTTP /status)        : gemini codex deepseek
  triumvirate/main.rs:4059  (test asserting 2175) : gemini codex deepseek

FOUR sites, THREE different answers, and none of them matches the allowlist. `claude` is
dispatchable today but absent from every advertised list, so /status under-reports an agent
that already works. The guide's proposed `supported_agent_names()` helper is therefore more
load-bearing than it argues: it is not tidiness ahead of adding a fifth agent, it is a fix for
a live inconsistency. The main.rs:4059 test currently pins the wrong answer, so it must change
in the same commit or it will block the fix.

NOT STARTED. This is landed for reference, not implemented.
-->

# Grok Build adapter for Triumvirate — implementation guide for Claude

**Audience:** Claude Code (or any implementer) working in `github.com/michaeljboscia/triumvirate`.
**Goal:** Add `grok` as a first-class daemon peer, the same way `agy` (public name Antigravity, internal key `gemini`), `codex`, and `deepseek` already exist.
**Date:** 2026-08-30
**Scope of v1:** headless Grok Build CLI (`grok -p … --output-format streaming-json`). Not this chat app. Not Grok Bot. Not ACP (`grok agent stdio`). Not DeepSeek-style HTTP.

Read this whole file before editing. Copy existing patterns. Do not invent a fourth architecture.

---

## 0. What you are building, in one sentence

A new canonical agent key `"grok"` that the daemon can `spawn_session` / `ask_session`. The daemon starts the local `grok` binary, parses `streaming-json` NDJSON from stdout into existing `WorkingStateEvent` / `ParsedAgentResult` / `AgentStreamEvent` types, resumes via `--resume <sessionId>`, and reports tokens + tools + errors like Codex.

## 1. Hard constraints (do not violate)

1. **The peer is the `grok` CLI on the operator's machine.** It is not grok.com, not the iOS Grok app, not an xAI chat session the human is in. Auth is `XAI_API_KEY` or `grok login` on that machine.
2. **SuperGrok is a billing plan, not an executable.** Do not name the execution key `supergrok`. Canonical key: `grok`. Display name: `Grok`. Aliases that normalize to `grok`: `grok-build`, `xai`, `supergrok` (alias only).
3. **Do not copy the DeepSeek crate.** DeepSeek is HTTP + SSE to `api.deepseek.com`. Grok is a spawned CLI, like Codex exec and Agy `-p`.
4. **Do not start with ACP.** `grok agent stdio` is JSON-RPC. Codex already has a separate `app-server` path and it is the painful one. v1 is headless `-p` + NDJSON, same consult model as Agy.
5. **Apply `normalize_agent_name` at every trust boundary** before worker-acquire, session storage, and dispatch. Comment in `mcp-bridge/src/lib.rs` is law. If you store `supergrok` in one place and dispatch `grok` in another, sessions split.
6. **Triumvirate owns session, output format, cwd, and approval flags.** Operator `TRIUMVIRATE_GROK_ARGS` must not override them. This is Agy H3.
7. **Parser must be fixture-tested.** Live `grok` is optional for unit tests. Do not gate `cargo test` on `XAI_API_KEY`.
8. **Do not vendor Grok Build source.** Call the installed binary.
9. **Do not change Gemini/Agy/Codex/DeepSeek behavior** except where a shared allowlist or `supported_agents` vec must grow.
10. **v1 is consult + named session.** Fleet/ABE worktree swarms for Grok are v2. Skills (`/goatrodeo` fourth seat) are v2. Token-economics JSONL scanner for `~/.grok` is v2 unless you can do it cheaply without blocking v1.

## 2. Current system (read these files first)

Workspace: `daemon/`.

| File | Why it matters |
|---|---|
| `daemon/crates/mcp-bridge/src/lib.rs` | `normalize_agent_name`, `display_agent_name`, `is_supported_agent_name`, `gemini_command` / `codex_command` / `agy_command`, `resolve_connector_command` |
| `daemon/crates/mcp-bridge/src/agy.rs` | Invocation builder + forbidden extra-flags + tests. **Copy this file's shape for `grok.rs`.** |
| `daemon/crates/agent-adapter/src/types.rs` | `WorkingState`, `WorkingStateEvent`, `ParsedAgentResult`, `TokenUsage`, `ToolKind` |
| `daemon/crates/agent-adapter/src/gemini.rs` | NDJSON line parser → events + optional `AgentStreamEvent` channel |
| `daemon/crates/agent-adapter/src/codex.rs` | Same idea, different event names. Closest parser sibling for usage fields. |
| `daemon/crates/agent-adapter/src/lib.rs` | `pub mod` + re-exports |
| `daemon/crates/shared-types/src/streaming.rs` | `AgentStreamEvent` variants the dashboard/watch CLI consume |
| `daemon/crates/triumvirate/src/agent_exec.rs` | `run_named_agent_with_session_and_model` match (~1529), `run_agent_process_with_session` match (~2620), retry schedules, tests that assert allowlist |
| `daemon/crates/triumvirate/src/agy.rs` | Agy spawn + doctor probe. Pattern for a dedicated runner module if Grok spawn gets long. |
| `daemon/crates/mcp-tools/src/inter_agent.rs` | `/status` `supported_agents` fallback list (~313) |
| `daemon/crates/triumvirate/src/main.rs` | HTTP `/status` snapshot `supported_agents` (~2175) and tests (~4059) |
| `daemon/crates/triumvirate/src/cli_ops.rs` | CLI `status` fallback list (~223) — currently stale (`gemini`,`codex` only). Fix while you are here. |
| `daemon/crates/fleet/src/orchestrator.rs` | Fleet still special-cases Agy. **Do not teach fleet Grok in v1** unless a shared helper already exists. |
| `daemon/crates/token-economics/src/scanner.rs` | JSONL scanners per agent. v2. |
| `daemon/README.md` | Document new env vars next to `TRIUMVIRATE_GEMINI_BIN` / `TRIUMVIRATE_CODEX_BIN`. |

Public allowlist today:

```rust
matches!(
    normalize_agent_name(agent).as_str(),
    "gemini" | "codex" | "deepseek" | "claude"
)
```

`agy` / `antigravity` → `gemini`. DeepSeek is HTTP. Claude is a subprocess path that is less mature than Codex. Grok should follow **Codex-quality subprocess + Agy-quality invocation builder**.

## 3. Grok Build CLI contract (upstream)

Install (operator, not CI unit tests):

```bash
curl -fsSL https://x.ai/cli/install.sh | bash
grok --version
```

Auth — either is sufficient:

```bash
export XAI_API_KEY="xai-..."          # console.x.ai; required for headless/CI
# or
grok login                             # browser
grok login --device-auth               # no GUI
```

Official docs state ACP/headless work with cached login **or** `XAI_API_KEY`. SuperGrok subscription is **not** a compile-time or unit-test dependency.

### 3.1 v1 invocation (new session)

```bash
grok --no-auto-update \
  --no-alt-screen \
  --output-format streaming-json \
  --cwd "$WORKSPACE" \
  --session-id "$UUID" \
  -p "$PROMPT"
```

Optional, owned by mode not by operator extras:

- Consult / review (default v1): do **not** pass `--always-approve`. Prefer `--sandbox` read-leaning if the installed CLI accepts a documented profile (`workspace` / `read-only` / `strict` / `off`). If sandbox flag probe fails, run without it and document the gap.
- Implementer / yolo (env opt-in `TRIUMVIRATE_GROK_YOLO=1`): `--always-approve` (alias `--yolo`).
- Model: `-m/--model` only from `TRIUMVIRATE_GROK_MODEL` or per-request model override. Default recommended: `grok-build-0.1` if present, else omit and let CLI default (currently Grok 4.6 on many installs).
- Effort: `--effort` / `--reasoning-effort` only from `TRIUMVIRATE_GROK_EFFORT`.
- Runaway guard: `--max-turns` from `TRIUMVIRATE_GROK_MAX_TURNS` (default 20 for consult).
- Updates: always `--no-auto-update` in daemon-spawned processes.

### 3.2 v1 invocation (resume)

Grok stores headless sessions in `~/.grok/sessions`.

**Do not use `--continue`.** That is "most recent session in this cwd" and will cross-talk between Triumvirate sessions.

Use the session id returned by the previous turn's `end.sessionId` (or json `sessionId`):

```bash
grok --no-auto-update \
  --no-alt-screen \
  --output-format streaming-json \
  --cwd "$WORKSPACE" \
  --resume "$SESSION_ID" \
  -p "$PROMPT"
```

Docs disagree slightly on `--session-id` vs `--resume`:

- `-s/--session-id` — "create or resume a named headless session" on some pages; on others `-s` is "assign a NEW UUID" and does **not** resume.
- `-r/--resume` — resume existing id. This is the safe resume flag.

**Implementation rule:**

- First turn: generate a UUID in Triumvirate, pass `-s <uuid>` so Grok's on-disk session id matches our session record. If the installed CLI rejects `-s` for an existing id, first turn omits `-s` and we persist `end.sessionId` from the parser.
- Later turns: always `-r <persisted-id>`, never `-c`.

Probe both flag behaviors in `triumvirate doctor` (see §9). Persist whatever id the parser actually saw.

### 3.3 streaming-json events (authoritative for the parser)

Each stdout line is one JSON object with `type`. Unknown types: log at debug, do not fail the turn.

| `type` | Fields | Map to |
|---|---|---|
| `thought` | `data` string | **Do not** append to `response_text`. Optionally `WorkingState::MessageDelta` only at `Detailed`/`Raw` verbosity, or drop. Never show CoT in the operator-facing answer (DeepSeek bifurcation rule). |
| `text` | `data` string | Append `data` to `response_chunks`. `WorkingState::MessageDelta`. Optional `AgentStreamEvent::ResponseChunk` with a short preview. |
| `tool_call` | `toolCallId`, `toolName`, `kind`, `status`, `rawInput`, `title`, `content`, `locations` | `ToolCallStarted`. Record `ToolCallRecord { id: toolCallId, tool: toolName, kind: map_kind(kind\|toolName), args_json: rawInput }`. Emit `AgentStreamEvent::ToolCall` or `FileRead` if kind/name is read. |
| `tool_call_update` | `toolCallId`, `status`, `rawOutput`, `content`, `locations` | `ToolCallCompleted` (or `CommandCompleted` / `FileEditCompleted` if kind says so). Set `success` from `status == "completed"`. |
| `plan` | `entries` | `Unknown` or skip. Do not treat as final text. |
| `available_commands` | `tools`, `commands` | Skip. |
| `usage` | `messageId`, `stopReason`, `usage.{input_tokens,output_tokens,cache_read_input_tokens,cache_creation_input_tokens,reasoning_tokens,total_tokens}` | Update running `TokenUsage`. Not turn-complete by itself (more events may follow). |
| `end` | `stopReason`, `sessionId`, `requestId`, `usage`, `num_turns`, `modelUsage`, `total_cost_usd` | `TurnCompleted`. Set `session_id` from `sessionId`. Finalize `TokenUsage`. `parser_mode = "grok-streaming-json"`. |
| `error` | `message` | `WorkingState::Error` + `AgentStreamEvent::Error`. |
| `max_turns_reached` | | `Error` or `Stuck` with detail `max_turns_reached`. Treat as failed turn unless `text` already has a usable answer — prefer fail-closed. |
| `auto_compact_*` | | Skip / debug. |

Example fixture lines (use these in unit tests verbatim):

```json
{"type":"thought","data":"Analyzing the directory structure..."}
{"type":"tool_call","toolCallId":"call_1","title":"Read","kind":"read","status":"in_progress","toolName":"read_file","rawInput":{"path":"src/main.rs"},"content":[],"locations":[]}
{"type":"tool_call_update","toolCallId":"call_1","status":"completed","content":[],"rawOutput":{"lines":42},"locations":[]}
{"type":"text","data":"Here's a summary"}
{"type":"usage","messageId":"resp_1","stopReason":"end_turn","usage":{"input_tokens":812,"output_tokens":45,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"reasoning_tokens":0},"signature":"..."}
{"type":"end","stopReason":"end_turn","sessionId":"abc123","requestId":"xyz789","usage":{"input_tokens":812,"output_tokens":45,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"reasoning_tokens":0,"total_tokens":857},"num_turns":7,"modelUsage":{"grok-4.6":{"inputTokens":812,"outputTokens":45,"cacheReadInputTokens":0,"modelCalls":1,"costUSD":0.0012}},"total_cost_usd":0.0012}
```

`--output-format json` (batch, not v1 primary path) returns one object at the end including `sessionId`, `text`, `usage`. Keep a fallback parser for when streaming is disabled (`TRIUMVIRATE_GROK_STREAMING=0`), mirroring `TRIUMVIRATE_GEMINI_STREAMING`.

### 3.4 TokenUsage mapping

| Grok field | `TokenUsage` |
|---|---|
| `usage.input_tokens` | `input` |
| `usage.output_tokens` | `output` |
| `usage.cache_read_input_tokens` | `cached` |
| `usage.reasoning_tokens` | `thinking_tokens` |
| `usage.total_tokens` | `total` |
| `modelUsage.*.costUSD` or `total_cost_usd` | not on `TokenUsage` today — put cost in log/`detail` or skip in v1 |
| tool_call count | `tool_calls` = `self.tool_calls.len()` on `end` |

### 3.5 ToolKind mapping

```
read / read_file / Read / read_file-ish     → ReadFile
write / write_file                          → WriteFile
edit / edit_file / apply_patch / str_replace → EditFile
bash / shell / command / terminal           → Bash
grep / search                               → Grep
glob / glob_file_search                     → Glob
ask / request_user_input                    → RequestUserInput
else                                        → Unknown
```

Use both `kind` and `toolName`. Prefer `toolName` for `ToolCallRecord.tool`.

### 3.6 Exit codes

| Code | Meaning | Daemon behavior |
|---|---|---|
| 0 | Success | Parse stdout; if no `end` and no text, treat as error |
| 1 | Auth / network / runtime | Surface stderr snippet. Classify auth vs other for doctor/fallback |
| 130 / 143 | Signal | Kill-on-drop path; not a model error |
| nonempty stderr + 0 | Often debug | Log debug; do not fail if parser got `end` |

### 3.7 Flags Triumvirate owns (forbidden in `TRIUMVIRATE_GROK_ARGS`)

Mirror Agy `FORBIDDEN_EXTRA_FLAGS`:

```
-p, --single, --prompt, --prompt-file, --prompt-json
--output-format, -o
-r, --resume
-s, --session-id
-c, --continue
--cwd
--fork-session
--always-approve, --yolo, --dangerously-skip-permissions
--sandbox
--max-turns
-m, --model
--effort, --reasoning-effort
--no-auto-update
--no-alt-screen
```

Reject with a clear `InvalidInput` error naming the flag. Tests required.

## 4. Identity and naming

```rust
// normalize_agent_name
"grok" | "grok-build" | "xai" | "supergrok" => "grok"

// is_supported_agent_name  add "grok"

// display_agent_name
"grok" => "Grok"
```

WorkingStateEvent.agent and AgentStreamEvent.agent must be the **canonical** key `"grok"`, not the alias, not `"Grok"`. Display layer uses `display_agent_name`.

## 5. File-by-file implementation plan

Do these in order. Each slice should compile and have tests.

### Slice A — identity (smallest PR-shaped commit)

**`daemon/crates/mcp-bridge/src/lib.rs`**

- Extend `normalize_agent_name`.
- Extend `display_agent_name`.
- Extend `is_supported_agent_name`.
- Add `grok_command() -> (String, Vec<String>)` via `resolve_connector_command("TRIUMVIRATE_GROK_BIN", "TRIUMVIRATE_GROK_ARGS", "grok")`.
- Add unit tests next to the existing normalize/display tests in that file.

**`daemon/crates/mcp-tools/src/inter_agent.rs`**
- Add `"grok"` to the local `supported_agents` vec (~313).

**`daemon/crates/triumvirate/src/main.rs`**
- Add `"grok"` to HTTP status `supported_agents` (~2175) and fix the assertion at ~4059.

**`daemon/crates/triumvirate/src/cli_ops.rs`**
- Replace stale `["gemini","codex"]` fallback with the same four-or-five list the HTTP path uses. Prefer a single helper `mcp_bridge::supported_agent_names() -> Vec<&'static str>` so this cannot drift again.

Add:

```rust
pub fn supported_agent_names() -> &'static [&'static str] {
    &["gemini", "codex", "deepseek", "claude", "grok"]
}
```

Use it everywhere a literal list exists. Grep `supported_agents` before you finish.

**`daemon/crates/triumvirate/src/agent_exec.rs` tests**
- `assert!(mcp_bridge::is_supported_agent_name("grok"));`
- `assert!(mcp_bridge::is_supported_agent_name("supergrok"));` // alias
- `assert_eq!(mcp_bridge::normalize_agent_name("Grok-Build"), "grok");`
- `assert_eq!(mcp_bridge::display_agent_name("supergrok"), "Grok");`

Grep the whole daemon for `is_supported_agent_name("fake-agent")` style tests and update.

### Slice B — invocation builder

**New file `daemon/crates/mcp-bridge/src/grok.rs`**

Copy structure from `agy.rs`, not behavior.

```rust
pub struct GrokInvocation {
    pub program: String,
    pub args: Vec<String>,
    pub timeout: Duration,
}

pub fn grok_connector_timeout() -> Duration { /* TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS default 900 */ }
pub fn grok_yolo_enabled() -> bool { /* TRIUMVIRATE_GROK_YOLO default false for consult */ }
pub fn grok_streaming_enabled() -> bool { /* TRIUMVIRATE_GROK_STREAMING default true */ }
pub fn grok_max_turns() -> u32 { /* TRIUMVIRATE_GROK_MAX_TURNS default 20 */ }
pub fn grok_model() -> Option<String> { /* TRIUMVIRATE_GROK_MODEL */ }
pub fn grok_effort() -> Option<String> { /* TRIUMVIRATE_GROK_EFFORT */ }

pub fn build_grok_invocation(
    bin: &str,
    extra_args: &[String],
    prompt: &str,
    cwd: &str,
    session_id: Option<&str>,
    resume: bool,
) -> std::io::Result<GrokInvocation>;
```

Arg order (stable, tested):

1. `--no-auto-update`
2. `--no-alt-screen`
3. `--output-format` `streaming-json` (or `json` if streaming disabled)
4. `--cwd` `<cwd>` if cwd nonempty
5. if `resume` { `--resume` `<id>` } else if `session_id` { `--session-id` `<id>` }
6. if model set { `-m` `<model>` }
7. if effort set { `--effort` `<effort>` }
8. `--max-turns` `<n>`
9. if yolo { `--always-approve` }
10. extra_args (already validated)
11. `-p` `<prompt>` last so a prompt starting with `-` cannot be parsed as a flag… actually Grok takes `-p VALUE`. Keep `-p` immediately before prompt. Extra args must not come after `-p`.

**Export** `pub mod grok;` from `mcp-bridge/src/lib.rs`.

Tests in the same file (table-driven):

- default consult has `--output-format streaming-json`, `-p`, no `--always-approve`, no `-c`
- resume uses `--resume` and does not also pass `--session-id`
- new session uses `--session-id` when provided
- forbidden extra flag errors
- yolo injects `--always-approve`
- streaming off uses `json`
- model/effort only when env/args say so
- `--cwd` present

### Slice C — parser

**New file `daemon/crates/agent-adapter/src/grok.rs`**

Clone the shape of `GeminiStreamParser` / `CodexExecParser`:

```rust
pub struct GrokStreamParser { /* session_id, response_chunks, events, tool_calls, token_usage, stream_tx, stream_seq */ }

impl GrokStreamParser {
    pub fn new() -> Self;
    pub fn with_stream_channel(tx: mpsc::Sender<AgentStreamEvent>) -> Self;
    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent>;
    pub fn finish(self) -> ParsedAgentResult;
}
```

`parse_line`:

- trim; skip empty
- `serde_json::from_str` fail → return None (Codex/Gemini ignore non-JSON; Grok may print a banner — ignore)
- dispatch on `type`
- `agent` field always `"grok"`

`finish`:

```rust
ParsedAgentResult {
    response_text: self.response_chunks.concat(),
    session_id: self.session_id,
    events: self.events,
    tool_calls: self.tool_calls,
    token_usage: self.token_usage,
    cli_version: None, // fill if an init-like event appears later
    parser_mode: "grok-streaming-json".into(),
}
```

Also implement `parse_batch_json(value: &Value) -> ParsedAgentResult` for the one-shot `json` format. Keep it small: read `text`/`sessionId`/`usage`.

Re-export from `agent-adapter/src/lib.rs`.

Parser tests (no network):

1. Happy path fixture in §3.3 → response_text `Here's a summary`, session `abc123`, one tool call succeeded, tokens 812/45/0, thinking 0.
2. Thought-only lines do not appear in `response_text`.
3. Split JSON across... no, parsers are line-oriented. Test that a non-JSON line is ignored.
4. `error` type → WorkingState::Error.
5. `max_turns_reached` → fail-closed.
6. Unknown type → None, parser still `finish()`s.
7. `tool_call` without `toolName` uses `title` or `"unknown"`.
8. FileRead stream event when kind is read.

### Slice D — spawn / dispatch

**`daemon/crates/triumvirate/src/agent_exec.rs`**

In `run_named_agent_with_session_and_model` add:

```rust
"grok" => {
    let (bin, args) = mcp_bridge::grok_command();
    run_grok_cli_process_with_session(
        &bin, &args, message, cwd, session_id, events_tx, model_override,
    ).await
}
```

In `run_agent_process_with_session` you can either add a `"grok"` arm or **not** route Grok through that generic match at all (preferred: dedicated function like Agy/Codex).

Implement `run_grok_cli_process_with_session` next to `run_codex_cli_process_with_session`:

1. `build_grok_invocation(...)`.
2. `Command::new(program).args(args).current_dir(cwd).stdin(null).stdout(piped).stderr(piped).kill_on_drop(true)`.
3. `configure_process_group` like Codex.
4. Line-read stdout with `BufReader`. For each line: `parser.parse_line`; if Some(event) and `should_display`, forward on `events_tx`.
5. Drain stderr to tracing debug.
6. `timeout` = `grok_connector_timeout()`.
7. Wait child. On timeout, kill process group.
8. `parser.finish()`. If exit != 0 and response_text empty, `bail` with stderr tail.
9. If streaming disabled, parse the single JSON object from stdout.

Retry schedule: **single attempt** like DeepSeek, not Gemini's multi-timeout ladder. Grok turns are long and expensive. Do not double-spend. Add a test next to `schedule_len_for("deepseek")`.

Stuck detection: reuse `agent-adapter::StuckDetector` on the event stream if Codex/Gemini already hook it in this function. If they do, hook Grok the same way. If they don't in this path, do not invent it in v1.

Pass `XAI_API_KEY` through from the daemon environment (Command inherits env by default — do not strip it). Do not log the key.

### Slice E — doctor and README

**Doctor** (`triumvirate/src/cli_ops.rs` and/or `agy.rs` probe style):

Print:

- resolved `TRIUMVIRATE_GROK_BIN` / PATH `grok`
- `grok --version` stdout
- auth: `XAI_API_KEY` set (boolean only) OR `~/.grok` auth file exists (do not print secrets)
- `grok models` success optional, 5s timeout
- note: SuperGrok plan is not required if API key works

**`daemon/README.md`** Runtime Environment Variables section — add:

```
TRIUMVIRATE_GROK_BIN, TRIUMVIRATE_GROK_ARGS
TRIUMVIRATE_GROK_MODEL
TRIUMVIRATE_GROK_EFFORT
TRIUMVIRATE_GROK_MAX_TURNS          (default 20)
TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS  (default 900)
TRIUMVIRATE_GROK_STREAMING          (default on)
TRIUMVIRATE_GROK_YOLO               (default off)
```

Root `README.md` ecosystem table: add Grok Build CLI as a peer next to Gemini CLI / Codex CLI. One paragraph, no marketing rewrite.

### Slice F — integration tests

Follow `tests/integration_http.rs` DeepSeek status test:

- `/status` `supported_agents` contains `"grok"`
- MCP ask with agent `supergrok` is accepted and normalized (mock bin)

Mock connector: Codex/Gemini tests set `TRIUMVIRATE_*_BIN` to a script. Add `tests/fixtures/mock_grok.sh` that:

```bash
#!/usr/bin/env bash
# echo the fixture NDJSON from §3.3 then exit 0
```

Respect `-p` existence. Ignore other flags. This unlocks `run_grok_cli_process_with_session` tests without a network.

Mark live tests `#[ignore]` behind `TRIUMVIRATE_LIVE_GROK=1`.

## 6. Environment matrix

| Var | Default | Meaning |
|---|---|---|
| `TRIUMVIRATE_GROK_BIN` | `grok` | Executable |
| `TRIUMVIRATE_GROK_ARGS` | empty | Extra args; forbidden flags rejected |
| `TRIUMVIRATE_GROK_MODEL` | unset (CLI default) | `-m` |
| `TRIUMVIRATE_GROK_EFFORT` | unset | `--effort` |
| `TRIUMVIRATE_GROK_MAX_TURNS` | `20` | `--max-turns` |
| `TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS` | `900` | wall clock |
| `TRIUMVIRATE_GROK_STREAMING` | on | `streaming-json` vs `json` |
| `TRIUMVIRATE_GROK_YOLO` | off | `--always-approve` |
| `XAI_API_KEY` | unset | inherited by child |

Do not add `TRIUMVIRATE_SUPERGROK_*`.

## 7. Session semantics

Triumvirate sessions are named (`spawn_session(..., session_name)`). Persist Grok's UUID **on the session record** (whatever field Codex uses for thread id). On `ask_session`:

- If stored grok id present → `resume=true`
- Else first ask → new id, store whatever parser returns

If parser returns a different `sessionId` than the one we passed with `-s`, **trust the parser** and update the record. Log a warning.

Never share one Grok on-disk session across two Triumvirate session names.

## 8. Error classification (keep small)

| Symptom | Operator-facing message |
|---|---|
| bin not found | `grok binary not found; set TRIUMVIRATE_GROK_BIN or install https://x.ai/cli` |
| exit 1 + stderr contains `auth` / `401` / `unauthorized` / `login` | `Grok auth failed; set XAI_API_KEY or run grok login` |
| timeout | `Grok exceeded TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS` |
| max_turns_reached | `Grok hit --max-turns; raise TRIUMVIRATE_GROK_MAX_TURNS or shrink the prompt` |
| empty text + exit 0 | `Grok produced no text; see debug logs` |
| forbidden extra arg | existing InvalidInput string |

Fallback outbox: use the same path other agents use on spawn failure. Do not invent a Grok-specific queue.

## 9. doctor probe algorithm

```
which $TRIUMVIRATE_GROK_BIN || which grok
if missing → FAIL grok: not installed
run: grok --no-auto-update --version   (5s)
if fail → WARN grok: binary exists but --version failed
auth_ok = env XAI_API_KEY nonempty OR file exists:
    ~/.grok/auth.json OR ~/.grok/credentials.json
    (glob ~/.grok/*auth* if names differ; do not dump contents)
if !auth_ok → WARN grok: no XAI_API_KEY and no cached login
else OK grok: binary + auth present
```

Do not call `-p` in doctor. That spends tokens.

## 10. What not to touch in v1

- `/goatrodeo` and `/postrodeo` skill markdown (v2: add Grok as optional third reviewer).
- `fleet::orchestrator` worktree swarms.
- `token-economics` `~/.grok` transcript scanner.
- `grok agent stdio` ACP client.
- HTTP to `https://api.x.ai/v1` (that would be a DeepSeek-shaped sibling; only do it if the CLI is unavailable and a later spec says so).
- Renaming internal `gemini` key to `agy`. Out of scope.
- Paying / checking SuperGrok entitlement. If the CLI runs, the adapter runs.

## 11. v2 backlog (write into ROADMAP.md as one bullet, do not implement)

- ACP persistent worker (`grok agent stdio`) for lower spawn cost.
- Grok as goatrodeo twin.
- ABE worktree worker type `grok`.
- Token economics scanner for Grok session JSON.
- Shadow-compare Grok vs Codex on the same prompt (opt-in).
- Cedar / approval channel if Grok starts emitting permission prompts in non-yolo mode.

## 12. Suggested commit series

1. `feat(bridge): recognize grok agent aliases and supported_agents`
2. `feat(bridge): grok invocation builder`
3. `feat(adapter): GrokStreamParser for streaming-json`
4. `feat(daemon): spawn grok CLI sessions`
5. `feat(cli): doctor probe + README env for grok`
6. `test: mock grok fixture + status contract`

Do not squash these into one unreviewable diff if the repo's recent DeepSeek work was sliced.

## 13. Acceptance checklist

Claude must be able to tick all of these before claiming done:

- [ ] `cargo test -p mcp-bridge -p agent-adapter` passes with no network
- [ ] `is_supported_agent_name("grok")` and aliases
- [ ] `display_agent_name("supergrok") == "Grok"`
- [ ] `/status` JSON includes `grok`
- [ ] `build_grok_invocation` never emits `-c` / `--continue`
- [ ] `build_grok_invocation` resume ≠ new-session flags mixed
- [ ] Forbidden `TRIUMVIRATE_GROK_ARGS` rejected
- [ ] Parser fixture yields text, session id, tokens, one successful tool
- [ ] Thoughts excluded from `response_text`
- [ ] Dispatch match has `"grok"` arm; unknown agent still bails
- [ ] Retry schedule length for grok is 1
- [ ] README documents env vars
- [ ] doctor does not spend tokens
- [ ] No SuperGrok purchase required for unit tests
- [ ] Existing gemini/codex/deepseek tests still pass
- [ ] `rg 'supported_agents'` lists are consistent

Live smoke (operator, optional):

```bash
export XAI_API_KEY=...
cd daemon && cargo build -p triumvirate --release
# register MCP, then from cockpit:
#   spawn a Grok session called 'research'
#   ask research: reply with the single word pong
```

Expect WorkingState TurnStarted → maybe tools → TurnCompleted, response contains `pong`, session resumes on a second ask.

## 14. Style rules this repo actually uses

- REQ-IDs in comments when a behavior is contractual (`REQ-GROK-001` …).
- `#[instrument(skip_all)]` on public bridge fns.
- `tracing::info!` / `debug!` for backend selection, never log prompts at info if other agents do not.
- Tests that mutate env use the same `unsafe { set_var / remove_var }` + lock pattern DeepSeek tests use. Follow it; do not "clean up" by introducing a new env crate mid-PR.
- Display names are brand-correct: `DeepSeek`, `Antigravity`, `Grok` — never `Deepseek`, `Agy`, `Supergrok` in operator-facing strings.
- Prefer extending an existing match to adding a parallel subsystem.

## 15. REQ register (assign these IDs in code comments)

| ID | Requirement |
|---|---|
| REQ-GROK-001 | Canonical key `grok`; aliases `grok-build`, `xai`, `supergrok` |
| REQ-GROK-002 | Display name `Grok` |
| REQ-GROK-003 | Allowlist + `/status` + doctor include `grok` |
| REQ-GROK-004 | Spawn `grok` CLI; no HTTP v1 |
| REQ-GROK-005 | Default output `streaming-json`; batch `json` if streaming off |
| REQ-GROK-006 | Session resume via `--resume`; never `--continue` |
| REQ-GROK-007 | Persist parser `sessionId` as source of truth |
| REQ-GROK-008 | Forbidden extra flags (H3) |
| REQ-GROK-009 | YOLO / `--always-approve` opt-in only |
| REQ-GROK-010 | `--no-auto-update` and `--no-alt-screen` always |
| REQ-GROK-011 | Thoughts not in `response_text` |
| REQ-GROK-012 | Token map per §3.4 |
| REQ-GROK-013 | Single-attempt retry |
| REQ-GROK-014 | Timeout default 900s |
| REQ-GROK-015 | Fixture tests; live tests ignored |
| REQ-GROK-016 | Auth errors classified without leaking key |
| REQ-GROK-017 | Inherit `XAI_API_KEY`; do not require SuperGrok |
| REQ-GROK-018 | `parser_mode = grok-streaming-json` |

## 16. If the installed `grok` schema drifts

The open-source docs live in `xai-org/grok-build` user-guide chapter 14 (headless). If a live capture disagrees with §3.3:

1. Save the raw NDJSON under `daemon/crates/agent-adapter/tests/fixtures/grok-streaming-YYYYMMDD.jsonl`
2. Extend the parser with additional `type` arms
3. Keep old fixture tests passing
4. Do not break Gemini/Codex parsers to "share a generic JSON agent parser" in the same PR

## 17. Prompt you can give yourself (Claude) when starting

> Implement REQ-GROK-001 through REQ-GROK-018 in triumvirate as specified in GROK_ADAPTER_IMPLEMENTATION_GUIDE.md. Start with Slice A. Do not implement fleet, skills, ACP, or DeepSeek-style HTTP. Mirror mcp-bridge/src/agy.rs and agent-adapter/src/codex.rs. All new unit tests must be offline.

End of guide.
