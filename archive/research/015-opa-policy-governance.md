# Research 015: OPA/Rego — Policy-as-Code for Agent Governance

**Confirmed:** OPA embeds directly into Go as a library. No sidecar needed.

## What OPA Does for Triumvirate
- Evaluates every agent action against policies BEFORE execution
- Rego policies: versioned, tested, deployed like code
- Supports RBAC, ABAC, and relationship-based access control
- Microsoft Agent Governance Toolkit (Apr 2026) already supports OPA Rego

## Concrete Policies We'd Write
- "Codex cannot git push without human approval" → OPA policy
- "No agent can delete files outside its assigned worktree" → OPA policy
- "Debate claims without data field are rejected" → OPA policy
- "Cost budget per workflow: $X max" → OPA policy
- "Destructive operations require 2-of-3 agent consensus" → OPA policy

## Sources
hoop.dev, openpolicyagent.org, nexastack.ai, medium.com, permit.io, microsoft.com, styra.com
