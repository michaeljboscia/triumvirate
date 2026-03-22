# Oracle Engine (Pythia) — Complete Guide

## What Is an Oracle?

An oracle is a persistent Gemini knowledge daemon. Unlike a basic daemon (which you spawn, ask questions, and dismiss), an oracle has:

- A **corpus** — a managed set of files that define what the oracle knows
- **State** — interaction history, degradation signals, context pressure metrics
- **Lifecycle management** — checkpoint, salvage, reconstitute, decommission
- **Registry** — tracked across projects in `~/.pythia/registry.json`

Think of it as a research assistant that remembers everything across sessions. You load documents into it, ask questions over days or weeks, and it maintains context. When its context window fills up, you checkpoint (compress its knowledge) and reconstitute (start a new generation with the compressed knowledge).

---

## When to Use an Oracle vs a Basic Daemon

| Use Case | Use This |
|----------|----------|
| Quick question for Gemini | `send_message` (fire-and-forget) |
| Multi-turn conversation, single session | `spawn_daemon` / `ask_daemon` |
| Research across multiple sessions | **Oracle** |
| Codebase context that persists | **Oracle** |
| Loading 20+ documents for reference | **Oracle** |

Rule of thumb: if you'd be annoyed re-loading context every session, use an oracle.

---

## Quick Start

```
1. oracle_init({ project: "my-project" })
   → Creates registry entry + empty manifest

2. oracle_add_to_corpus({ oracle_id: "...", file_path: "/path/to/important-doc.md" })
   → Adds a file to the manifest (repeat for each file)

3. spawn_oracle({ oracle_id: "...", session_name: "my-project-oracle" })
   → Spawns Gemini daemon, loads corpus files into context

4. ask_daemon(daemon_id, "Based on the architecture doc, what's the best approach for X?")
   → Oracle answers using its loaded context

5. oracle_pressure_check({ oracle_id: "..." })
   → Reports how much context is used (e.g., "62% — healthy")

6. oracle_checkpoint({ oracle_id: "..." })
   → Oracle synthesizes its knowledge into a checkpoint document

7. oracle_reconstitute({ oracle_id: "..." })
   → Creates generation v2 from the checkpoint, fresh context window
```

---

## The Corpus

An oracle's corpus is defined by its **manifest** — a JSON file listing every file the oracle should know about.

### Entry Types

**Static entries:** Files you explicitly add. Each has a path, hash, and role:
```json
{
  "path": "/path/to/architecture.md",
  "hash": "sha256:abc123...",
  "role": "reference",
  "added_at": "2026-03-22T04:00:00Z"
}
```

**Live sources:** Files that are tracked for changes. When you call `oracle_sync_corpus`, the system re-reads these files and pushes updates to the running daemon.

### Corpus Roles

- `reference` — Foundational knowledge (architecture docs, specs, design decisions)
- `context` — Current working context (session logs, recent changes)
- `training` — Examples and patterns the oracle should learn from

---

## The Lifecycle

### Birth: `oracle_init` + `spawn_oracle`

`oracle_init` creates the registry entry and an empty manifest. `spawn_oracle` starts the Gemini daemon and loads all corpus files. If a session with the same `session_name` already exists on disk, Gemini resumes it — zero re-feed cost.

### Growth: `oracle_add_to_corpus` + `oracle_sync_corpus`

As you add files and the oracle processes queries, its context fills up. `oracle_sync_corpus` pushes changed files to the running daemon. `oracle_log_learning` records interactions.

### Monitoring: `oracle_pressure_check` + `oracle_quality_report`

`oracle_pressure_check` computes what percentage of the context window is used and recommends an action:
- **Healthy** (< 50%) — Keep working
- **Warming** (50-75%) — Consider checkpointing soon
- **Critical** (75-90%) — Checkpoint now
- **Emergency** (> 90%) — Salvage immediately

`oracle_quality_report` looks at degradation signals: Are responses getting shorter? Is the oracle making more errors? Is the corpus stale?

### Preservation: `oracle_checkpoint`

Asks the oracle to produce a checkpoint — a synthesized, compressed representation of everything it knows. The checkpoint is written to disk and becomes the seed for the next generation.

The oracle itself writes the checkpoint. You're not extracting information — you're asking the oracle to distill its own knowledge. This produces better compression than external summarization because the oracle knows what's important in its own context.

### Rebirth: `oracle_reconstitute`

Creates a new generation (v→v+1). Spawns a fresh Gemini daemon, loads the checkpoint document, and re-adds the corpus manifest. The oracle has fresh context but inherits the accumulated knowledge via the checkpoint.

### Emergency: `oracle_salvage`

When a daemon dies unexpectedly (process crash, quota exhaustion, network failure), salvage attempts to recover whatever is still accessible. It probes the daemon, extracts what it can, and writes an emergency checkpoint. This is the safety net — you lose some context but not everything.

### Death: `oracle_decommission_*`

