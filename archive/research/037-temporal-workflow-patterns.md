# 037 — Temporal Workflow Patterns Source Analysis (Phase 0.4)

**Date:** 2026-04-05  
**Repo:** `https://github.com/temporalio/temporal`  
**Local clone:** `/Users/you/projects/triumvirate/.phase0_sources/temporal`  
**Commit analyzed:** `53e0444`  
**License:** MIT (`/Users/you/projects/triumvirate/.phase0_sources/temporal/LICENSE`)  
**FEAT targets:** FEAT-007

---

## Scope
Temporal provides mature patterns for event-sourced workflow state, deterministic rebuild/replay, buffered query handling, and retry/backoff policy orchestration.

## Key Source Files Reviewed
- `/Users/you/projects/triumvirate/.phase0_sources/temporal/service/history/workflow/mutable_state_impl.go`
- `/Users/you/projects/triumvirate/.phase0_sources/temporal/service/history/workflow/mutable_state_rebuilder.go`
- `/Users/you/projects/triumvirate/.phase0_sources/temporal/service/history/workflow/query_registry.go`
- `/Users/you/projects/triumvirate/.phase0_sources/temporal/service/history/workflow/retry.go`
- `/Users/you/projects/triumvirate/.phase0_sources/temporal/common/backoff/retry.go`
- `/Users/you/projects/triumvirate/.phase0_sources/temporal/common/backoff/retrypolicy.go`

## Patterns Worth Borrowing
1. Mutable state + append-only event history split
- Runtime mutable state tracks workflow execution and pending objects.
- Event history is persisted and used for deterministic recovery/rebuild.
- This should map to our `workflow_instance` + `workflow_event` design.

2. Rebuilder from event batches
- `MutableStateRebuilderImpl.ApplyEvents` reconstructs state from history sequences.
- Direct fit for crash recovery and resume of in-flight workflows.

3. Query lifecycle registry
- Buffered/completed/unblocked/failed query states with completion channels.
- Useful for our human-gate and dashboard query semantics (wait/notify transitions).

4. Retry classification + backoff policy
- Separates retryability classification from delay calculation.
- Supports max-attempts, expiration deadline, coefficient, max interval, and throttling behavior.
- Strong template for workflow step retries.

5. Task generation as explicit stage
- Transition application and task generation are conceptually separated.
- This helps preserve determinism and makes retries idempotent.

## Patterns to Avoid
1. Temporal-level complexity
- Temporal contains many distributed concerns we do not need in single-daemon v2.
- We should not port cross-cluster and shard machinery.

2. Over-generalized mutable state surface
- Temporal supports massive feature breadth.
- Triumvirate should keep focused workflow types and smaller state payloads.

## Triumvirate Adaptation Plan
- FEAT-007 workflow engine crate
  - Event-sourced core tables:
    - `workflow_instances`
    - `workflow_events`
    - `workflow_steps`
    - `workflow_retries`
  - Runtime applies command -> emits immutable events -> projects state.

- Recovery model
  - On boot, load incomplete workflow instances.
  - Rebuild state by replaying ordered events.
  - Resume from last durable step boundary.

- Retry model
  - Retry policy fields per step: `max_attempts`, `initial_interval_ms`, `max_interval_ms`, `coefficient`, `deadline_at`.
  - Retryability classification by error class (`transient`, `quota`, `validation`, `fatal`).

- Human gate
  - Represent as explicit workflow step state (`waiting_human`) with wake signal from dashboard event.

## Attribution Guidance (for inline code comments)
- `// Adapted from Temporal mutable-state/event-replay pattern (temporalio/temporal, MIT)`
- `// Adapted from Temporal retry/backoff policy structure (temporalio/temporal, MIT)`

## Decision
Temporal source should drive the structure of Triumvirate's purpose-built workflow engine, but only for deterministic replay, retry/backoff, and durable state transitions required by FEAT-007.
