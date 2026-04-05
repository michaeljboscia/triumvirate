# 034 — Ruflo Source Analysis (Phase 0.1)

**Date:** 2026-04-05  
**Repo:** `https://github.com/ruvnet/ruflo`  
**Local clone:** `/Users/mikeboscia/projects/triumvirate/.phase0_sources/ruflo`  
**Commit analyzed:** `322b2ae`  
**License:** MIT (`/Users/mikeboscia/projects/triumvirate/.phase0_sources/ruflo/LICENSE`)  
**FEAT targets:** FEAT-001, FEAT-010

---

## Scope
This repo is large and polyglot. For Triumvirate v2, the useful pieces are:
- Task/workflow orchestration primitives
- Model routing and cost-aware selection logic
- Swarm coordination surfaces that can be translated to Rust daemon patterns

## Key Source Files Reviewed
- `/Users/mikeboscia/projects/triumvirate/.phase0_sources/ruflo/v3/src/task-execution/application/WorkflowEngine.ts`
- `/Users/mikeboscia/projects/triumvirate/.phase0_sources/ruflo/v3/src/task-execution/domain/Task.ts`
- `/Users/mikeboscia/projects/triumvirate/.phase0_sources/ruflo/v3/@claude-flow/cli/src/ruvector/model-router.ts`
- `/Users/mikeboscia/projects/triumvirate/.phase0_sources/ruflo/v3/@claude-flow/cli/src/ruvector/q-learning-router.ts`

## Patterns Worth Borrowing
1. Explicit task domain object with dependency resolution
- `Task.resolveExecutionOrder()` does topological ordering + priority sorting.
- Direct fit for `fleet_tasks` dependency unblocking and deterministic scheduling.

2. Workflow execution ledger
- `WorkflowEngine` keeps `executionOrder`, per-task timings, and event log entries.
- Direct fit for our workflow event-sourcing rows and dashboard timeline.

3. Cost-aware routing as a first-class concern
- `model-router.ts` combines complexity score, confidence, uncertainty, and cost multipliers.
- We should replicate the shape (not the model implementation):
  - complexity scoring
  - uncertainty gate
  - explicit fallback/escalation path
  - persisted routing telemetry

4. Persistent router state for learning loop
- `q-learning-router.ts` stores state + outcomes to disk, not ephemeral process memory.
- Direct fit for storing routing outcomes in SQLite and improving lead-agent routing over time.

## Patterns to Avoid
1. Monorepo framework sprawl
- Ruflo mixes many concerns and plugins in one codebase.
- Triumvirate should keep strict crate boundaries (`proto`, `agentd`, `workflow`).

2. JavaScript runtime assumptions
- Many APIs are optimized for Node/TS plugin ecosystems.
- We should port only architecture patterns, not runtime structure.

## Triumvirate Adaptation Plan
- FEAT-010 (Fleet)
  - Implement `Task`-like domain model in Rust with explicit dependency graph and priority comparator.
  - Persist execution order and task timing events to SQLite for replay/debug.

- FEAT-001 (Agent pool + supervision)
  - Borrow routing telemetry shape:
    - selected agent/model
    - confidence
    - uncertainty
    - complexity
    - cost estimate
    - outcome status
  - Store in `routing_log` and feed future routing decisions.

## Attribution Guidance (for inline code comments)
Use this exact style in Rust files where patterns are adapted:
- `// Adapted from Ruflo workflow/task orchestration (ruvnet/ruflo, MIT)`
- `// Adapted from Ruflo model-routing telemetry shape (ruvnet/ruflo, MIT)`

## Decision
Ruflo contributes pattern-level guidance for:
- dependency-aware task ordering
- workflow execution telemetry
- cost-aware routing and learning loops

It should not be used as a dependency and should not drive crate structure.
