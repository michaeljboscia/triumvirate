# 036 — swarms-rs Source Analysis (Phase 0.3)

**Date:** 2026-04-05  
**Repo:** `https://github.com/The-Swarm-Corporation/swarms-rs`  
**Local clone:** `/Users/you/projects/triumvirate/.phase0_sources/swarms-rs`  
**Commit analyzed:** `9d22ba9`  
**License:** Apache-2.0 (`/Users/you/projects/triumvirate/.phase0_sources/swarms-rs/LICENSE`)  
**FEAT targets:** FEAT-001, FEAT-023

---

## Scope
swarms-rs is relevant for lifecycle and orchestration mechanics in Rust: builder-driven agent config, loop/retry semantics, state persistence, and concurrent/sequential swarm workflows.

## Key Source Files Reviewed
- `/Users/you/projects/triumvirate/.phase0_sources/swarms-rs/swarms-rs/src/agent/swarms_agent.rs`
- `/Users/you/projects/triumvirate/.phase0_sources/swarms-rs/swarms-rs/src/structs/concurrent_workflow.rs`
- `/Users/you/projects/triumvirate/.phase0_sources/swarms-rs/swarms-rs/src/structs/sequential_workflow.rs`
- `/Users/you/projects/triumvirate/.phase0_sources/swarms-rs/swarms-rs/src/structs/rearrange.rs`

## Patterns Worth Borrowing
1. Builder-first lifecycle configuration
- Agent configuration via builder includes `max_loops`, `retry_attempts`, persistence dir, and tool wiring.
- Directly maps to daemon connector config and supervisor policy setup.

2. Explicit bounded execution loops
- Execution loop is capped by `max_loops` with early completion exits.
- Strong guardrail for runaway agent turns and predictable resource usage.

3. Structured retry discipline
- Retry attempts around model/tool calls with per-attempt handling.
- Good base for connector-level retry policy before escalating to workflow-level retry.

4. Concurrency patterns for fanout/collect
- `for_each_concurrent` + channel collection in concurrent workflow.
- Useful for fleet fanout phases and result aggregation.

5. Mixed sequential + parallel flow primitive
- Rearranged flow supports both sequential and parallel steps with explicit flow declarations.
- Strong conceptual model for FleetWorkflow wave execution.

## Patterns to Avoid
1. `unwrap` in runtime paths
- There are non-test `unwrap`/`expect` usages in async flows.
- Triumvirate production paths must remain panic-free.

2. Loose shared-state assumptions
- Some patterns rely on in-memory stores and clone-heavy state without strict durability semantics.
- Triumvirate needs SQLite-backed source of truth for restart safety.

## Triumvirate Adaptation Plan
- FEAT-001 (agent pool)
  - Implement builder-like config for connector instances and supervisor policies.
  - Enforce bounded loops/turn limits and retry attempts per connector.

- FEAT-023 (health)
  - Emit lifecycle state transitions as first-class events: spawned, ready, busy, retrying, degraded, dead.
  - Couple health state with retry counters and last-success timestamp.

- FEAT-010 (fleet)
  - Adopt concurrent fanout and sequential/parallel wave execution model from workflow patterns.

## Attribution Guidance (for inline code comments)
- `// Adapted from swarms-rs agent lifecycle loop/retry pattern (The-Swarm-Corporation/swarms-rs, Apache-2.0)`
- `// Adapted from swarms-rs concurrent workflow fanout/collect pattern (Apache-2.0)`

## Decision
swarms-rs is valuable as Rust-native prior art for lifecycle controls and orchestration mechanics, with selective adoption under stricter panic-free + durable-state constraints.
