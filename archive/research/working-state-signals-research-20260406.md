# Working State Signals Research — Triumvirate Agent Observability

**Date:** 2026-04-06
**Purpose:** Inform design of additional "working state" communication messages for triumvirate daemon

---

## The Problem

When triumvirate dispatches work to Gemini or Codex via `spawn_session` / `ask_session`, the caller (Claude) gets back a final response — but has no visibility into what's happening *during* execution. The current heartbeat system in `execute_ask_twins` emits periodic "still working (Ns elapsed)" messages, but these are dumb timers, not actual state signals.

We want to know: Is the agent thinking? Calling a tool? Waiting on an API? Reading files? Writing code? Stuck in a loop?

---

## Three Signal Layers (from research)

### Layer 1: Network Traffic Inference (Passive, No Agent Cooperation Required)

Monitor the subprocess's network I/O to infer state without any agent-side changes.

| Network Pattern | Inferred State |
|----------------|----------------|
| Outbound POST spike → silence | `WAITING_FOR_API` — prompt sent, waiting for LLM response |
| Sustained inbound SSE/streaming data | `GENERATING` — LLM is producing tokens |
| Near-zero traffic | `IDLE` or `LOCAL_WORK` — agent is doing filesystem/tool work, no API call |
| Burst of small outbound requests | `TOOL_CALLS` — agent is hitting external APIs (GitHub, search, etc.) |

**Implementation for triumvirate:**
- On macOS: `nettop -p <pid> -J bytes_in,bytes_out` sampled every 1-2s
- Track bytes_in/bytes_out delta per sample
- Classify into state based on thresholds:
  - `bytes_out > threshold && bytes_in ≈ 0` → WAITING_FOR_API
  - `bytes_in > threshold (sustained)` → GENERATING
  - `both ≈ 0` → LOCAL_WORK or IDLE
- Publish state changes to the fabric broadcast channel

**Pros:** Works with any CLI agent. No agent cooperation needed. Zero changes to Gemini/Codex.
**Cons:** Coarse-grained. Can't distinguish "reading a file" from "truly idle." SSL makes payload inspection impossible (but byte volume still works).

### Layer 2: Stdout/Stderr Stream Analysis (Semi-passive)

Parse the agent's stdout/stderr stream for structured signals.

**Claude Code** emits:
- Tool call names and results (visible in streaming output)
- Progress notifications via MCP protocol

**Codex CLI** emits:
- Streaming output with tool markers
- `codex exec` returns structured output

**Gemini CLI** emits:
- Streaming text output
- Tool use indicators in output

**Implementation for triumvirate:**
- The PTY reader (already in the daemon) captures all stdout
- Add a classifier layer between PTY reader and response accumulator
- Pattern-match for:
  - Tool call markers → `TOOL_CALLING: <tool_name>`
  - Code block output → `WRITING_CODE`
  - Search/read patterns → `READING_CODEBASE`
  - Streaming text → `THINKING` or `RESPONDING`
- Emit classified events to the fabric

**Pros:** Richer than network inference. Agent-specific patterns.
**Cons:** Fragile — output format changes break parsers. Different per agent.

### Layer 3: Cooperative Signals (Agent Self-Report)

Have the agent explicitly report its state through a structured protocol.

**Work Notes pattern** (from arxiv research):
- Agent maintains a "work journal" of plans and outcomes
- Each step logged with: action type, target, status, duration

**Factory.ai "Signals" pattern:**
- Post-session LLM analysis extracts abstract patterns
- Identifies friction moments vs success moments
- Feeds back into agent improvement loop

**LangChain streaming events:**
- `on_llm_start` / `on_llm_end` — LLM call lifecycle
- `on_tool_start` / `on_tool_end` — tool execution lifecycle
- `on_chain_start` / `on_chain_end` — chain/agent step lifecycle
- Custom `dispatch_custom_event` for user-defined signals

**Implementation for triumvirate:**
- Define a `WorkingState` enum:
  ```
  SPAWNING, IDLE, THINKING, TOOL_CALLING, READING, WRITING, 
  SEARCHING, API_WAITING, GENERATING, STUCK, ERROR, DONE
  ```
- MCP protocol already has `ProgressNotification` — extend with state metadata
- For sessions: inject a system prompt addendum asking the agent to emit state signals
- Daemon correlates self-reported state with network/stdout signals for ground truth

**Pros:** Highest fidelity. Agent knows what it's doing.
**Cons:** Requires agent cooperation. Adds tokens to every turn. Agents might lie or hallucinate state.

---

## Recommended Architecture for Triumvirate

### Phase 1: Network Traffic Monitor (cheapest, no agent changes)

Add a `NetworkProbe` that samples subprocess network I/O every 2 seconds:

```
NetworkProbe 
  → samples bytes_in/bytes_out per PID
  → classifies into WorkingState
  → publishes to fabric broadcast channel
  → stenographer captures to session JSONL
```

This gives immediate value: the `ask_session` heartbeat can report "Gemini: generating tokens" vs "Gemini: waiting for API" vs "Gemini: working locally" instead of the current dumb "still working (10s elapsed)."

### Phase 2: Stdout Stream Classifier (medium effort)

Add pattern matching to the PTY reader output:

```
PTY Reader 
  → raw bytes 
  → StreamClassifier (per-agent patterns)
  → classified events to fabric
  → merge with NetworkProbe signals
```

### Phase 3: Cooperative Protocol (requires agent integration)

Define a lightweight JSON event format that agents can emit:

```json
{"type": "working_state", "state": "TOOL_CALLING", "detail": "reading src/main.rs", "ts": 1712380800}
```

Inject via system prompt or MCP notification channel. Daemon parses and publishes.

---

## Stuck Detection (Cross-cutting)

All three layers feed into a `StuckDetector`:

| Signal | Stuck Indicator |
|--------|----------------|
| Network: zero traffic for >60s after initial request | Probable timeout or hang |
| Stdout: same output repeated 3+ times | Agent looping |
| Stdout: no output for >90s | Agent frozen or API timeout |
| Network: sustained outbound without any inbound | Request sent but API not responding |
| Self-report: same state for >120s | Agent stuck in a step |

When stuck is detected:
1. Emit `STUCK` lifecycle event
2. Log to outbox
3. After configurable timeout: kill and dead-drop the request
4. Report to caller via progress notification

---

## Key Sources

- **Anthropic** — concept injection for introspection, comparing self-reported vs actual internal states
- **Factory.ai** — "Signals" system for recursive self-improvement from session analysis
- **OpenHands** — stuck detectors via pattern recognition in action-observation cycles
- **LangChain** — streaming reasoning tokens + custom event dispatch
- **arxiv** — "Work State-Centric Models" with work notes/journals
- **Portkey.ai / Maxim / OpenObserve** — agent observability platforms with distributed tracing
