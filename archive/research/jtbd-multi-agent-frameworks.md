# JTBD for Multi-Agent System Design -- Research Findings

**Date:** 2026-04-05
**Source:** niksacdev/multi-agent-system + ModernAnalyst JTBD-Enhanced User Story Framework

---

## 1. JTBD Structure (Functional / Emotional / Social)

The niksacdev repo defines three job tiers per agent:

| Tier | Definition | Example (Intake Agent) |
|------|-----------|------------------------|
| **Functional** | What needs to be accomplished | Submit info once, get immediate validation, avoid re-entry |
| **Emotional** | How the user wants to feel | Confident the process works, reduced anxiety, trust in security |
| **Social** | How the user wants to be perceived | Demonstrate competence, maintain credibility, build reputation |

**Primary Job Statement format:**
> "When I need [situation], I want [motivation], so I can [outcome]."

---

## 2. How They Define "Jobs" for Multi-Agent Coordination

Each agent maps to a customer job, not an internal process. The pattern:

```
Agent = Job Owner
  Primary Job: one-sentence customer outcome
  Functional Jobs: 4-5 concrete task completions
  Emotional Jobs: 4-5 feeling-states to achieve
  Pain Points Addressed: what currently goes wrong
  Value Created: what the agent uniquely delivers
  Success Metrics: measurable customer outcomes
```

The orchestrator agent's job is meta-coordination: "Get a well-reasoned decision I can act on with confidence." It owns the job-flow, not the sub-jobs.

---

## 3. Templates and Mapping Frameworks

### Job Story Template (ModernAnalyst)
> "When [situation], [persona] wants to [goal] so that [outcome]."

### Agent Persona Template (niksacdev)
```markdown
## Agent Identity & Role
[Technical capabilities]

## Jobs-to-be-Done Focus
**Primary Customer Job**: [What customer wants to accomplish]
**Key Outcomes You Enable**: [How you help them succeed]
**Success Metrics**: [Measurable outcomes]

## Business Domain Knowledge
[Industry-specific context]
```

### Job Mapping Process (per interaction)
1. Triggering Event -- what caused the job to start?
2. Job Steps -- what are they trying to accomplish at each stage?
3. Desired Outcomes -- what would success look like?
4. Pain Points -- what typically goes wrong?
5. Emotional Needs -- how do they want to feel throughout?

### Hierarchical Backlog (ModernAnalyst)
- **Job Backlog**: high-level job statements for release planning
- **User Story Backlog**: implementation-ready stories grouped by parent job
- Each user story references its parent job for traceability

---

## 4. Adoptable Patterns for Triumvirate

**Pattern 1: Agent = Job Owner.** Each agent in the fleet owns one customer job. The agent's capabilities are designed to complete that job, not to perform a technical function.

**Pattern 2: Orchestrator owns the meta-job.** The coordinator agent's job is "deliver a coherent outcome the user can act on." It doesn't do sub-jobs -- it ensures sub-jobs compose into a complete result.

**Pattern 3: Three-tier job definition per agent.** Functional (what), Emotional (feel), Social (perception). Forces you to design agents that aren't just technically correct but experientially right.

**Pattern 4: Job-first success metrics.** Measure job completion rate, effort score, emotional satisfaction, and recommendation likelihood -- not throughput or latency.

**Pattern 5: JTBD-Enhanced User Stories.** Bridge job statements to implementation via: Job Statement -> Job Story -> User Stories. Each story traces to a job. Orphan stories = scope creep.

---

## Sources

- [niksacdev/multi-agent-system/docs/jobs-to-be-done.md](https://github.com/niksacdev/multi-agent-system/blob/main/docs/jobs-to-be-done.md)
- [niksacdev/multi-agent-system/docs/agent-based-development.md](https://github.com/niksacdev/multi-agent-system/blob/main/docs/agent-based-development.md)
- [ModernAnalyst: JTBD-Enhanced User Story Framework](https://www.modernanalyst.com/Resources/Articles/tabid/115/ID/6974/Making-Agile-Work-in-Complex-Systems-The-JTBD-Enhanced-User-Story-Framework.aspx)
- [Gayatri Diwan: When AI Agents Meet JTBD (Medium)](https://medium.com/@gdiwan_38713/when-ai-agents-meet-the-jobs-to-be-done-jtbd-world-5eeb73bb82e5)
