# ADR-001 — Data Plane Segregation for Triumvirate/Pantheon

**Status:** Proposed (revision 3, 2026-04-19)
**Deciders:** Mike Boscia + Claude Opus 4.7
**Revision history:**
- r1 (2026-04-19 04:20 ET): initial 4-tier draft
- r2 (2026-04-19 05:30 ET): Twin r1 feedback baked in; Tier 0 added; Tier 1 bounded; Tier 2 rationale = failure-domain isolation not throughput; JetStream deferred to Zeus via JSONL bridge
- r3 (2026-04-19 05:55 ET): Twin r2 feedback baked in; **"headroom multiplier" killed**, replaced with per-event-type SLO envelopes + blast-radius framing; per-event disposition table added; JSONL↔JetStream parity-test contract + conformance tests added; Tier 0 concrete tool (1Password CLI + Keychain) pinned; stenographer dedupe strategy named (SQLite dedupe table); **trace-capture promoted to Wave 0.5** as the prerequisite for any Tier 1 claim that survives production

---

## Context

Triumvirate/Pantheon is a unified operating environment for multi-agent (Claude + Gemini + Codex) work. Prior architecture drifted into ad-hoc per-subsystem choices that failed in characteristic ways: silent pipelines (stenographer v1, claude-mem), filesystem polling (codex rollout logs), scattered credentials, no cross-agent correlation.

This ADR defines a five-tier segregation rule (Tier 0 orthogonal + four data tiers) and — critically — **refuses to make durable capacity claims from synthetic benchmarks alone**. Prior drafts (r1, r2) leaned on "30-100× headroom" framing that conflated wave-throughput with sustained capacity and ignored per-event-type SLOs. r3 replaces that with:
1. Per-event-type SLO envelopes (placeholders where SLOs are not yet pinned)
2. Per-tier blast-radius analysis driving availability/RTO decisions
3. A trace-capture + replay program (Wave 0.5) that produces evidence grounded in real Pantheon behavior

## Decision

**Every data domain is classified into exactly one tier. Tier 0 governs access; Tiers 1–4 define storage + concurrency + retention.**

### Tier 0 — Secrets & Configuration

- **Storage — Mac-era (now):** macOS Keychain as primary; **1Password CLI (`op`)** for on-demand fetch of shared secrets across machines. Single Rust resolver module (`triumvirate-secrets`) fronts both.
- **Storage — Zeus-era:** `sops` + `age` for encrypted-in-repo config, with 1Password CLI still the interactive fetch layer. GCP Secret Manager considered but NOT chosen — the Gemini Ultra credit covers other GCP spend, but adds a cloud dependency Pantheon does not otherwise need on-host.
- **What lives here:** all API keys (Anthropic/OpenAI/Google/GitHub/HubSpot/Cloudflare), Supabase service-role keys, nats-server config when it lands, model endpoints, per-tier retention policies, feature flags, subject-hierarchy config.
- **Hard rules:**
  - No subsystem in Tiers 1–4 reads credentials from anywhere but Tier 0.
  - Tier 2 messages carry token-references, never tokens.
  - Config that changes behavior of other tiers lives here. Changes version-controlled where non-secret, never in application code.
- **Resolver failure policy (Codex r2 gap):** if Tier 0 resolver fails, daemon refuses to start (fail-closed). No fallback to env-var scrape. Panic with a clear message naming the missing secret.
- **Consolidation is Sprint 0.5:** credentials today scattered across `.env`, `~/.anthropic/`, `~/.codex/`, `~/.claude/infrastructure.md`, shell exports. First task before any Tier 1+ work.

### Tier 1 — Transactional State

