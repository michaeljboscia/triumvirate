# Research 029: Operational Gaps — WAL, Credentials, Chaos, Turn-Taking

## WAL + Snapshot Recovery — SOLVED
**Two viable options for local daemon persistence:**

### SQLite WAL Mode (Recommended for MVP)
- `PRAGMA journal_mode = WAL` — concurrent reads/writes, crash-safe
- Events table: stream_id, version, event_type, payload
- Snapshots table: stream_id, version, state, created_at
- Recovery: load latest snapshot → replay events after snapshot version → current state
- SQLite is embedded, zero-config, battle-tested, Go bindings mature

### BadgerDB (Alternative)
- Pure Go key-value store, LSM-tree architecture
- ULID keys for chronological ordering
- Prefix scan for efficient tail replay
- Go-native, no CGo dependency (unlike SQLite)

**Key Insight:** NATS JetStream already provides event persistence + replay. SQLite/Badger needed only for local blackboard state that doesn't fit in NATS (agent memory, snapshots, OPA policy cache). Don't duplicate NATS's event store — use it.

## Credential Rotation — SOLVED
**For CLI-first architecture (subscriptions):**
- CLI tools manage their own auth sessions (OAuth, browser auth)
- Daemon monitors subprocess for auth errors (401/403 in output)
- On auth failure: pause tasks → notify user → re-auth subprocess → resume
- Dual-key strategy for API mode: overlap old/new keys during rotation

**For future API mode:**
- `golang.org/x/oauth2` handles token refresh automatically
- Proactive refresh before expiry via `ReuseTokenSourceWithExpiry`
- Distributed lock for concurrent refresh prevention
- Persist refresh tokens in SQLite for daemon restart survival

## Chaos Engineering — SOLVED
**Go chaos testing toolkit:**

| Tool | Use Case |
|---|---|
| **Toxiproxy** (Shopify) | Network faults: `limit_data` for stream truncation, `slicer` for fragmented frames, latency injection |
| **pingcap/failpoint** | Code-level fault injection: panic, error, sleep at marked points. Activated via env vars |
| **rom8726/chaoskit** | `MaybeDelay`, `MaybeError`, `MaybePanic` at controlled points |
| **go-fault** | HTTP middleware fault injection: latency, errors, rejections |
| **Testcontainers** | Spin up Toxiproxy in test containers for integration testing |

**Test scenarios for triumvirate-agentd:**
1. Stream truncation mid-JSON via Toxiproxy `limit_data`
2. Partial JSON frame via Toxiproxy `slicer`
3. Process death via failpoint `panic()` mid-Temporal activity
4. NATS connection loss during debate workflow
5. All 3 CLI subprocesses die simultaneously
6. Daemon crash during event write (WAL recovery test)

## Human-in-the-Loop Turn-Taking — DESIGNED
**Principles for 4-way terminal collaboration:**

### Human Always Has Priority
- Human keypress in input pane immediately queues for processing
- If human types during agent output, agent output continues in its pane but human input takes priority for next action
- ESC key: interrupt current agent generation (sends AbortError to active CLI)

### Interruption Modes (configurable)
- `interrupt`: Stop agent immediately, process human input
- `append`: Let agent finish current sentence, then process human input
- `queue`: Queue human input, process after current agent turn completes

### Attention Management in BubbleTea TUI
- Visual differentiation: each agent gets distinct color/style
- Active agent pane highlighted, idle panes dimmed
- Non-intrusive progress indicators (spinner) for working agents
- Status bar: which agents are thinking/streaming/idle
- Notification badge for debate rebuttals requiring human attention

### Approval Gates
- Destructive operations (git push, file delete, DB write) pause in TUI
- Human sees proposed action as readable text
- Accept/Reject/Edit before execution
- OPA policy determines which actions require human approval

## Sources
medium.com, substack.com, oneuptime.com, sqliteforum.com, microsoft.com, eventsourcing.dev, kurrent.io, barkeywolf.consulting, labex.io, tolubanji.com, kontent.ai, pluto.security, oauth.com, nango.dev, github.com (toxiproxy, failpoint, chaoskit, go-fault), dolthub.com, josephwoodward.co.uk, aiuxdesign.guide, erichorvitz.com, agora.io, precallai.com, flowhunt.io, permit.io, deloitte.com, arxiv.org