Permanent destruction of an oracle and all its state. Intentionally difficult — 7 validation gates:

1. Must call `oracle_decommission_request` first (returns a time-limited token)
2. Token has a TTL (expires if you wait too long)
3. Must provide the token back in `oracle_decommission_execute`
4. TOTP verification (time-based one-time password)
5. Confirmation phrase
6. Registry integrity check
7. State file existence verification

Why so many gates? Oracles can hold months of accumulated knowledge. Accidental deletion is catastrophic and irreversible. The gates ensure you really, truly mean it.

`oracle_decommission_cancel` cancels an active request if you change your mind.

---

## Error Handling

Every oracle tool returns `OracleResult<T>`:

```typescript
// Success
{ ok: true, data: { /* tool-specific response */ } }

// Failure
{
  ok: false,
  error: {
    code: "DAEMON_BUSY_QUERY",        // Machine-readable error code
    message: "Daemon is processing another query",  // Human-readable
    retryable: true,                   // Can the caller retry?
    details: { daemon_id: "abc123" }   // Additional context
  }
}
```

### Error Codes

**Daemon state errors:**
- `DAEMON_BUSY_QUERY` — Another query is in flight. Retryable (auto-waits).
- `DAEMON_BUSY_LOCK` — An operation lock is held. Surface to user.
- `DAEMON_DEAD` — Daemon process has exited. Try `spawn_oracle` to restart.
- `DAEMON_QUOTA_EXHAUSTED` — All models in the fallback chain are exhausted.

**Oracle state errors:**
- `ORACLE_NOT_FOUND` — No oracle with that ID in the registry.
- `ORACLE_ALREADY_EXISTS` — Trying to init an oracle that already exists.
- `ORACLE_NOT_SPAWNED` — Operation requires a running daemon.

**Operation errors:**
- `CHECKPOINT_FAILED` — Oracle couldn't produce a checkpoint.
- `LOCK_TIMEOUT` — Couldn't acquire operation lock in time.
- `SYNC_FAILED` — Corpus sync failed (file not found, permission denied, etc.)

**Decommission errors:**
- `DECOMMISSION_TOKEN_EXPIRED` — Took too long between request and execute.
- `TOTP_INVALID` — Wrong verification code.
- `DECOMMISSION_NOT_REQUESTED` — Trying to execute without requesting first.

---

## Configuration

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `PYTHIA_HOME` | `~/.pythia` | Root directory for oracle state |
| `PYTHIA_REGISTRY_PATH` | `~/.pythia/registry.json` | Oracle registry file |
| `GEMINI_API_KEY` | (required) | Gemini API access |
| `AI_MEMORY_DIR` | `~/.ai-memory` | Session log storage |

### Permissions

Oracle tools need explicit permission in Claude Code. Copy `starter-kit/claude/settings.local.json.example` to `~/.claude/settings.local.json`.

Safe tools (auto-allowed): `oracle_init`, `oracle_health`, `oracle_refresh`, `oracle_sync_corpus`, `oracle_pressure_check`, `oracle_log_learning`, `oracle_checkpoint`, `oracle_salvage`, `oracle_reconstitute`, `oracle_quality_report`, `oracle_add_to_corpus`, `oracle_update_entry`, `spawn_oracle`.

Destructive tools (manual approval): `oracle_decommission_request`, `oracle_decommission_cancel`, `oracle_decommission_execute`.

---

## Architecture

```
oracle-tools.ts (4,946 lines)
    │
    ├── oracle-types.ts (338 lines) — types, constants, error codes
    │
    └── runtime.ts (632 lines) — GeminiRuntime singleton
         │                         OracleRuntimeBridge interface
         │                         executeWithFallback / spawnWithFallback
         │
         └── model-fallback.ts — quota tracking, model chain
              cli-executor.ts — process spawning engine
```

**Zero circular dependencies.** oracle-types.ts has no runtime imports. oracle-tools.ts imports from types and runtime. runtime.ts does NOT import oracle-tools.ts. The bridge interface ensures clean separation.

**Singleton GeminiRuntime:** All daemon operations go through a single instance protected by an async mutex (`toolMutex`). This prevents race conditions when multiple oracle tools fire concurrently (e.g., two hooks both calling `oracle_pressure_check`).

---

## Importing the Oracle Engine

The oracle engine is included in the MCP server build by default. When the Gemini MCP server starts, it registers both regular Gemini tools AND oracle tools:

```typescript
// gemini/server.ts
registerGeminiTools(server);    // 10 basic daemon tools
registerOracleTools(server);    // 17 oracle tools
```

**If you don't want the oracle engine:** You can remove the `registerOracleTools(server)` line from `gemini/server.ts` and rebuild. The basic daemon tools will work fine without it. The oracle is additive — removing it doesn't break anything.

**If you want ONLY the oracle engine:** That's not supported — the oracle depends on the runtime bridge which depends on the basic daemon infrastructure. You need both.