- **Storage:** SQLite + WAL + `busy_timeout=5000ms` + sqlx async pool (`max_connections = N+4`).
- **What lives here:** peer-review workflow, fleet agent-session registry, task manifests, cost aggregates-of-record, crystallized lessons.
- **Capability claim (honest form, empirical bounds):**
  > On a consumer SSD Mac laptop with 3-row write-only peer-review-style transactions:
  > - Wave profile (steady 1op/s/worker): p99 < 250 ms up to N=500, aggregate throughput up to 487 ops/sec, 0 BUSY retries.
  > - Sustained profile (sparse bursty 3-10s gaps): p99 crosses 500 ms at N≈100, reaches 1.6s at N=500, still 0 BUSY retries.
  > - Herd profile (synchronized 30s barriers): wall-clock mutates past target duration at N≥500 — workload becomes drain-bound on fsync queue.
  > - Zero ops failed. Zero BUSY retries. Degradation mode is queuing, not failure.
  >
  > These are capability bounds, NOT capacity claims. A capacity claim requires per-event-type SLOs (below) evaluated against real workload traces (Wave 0.5). Wave 3 Zeus re-run required before any production claim.
- **Per-event-type SLO envelope (placeholders, to be pinned by Wave 0.5 trace analysis):**

| Event type | Target p99 commit latency | Max acceptable BUSY rate | Target RTO on crash | Data-loss budget |
|---|---|---|---|---|
| peer-review decision | < 200 ms | 0.1% | < 30 s | zero (ACID) |
| fleet worker lifecycle | < 500 ms | 0.5% | < 60 s | zero (replay from Tier 2) |
| agent session registry | < 100 ms | 0.1% | < 30 s | zero (ACID) |
| cost aggregate-of-record | < 1000 ms | 1.0% | < 5 min | < 1% drift tolerated |
| crystallized lesson | < 500 ms | 0.5% | < 5 min | zero (rare write) |

- **Blast radius** (Gemini r2 sharpest question):

| Failure | Impact | Mitigation |
|---|---|---|
| SQLite corruption | Peer-review halts; agent sessions orphaned; cost billing drifts. Severity: **high** | Daily SQLite `.backup` to Tier 4 archive; integrity-check on daemon start; WAL replay validated by Wave 0.5 crash matrix |
| Single-host hardware loss | Entire Tier 1 gone until restore. Severity: **high** | Continuous SQLite backup shipped to remote (Tier 4); RTO target 1h from bare-metal |
| Single-writer saturation | Backlog grows, p99 breaches SLO, BUSY retries climb. Severity: **medium** | Monitor SLO envelope; trigger multi-host migration (T1c) when production sustains > 70% of capability bound |

- **Hard rule:** one daemon-owned DB, multiple schemas. No subsystem hosts its own file.

### Tier 2 — Context Capture Event Stream

- **Storage — Mac-era (now):** JSONL append files at `~/.triumvirate/streams/<subject>/<YYYY-MM-DD>.jsonl`. Rotated daily. Consumer offsets tracked in SQLite (Tier 1, dedicated `stream_offsets` table). Atomic segment-id + offset writes (Codex r2 gotcha fix).
- **Storage — Zeus-era:** NATS JetStream sidecar (`nats-server`), launchd/systemd managed. Swap is a one-module change via `triumvirate-stream` trait.
- **What lives here:** fleet worker state transitions, inter-agent message bus, dispatch task queues, tool-call events, cost/token telemetry, lesson-candidate events, peer-review trigger events.
- **Why sidecar when JetStream lands:** failure-domain isolation (embedded broker crash must not take daemon down), connection count (dozens of clients per host), independent lifecycle, multi-host readiness.
- **Subject hierarchy:** `pantheon.<env>.<project>.<domain>.<event>` (e.g. `pantheon.prod.old-iron.worker.spawned`). Schema version goes in **payload headers, not subject** (Codex r2).
- **Per-subject retention + uniformity caveat (Gemini r2):** not every subject has the same replay/retention needs. Declared per-subject in Tier 0 config:

| Subject domain | Retention | Replay required? | Ordering |
|---|---|---|---|
| worker.* | age-based 30d | yes | per-worker |
| agent.* | age-based 7d | yes | per-agent-pair |
| peer-review.* | limits + max-age 90d | yes | per-review-id |
| tool-call.* | age-based 7d | rebuild-only (stenographer) | per-session |
| lesson.* | age-based 90d | yes | none |
| cost.* | age-based 30d | yes | per-day-aggregate |

