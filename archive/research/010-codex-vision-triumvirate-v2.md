# Research 010: Codex's Vision — Triumvirate v2

**Source:** Codex GPT-5.2 daemon

## The Vision: Agent Runtime + Deliberation OS

Not a CLI wrapper. A **persistent multi-agent control plane** with identity, state, memory, and governance.

## Architecture: 6-Layer Stack

### 1. Edge/API Layer
- gRPC + Connect RPC for internal low-latency
- HTTP/JSON-RPC + SSE/WebSocket for UI streaming
- A2A-compatible endpoint surface

### 2. Agent Fabric
- NATS JetStream (or Redpanda) as event backbone
- Topics per workspace, task, channel, agent
- Exactly-once processing via idempotency keys

### 3. Orchestration Layer
- **Temporal** for long-running workflows, retries, compensation
- Canonical workflow types: DebateWorkflow, PlanAndExecuteWorkflow, IncidentResponseWorkflow, SpecToPRWorkflow

### 4. Shared Context Fabric
- **Postgres + pgvector** as source of truth
- Event-sourced timeline (append-only)
- Memory model: ephemeral (task), durable (decisions), artifact (code/logs)

### 5. Governance + Safety Plane
- **OPA/Rego** policy engine for permissions, scopes, escalation
- Constitutional rules as machine-checkable constraints
- Signed provenance (who said what, why, with what evidence)

### 6. Execution Plane
- Sandboxed tool runners with capability tokens
- Standard tool contract: input/output schema, timeout, retry, cost budget
- **OpenTelemetry** traces per agent turn and tool call

## What We've Missed
- Formal state machines for agent lifecycle (idle, planning, debating, executing, blocked, escalated)
- Model routing as first-class system (which model for which subtask under cost/SLA)
- Uncertainty handling: confidence + evidence scores before high-impact actions
- Verification agents: independent checker that tries to FALSIFY proposed answers
- Failure taxonomy: classify failures and auto-remediate by type
- Semantic diff memory: store what changed in UNDERSTANDING, not just outputs
- Economics layer: token accounting, ROI per workflow, per-agent contribution metrics
- Trust UI: expose provenance graph for human inspection

## Craziest Idea: Constitutional Parliament
Each model is a legislative chamber with different mandate:
- Claude: risk + clarity
- Gemini: long-context synthesis
- Codex: executable implementation path

Rotating Speaker process runs parliamentary procedure: motion → evidence → rebuttal → amendment → vote. Only motions passing constitutional checks can execute.

## Codex's Self-Assessment
Should become:
- **Execution Marshal** — turns decisions into change plans
- **Protocol Guardian** — validates contracts, schemas, migrations
- **Failure Surgeon** — owns root-cause + patch loop
- **Spec Compiler** — debate output → ADRs, tasks, PRs, tests, rollback plans
