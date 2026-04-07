# BUG-001 Diagnosis — Claude Doesn't Respond to Dashboard Messages

**Date:** 2026-04-05
**Diagnosed by:** Claude (Opus 4.6)
**Status:** Root-caused, not yet fixed

---

## Root Cause (Two bugs, not one)

### Bug A: Missing `-p` flag on CLI invocation

File: `daemon/crates/agentd/src/agent/claude.rs` lines 66-77

The connector spawns:
```
claude --input-format stream-json --output-format stream-json --session-id <uuid>
```

But the CLI help says explicitly:
```
--input-format <format>   Input format (only works with --print)
--output-format <format>  Output format (only works with --print)
```

**Without `-p`, both flags are silently ignored.** Claude enters interactive TUI mode, which hangs on piped stdio. The process stays alive but never reads stdin or writes stdout.

### Bug B: Event parser doesn't match real CLI output format

File: `daemon/crates/proto/src/claude_events.rs`

The parser was written against assumed output. Here's what Claude CLI **actually** outputs with `-p --output-format stream-json` (verified empirically):

```jsonl
{"type":"system","subtype":"hook_started",...}
{"type":"system","subtype":"hook_response",...}
{"type":"system","subtype":"init","cwd":"...","session_id":"...","tools":[...],"model":"claude-opus-4-6[1m]",...}
{"type":"assistant","message":{"model":"...","id":"msg_...","type":"message","role":"assistant","content":[{"type":"text","text":"Hey, what's up."}],"stop_reason":null,"usage":{...}},"session_id":"..."}
{"type":"rate_limit_event","rate_limit_info":{...}}
{"type":"result","subtype":"success","is_error":false,"duration_ms":2828,"result":"Hey, what's up.","session_id":"...","total_cost_usd":0.222,"usage":{...}}
```

**Three parser failures:**

1. **Kind classification**: Assistant messages have `type: "assistant"`, not `"message"`. The parser's `event_name.contains("message")` check never matches `"assistant"`. Falls through to `Unknown`.

2. **Text extraction from assistant events**: Content is at `message.content[0].text` — an **array** of content blocks. The `extract_text()` function only checks `message.content` as a string (`Value::as_str`), which returns `None` for an array.

3. **Text extraction from result events**: The `result` field is a **plain string** (`"result":"Hey, what's up."`), not a nested object. The parser tries `result.get("content")` and `result.get("text")`, which fail because you can't `.get()` on a string Value.

---

## `--input-format stream-json` Does NOT Work for Persistent Sessions

Tested 8+ JSON input formats with `-p --input-format stream-json --output-format stream-json`:
- `{"type":"user_input","content":"hi"}`
- `{"type":"user_input","message":"hi"}`
- `{"type":"message","role":"user","content":"hi"}`
- `{"type":"human","content":"hi"}`
- `{"role":"user","content":"hi"}`
- `{"prompt":"hi"}`
- `{"content":"hi"}`
- `"hi"`

**None produced a response.** The CLI starts, runs hooks, but never processes any JSON input. Exit code 0, no stderr. The feature either expects an undocumented format or is designed for a different use case than persistent multi-turn.

---

## What DOES Work (Verified)

Plain text piped to `-p --output-format stream-json`:
```bash
echo "say hi in 3 words" | claude -p --output-format stream-json
```

This produces the full event stream shown above. Response in ~3 seconds.

---

## Required Fix: Per-Turn Invocation

Change the connector from persistent-subprocess to per-turn invocation:

### Current (broken):
1. `spawn()` starts one Claude process with `--input-format stream-json`
2. Messages written to stdin as JSON
3. Process stays alive forever
4. stdout reader parses responses

### Fixed:
1. `spawn()` sets up session_id + message channel, does NOT start a process
2. Background task reads from channel; for EACH message, spawns:
   ```
   claude -p --output-format stream-json --bare --dangerously-skip-permissions --session-id <uuid>
   ```
   with the message piped as plain text to stdin
3. Reads stdout JSONL, emits to fabric
4. Process exits after turn completes
5. Next message spawns new process — session continuity via `--session-id <uuid>`

### CLI flags explained:
- `-p` — REQUIRED for `--output-format` to work
- `--output-format stream-json` — JSONL streaming output
- `--bare` — skip hooks/LSP/plugins (cuts startup from ~3s to <1s)
- `--dangerously-skip-permissions` — no permission prompts (daemon controls execution)
- `--session-id <uuid>` — session persistence across turns

### Parser fixes needed in `claude_events.rs`:

```rust
// 1. Add "assistant" to kind classification
} else if event_name.contains("assistant") {
    ClaudeEventKind::Message
}

// 2. Handle content as array in extract_text()
if let Some(message) = value.get("message") {
    if let Some(content_array) = message.get("content").and_then(Value::as_array) {
        for block in content_array {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    return Some(text.to_string());
                }
            }
        }
    }
}

// 3. Handle result as plain string
if let Some(result) = value.get("result") {
    if let Some(s) = result.as_str() {
        return Some(s.to_string());
    }
    // existing nested object checks...
}
```

### Latency consideration:
- Per-turn adds ~3s process startup per message
- `--bare` cuts hooks/LSP, should reduce to <1s
- Session context cached on disk via `--session-id`

---

## Files to Modify

| File | What |
|------|------|
| `daemon/crates/agentd/src/agent/claude.rs` | Rewrite spawn/send to per-turn model |
| `daemon/crates/proto/src/claude_events.rs` | Fix kind classification + text extraction |
| `daemon/crates/proto/src/claude_events.rs` (tests) | Update test fixtures to match real output |

## Files NOT to Modify

The rest of the pipeline is fine:
- `web/server.rs` message_handler correctly emits to `Topic::AgentInput(Claude)` with `Payload::HumanMessage`
- `fabric/bus.rs` publish/subscribe works correctly (no race condition)
- `routing.rs` correctly routes messages
- `web/ws.rs` will work once events actually reach the fabric

---

## Test Verification

After fixing, this should work:
```bash
# Build
cd /Users/mikeboscia/projects/triumvirate/daemon
cd frontend && npm run build && cd ..
cargo build

# Run daemon
cargo run --bin triumvirate-agentd

# Send message (in another terminal)
curl -s -X POST http://127.0.0.1:8080/api/message \
  -H "Content-Type: application/json" \
  -d '{"content": "Say hello in exactly 5 words"}'

# Check session log for agent output events
sleep 10
grep "agent_output\|AgentResponse\|TextChunk" ~/.triumvirate/sessions/*.jsonl | tail -5
```

---

## Additional Bugs Found (Not BUG-001)

### BUG-002 fix is written but not built
WebSocket reconnect logic in `frontend/src/lib/stores/fabric.ts`. Just needs `npm run build`.

### BUG-003 fix is written but not built
Layout widened in `frontend/src/routes/App.svelte`. Just needs `npm run build`.

Both are resolved by running the frontend build as part of the test workflow.