- **Source-of-truth rule (Codex r1, kept):** Tier 1 canonical. Tier 2 replay = side-effect reprocessing, NOT authoritative state reconstitution.
- **Idempotency contract:** at-least-once delivery. Every event carries `event_id` (UUIDv7 with embedded timestamp). Consumers idempotent by event_id or maintain dedupe log.
- **JSONL↔JetStream parity contract (Codex r2 gap — r3 addition):** before swap from JSONL bridge to JetStream, same event corpus replayed through both backends must produce:
  1. Byte-identical consumer output per projection
  2. Equal dedupe counts
  3. Equal DLQ counts
  4. Lag within 2× of target SLO under fault injection (kill-9, disk pressure)
  Parity test is a gate criterion, not a goal.

### Tier 3 — Narrative Projections

- Tier 3 is **a class of Tier 2 consumers that produce human-readable artifacts**. Not a storage tier.
- **Stenographer v2** — narrow Claude Haiku 4.5 consumer of `pantheon.*.*.tool-call.*`. Emits markdown session notes at `~/projects/<project>/<YYYYMMDD>_session_v<N>.md`. Single dependency (Haiku API). Triggers: (a) token-delta ~10K (b) compaction boundary (c) explicit `/session-notes-full`.
  - **Dedupe strategy (Codex r2 gap):** SQLite dedupe table `stenographer_processed_events (event_id PRIMARY KEY, session_id, emitted_at)` — before Haiku is called, check table; after successful markdown append, insert. TTL = 30d. Atomic w/ the markdown write via filesystem rename.
- **Other projections:** lesson crystallizer, cost rollup, peer-review notifier — each a consumer with its own dedupe + SLO.
- **Projection failure handling:** empty/malformed LLM output → skip + ack + log to DLQ. Human can rebuild via replay once projection is idempotent.

### Tier 4 — Analytic / Ephemeral

- **Storage:** JSONL append files rotated daily, DuckDB on-demand queries. Retained 90d then pruned.
- **What lives here:** stress-test matrix results, performance benchmarks, cost rollups, GCP evidence bundles, stenographer session-log archives.
- **Why:** DuckDB over JSONL is zero-setup, orders of magnitude cheaper than Postgres for analytics. Non-durable in Tier 2 sense — loss breaks nothing.

---

## Conformance Tests (Codex r2 gap — r3 addition)

Each tier is "done" only when its conformance test passes. Gate criteria:

| Tier | Conformance test |
|---|---|
| 0 | No application code reads env vars or `.env` for secrets directly. `grep -rE 'std::env::var.*KEY\|env!.*API' daemon/` returns zero matches outside `triumvirate-secrets` crate. Daemon refuses to start when resolver fails. |
| 1 | Per-event-type SLO envelopes met under Wave 0.5 replayed trace. Crash-trial `all_integrity_ok` true across 100 trials. Backup+restore round-trip verifies row counts + critical-table checksums. |
| 2 | Parity contract (above) passes between JSONL and JetStream backends. Every consumer declares `lag_sla_ms`, `redelivery_cap`, `dlq_subject`, `alerting_owner`. |
| 3 | Stenographer dedupe table prevents double-appends on forced Tier 2 replay. Each projection survives its backend-swap parity test. |
| 4 | Oldest retained file age > 90d triggers automatic pruning. DuckDB query latency < 30s on full retained window. |

---

## Migration / Current State Mapping

| Subsystem | Current | Tier | Action |
|---|---|---|---|
| API keys etc. | scattered | 0 | Consolidate to Keychain + 1Password CLI; single resolver |
| Non-secret config | hardcoded | 0 | TOML/YAML under `~/.triumvirate/config/` |
| peer-review crate | SQLite | 1 | Keep |
| fleet AgentLauncher + worker tracking | in-process state | 2 + 1 | Emit lifecycle to Tier 2; keep registry in Tier 1 |
| codex worker liveness | filesystem polling | 2 | Migrate to `pantheon.*.*.worker.codex.*` |
| stenographer v1 | disabled | 3 | Rebuild as Haiku consumer w/ SQLite dedupe |
| claude-mem | dead | 3 | Retire; Haiku stenographer replaces |
| lessons | markdown + ad-hoc | 1 + 2 | Candidates on stream; crystallized in Tier 1 |
| cost telemetry | ad-hoc | 2 + 4 | Events on stream; daily rollups to Tier 4 |
| stress-test results | JSON | 4 | Keep |

