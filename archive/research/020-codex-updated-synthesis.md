# Research 020: Codex's Updated Synthesis (Post-Research)

## Concrete MVP Spec

### Binary: `triumvirate-agentd`
Modules: fabric (NATS), orchestrator (Temporal), policy (OPA), debate (Toulmin), memory (blackboard), telemetry (OTel), connectors (Claude/Gemini/Codex adapters)

### 7 Protobuf Contracts Defined
1. AgentCard — identity, roles, capabilities, cost profile
2. TaskEnvelope — goal, constraints, deadline, priority
3. ToulminArgument — claim/data/warrant/rebuttal/confidence
4. ToolActionRequest — tool, input, risk level, idempotency key
5. PolicyDecision — allow/deny, policy ID, reason, obligations
6. CodePatchArtifact — files, diff, AST validity, test plan
7. ExecutionOutcome — status, cost, duration, selected argument

### Temporal Workflow: DeliberateThenExecute
10 activities: CollectContext → RequestArguments → ValidateToulmin → ScoreAndSelect → PolicyPrecheck → ExecuteAction → ValidateAST → HumanApproval → Finalize → EmitAuditTrail

### Wave Plan
- Wave 1 (4-6 wk): Core daemon, one workflow, Toulmin + OPA + Tree-sitter + OTel
- Wave 2 (4-8 wk): Safe speculation, blackboard memory, role specialization, debate scoring
- Wave 3 (8-12 wk): CRDTs, multi-workflow library, constitutional packs, external federation

### Key Decision
Constitutional first (Wave 1), speculation second (Wave 2), write-path speculation last (Wave 3). Preserves trust while unlocking speed.

### Headline
"Triumvirate v2 is a local-first, policy-governed, event-driven multi-agent runtime with structured debate and durable execution."
