# Critical Gap: MCP Progress Notifications Not Streaming

**Date:** 2026-04-05
**Found by:** Claude code review of daemon-v2
**Severity:** BLOCKER — this is the #1 feature in the entire spec
**Files:** `daemon-v2/crates/triumvirate/src/main.rs`

---

## The Problem

Lifecycle events are collected in `Vec<LifecycleEvent>` and returned in the final tool response. They are NOT streamed to Claude in real-time during tool execution.

**What the user sees today:**
```
[nothing for 20 seconds while both agents work]
[everything dumps at once when the tool call returns]
```

**What the spec requires (SPEC_FINAL.md, "Lifecycle Visibility" section):**
```
→ Gemini: sent ✓         (appears at 1s)
→ Codex: sent ✓           (appears at 1s)
→ Codex: working... (6s)  (appears at 7s)
→ Gemini: working... (10s) (appears at 11s)
→ Codex: responded ✓      (appears at 15s)
→ Gemini: responded ✓     (appears at 20s)
[final combined results]
```

This is the entire reason the rewrite exists. The old TS system had 16% silent failures. The new system was designed to show every step in real-time. Without streaming progress, we've rebuilt the same black box.

---

## The Fix

The `rmcp` crate supports this natively. Two mechanisms:

### 1. Progress Notifications (`notify_progress`)

```rust
// Inside an async tool handler, access the context peer:
context.peer.notify_progress(ProgressNotificationParam {
    progress_token: token,  // from the request metadata
    progress: 1.0,
    total: Some(6.0),       // total expected steps
    message: Some("→ Gemini: sent ✓".to_string()),
}).await;
```

### 2. Logging Messages (`notify_logging_message`)

```rust
context.peer.send_logging_message(LoggingMessageNotification {
    level: LoggingLevel::Info,
    logger: Some("triumvirate".to_string()),
    data: serde_json::json!("→ Codex: working... (8s elapsed)"),
}).await;
```

Both are fire-and-forget notifications that stream to Claude DURING the tool call, before the final result returns.

---

## Where To Wire It

### `execute_ask_agent` (line ~483)

Currently:
```rust
lifecycle.push(LifecycleEvent { state: "SPAWNED".into(), detail: format!("Started {agent}") });
// ... agent runs ...
lifecycle.push(LifecycleEvent { state: "DONE".into(), detail: "responded".into() });
```

Needs to ALSO emit each event as a progress notification at the time it happens:
```rust
lifecycle.push(LifecycleEvent { state: "SPAWNED".into(), detail: format!("Started {agent}") });
// EMIT NOW, don't wait:
peer.notify_logging_message(...).await;
```

### `execute_ask_twins` (line ~684)

Same pattern but for parallel fanout. Both agents should emit events independently as they happen:
```
→ Gemini: sent ✓      (emit immediately on spawn)
→ Codex: sent ✓       (emit immediately on spawn)
→ Codex: working...   (emit on heartbeat timer)
→ Gemini: working...  (emit on heartbeat timer)  
→ Codex: responded ✓  (emit when stdout parsed)
→ Gemini: responded ✓ (emit when stdout parsed)
```

### Retry events

Each retry attempt should emit:
```
→ Codex: TIMEOUT after 60s ✗  (emit on timeout)
→ Codex: retrying (2/3)...     (emit on retry start)
```

### Failure events

```
→ Codex: FAILED after 3 attempts. Error: stream disconnected
→ Codex: dead drop launched, PID 67890
```

---

## Implementation Approach

The tool handlers in `McpBridge` need access to the `rmcp` server peer to emit notifications. Two ways:

### Option A: Pass peer through context

The `rmcp` `#[tool]` macro provides a `context` parameter. Check if the peer is accessible from the tool handler's self or context. If so, emit directly.

### Option B: Channel-based

If the tool handler can't access the peer directly:
1. Create a `tokio::mpsc::channel` for lifecycle events
2. Tool handler sends events to the channel
3. A separate task reads from the channel and calls `peer.notify_logging_message`
4. Tool handler awaits the agent processes and sends lifecycle events as they happen

Option B is more flexible and matches the TS server's `makeProgressLogger` pattern — a callback that the execution engine calls at each lifecycle transition.

---

## Heartbeat Timer

The spec requires progressive heartbeat during "working" state:
- First heartbeat at 10s
- Then every 30s
- Then every 60s

Implementation: spawn a heartbeat task when an agent starts working. It emits "working... (Ns elapsed)" on the schedule above. Cancel it when the agent responds or times out.

---

## Test Strategy

### Unit test
Mock the peer/notification channel. Verify that `execute_ask_agent` emits SPAWNED, WORKING, DONE events in order with correct timing.

### Integration test
Run the MCP bridge with a mock CLI that takes 5s to respond. Capture all notifications emitted during the tool call. Verify they arrive BEFORE the final result.

### E2E test
Register the MCP server in Claude. Call `ask_agent`. Verify that Claude's display shows lifecycle events appearing progressively, not all at once.

---

## Acceptance Criteria

- [ ] `ask_agent` emits SPAWNED notification within 1s of tool call
- [ ] `ask_agent` emits WORKING heartbeat at 10s, 40s, 100s intervals
- [ ] `ask_agent` emits DONE/FAILED notification when agent responds or times out
- [ ] `ask_twins` emits independent lifecycle events for EACH agent as they happen
- [ ] Retry events emit TIMEOUT, RETRY notifications in real-time
- [ ] Dead drop fallback emits FALLBACK_LAUNCHED notification
- [ ] All notifications arrive BEFORE the final tool result
- [ ] Existing 66 tests still pass (no regression)

---

## Priority

This is not a nice-to-have. This is the product.

> "I fire off a question to the twins and have NO IDEA if they've gotten it, if they're working, where they are, or if they've hung up" — Mike, the reason the entire rewrite exists

Without progress notifications, the rewrite is a better-structured version of the same black box.