---

## Known Limitations and Open Questions

### Tier 1 SQLite

1. Evidence from Mac laptop only. Zeus/Vulcan NVMe retest required before production claims.
2. Long-held transactions: p99 = 3.3s at N=30 hold=100ms (smoke). Not matrix-characterized.
3. Read-heavy mixed workloads: smoke at 25% reader clean; higher ratios untested.
4. N>500 untested (Mac memory pressure aborted those runs).
5. Crash-recovery: 3/3 smoke trials integrity-ok. 100-trial Wave 0.5 confirmation required.
6. macOS F_BARRIERFSYNC vs Linux fdatasync durability gap — known but not quantified.
7. `busy_timeout=5000ms` is provisional, not SLA-grounded.

### Tier 2

8. JSONL bridge file rotation + atomic offset checkpoint is the most likely place to leak/dup events. Wave 0 contract module carries this as critical.
9. JetStream version pin + upgrade path TBD before Zeus standup.
10. Per-event authoritative disposition for dual-write domains is in the per-event SLO table above but needs review once Wave 0.5 traces land.

### Tier 3

11. Stenographer Haiku prompt template design: separate spec, not in this ADR.
12. Projection replay idempotency tested by Wave 0 parity contract.

### Scope

13. Multi-host switchover criteria: concurrent write-host = hard trigger; 70% of capability bound = soft trigger.
14. Pythia (local code search) not covered here.
15. Pantheon v4 UI direction note: "Ratatui TUI" directive from prior memory is reconsidered — wterm (vercel-labs/wterm, Zig→WASM VT100 DOM-rendered terminal) would collapse v4 to a PTY-broker + Next.js panel grid. Not part of this ADR; logged as a separate decision.

---

## Revisit Triggers

**T1 — Tier 1:**
- T1a. Three consecutive wave runs on target hardware breach p99>500ms OR BUSY>0.5% OR error>0.1%
- T1b. Production shows long-held transactions (>100ms sustained) with BUSY >1% over 24h
- T1c. Pantheon adds a second concurrent write-host
- T1d. Per-event SLO envelope (table above) breached by any event type over 7d rolling window

**T2:**
- T2a. JSONL bridge lag > 10× `lag_sla_ms` for any consumer over 1h
- T2b. JetStream sidecar memory/disk > 2GB on Zeus baseline w/o cluster
- T2c. User-visible duplicates in peer-review or cost aggregation despite idempotency keys

**T3:**
- T3a. Haiku stenographer lag > 10 min under normal load

**T4:**
- T4a. DuckDB query > 30s on 90-day window

---

## Implementation Order

**Wave 0 — Contracts (parallelizable within):**
1. Tier 0 consolidation: `triumvirate-secrets` crate, Keychain + 1Password CLI resolver, migrate scattered credentials. Conformance test: grep for direct env reads.
2. Tier 2 JSONL bridge: append writer, atomic offset tracking, rotation. Trait-abstracted for JetStream swap.
3. Shared event schema (Appendix B) — idempotency keys, subject layout, per-subject retention.

**Wave 0.5 — Trace Capture (r3 promotion — PREREQUISITE for Tier 1 claims):**
4. Instrument daemon emission points to emit events matching Appendix B schema into JSONL files under `~/.triumvirate/traces/<YYYY-MM-DD>.jsonl`.
5. Run instrumented daemon for 24-72h under normal Pantheon usage.
6. Add `trace-replay` profile to stress harness consuming the JSONL → replaying inter-arrival times + transaction shapes.
7. Re-evaluate Tier 1 SLO envelopes against replayed trace. Pin actual SLO numbers (replacing placeholders above).

**Wave 1 — Parallel (shared contracts from Wave 0):**
8a. Stenographer v2 Haiku worker + dedupe table
8b. Codex worker liveness via `pantheon.*.*.worker.codex.*`

**Wave 2:**
9. Peer-review event dual-publish (SQLite + stream)
10. Cost telemetry stream
11. Lesson-candidate stream + crystallizer

**Wave 3 — Hardware-contingent:**
12. Zeus NVMe re-run of full stress + trace-replay matrix. Closes T1 triggers proactively.
13. JetStream sidecar standup. Swap JSONL bridge. Parity contract gates the swap.

---

## Appendix A — Evidence

Break-matrix harness: `daemon/stress-tests/sqlite-concurrency/`.

**N=30-250 matrix (Mac, 60s each):** 2,550+ ops, 0 BUSY, 0 fail. Wave clean to N=250 (p99=149ms). N=250 sustained breached (p99=693ms); N=250 herd variance 493-776ms.

**N=500 partial (today):**
- Wave: **29,203 ops / 60s = 487 ops/sec**, p99=237ms, 0 BUSY, 0 fail
- Sustained: 80 ops/sec, p99=1,636ms, 0 BUSY, 0 fail
- Herd: wall-clock mutation (60s → 12+ min drain)

**Smokes (new profiles):** long-tx hold=100ms N=30 → p99=3,258ms (regime-change confirmed); mixed-rw 25% reader N=30 → p99=96ms (WAL concurrent reads behave); crash-trial 3 trials → all_integrity_ok true.

**Honest framing:** these numbers are **capability bounds** on a noisy Mac. Capacity claims require Wave 0.5 traces.

**Full evidence pack:** `results/EVIDENCE_PACK.md` via `evidence-pack` binary (commit `6eca9c9`).

---

## Appendix B — Event Schema (contract for Wave 0 + 0.5 workers)

All Tier 2 events conform to this Rust struct. Serialized as JSONL (Mac-era) or NATS protobuf/JSON (Zeus-era). Consumers parse `headers.event_type` before body.

```rust
struct TraceEvent {
    // Headers — stable across schema versions
    event_id: Uuid,            // UUIDv7, embeds timestamp
    event_type: String,        // e.g. "tool_call.completed", "worker.spawned"
    subject: String,           // pantheon.<env>.<project>.<domain>.<event>
    schema_version: u16,       // payload schema version per event_type
    emitted_at: DateTime<Utc>,
    correlation_id: Option<Uuid>,  // session, review, dispatch — whichever groups this event

    // Body — event-type specific
    payload: serde_json::Value,
}
```

**Initial event_types to instrument (Wave 0.5 scope):**

| event_type | Payload keys | Emission point in daemon |
|---|---|---|
| `tool_call.started` | agent, tool_name, session_id | Claude/Codex/Gemini adapter dispatch |
| `tool_call.completed` | agent, tool_name, session_id, duration_ms, bytes_in, bytes_out, success | Same adapter, on completion |
| `worker.spawned` | worker_id, agent, task_id, worktree_path | `fleet::orchestrator` spawn path |
| `worker.state_changed` | worker_id, from, to, reason | fleet status transitions |
| `worker.completed` | worker_id, commit_sha, duration_ms, exit_code | fleet reap |
| `peer_review.requested` | request_id, reviewer_agent, target_ref | `peer-review` crate |
| `peer_review.decided` | request_id, decision, reviewer_agent, duration_ms | `peer-review` crate |
| `cost.token_usage` | agent, model, input_tokens, output_tokens, session_id | adapter token-count hook |
| `cost.api_call` | agent, endpoint, status, duration_ms, usd_cost_estimate | adapter HTTP layer |
| `lesson.candidate` | project, agent, category, text_snippet | lesson capture trigger |

Wave 0.5 instruments these emission points to write events into `~/.triumvirate/traces/<YYYY-MM-DD>.jsonl`. Wave 0.5 trace-replay stress-harness profile reads these back and replays inter-arrival timing to answer the real capacity question.
