# Anthropic Skill Methodology — Reference

Extracted from `writing-skills` and `anthropic-best-practices.md` for crystallizer use.

## TDD for Skills (RED-GREEN-REFACTOR)

| Phase | What You Do |
|-------|------------|
| **RED** | Run pressure scenario WITHOUT skill. Document baseline failure + rationalizations. |
| **GREEN** | Write minimal skill addressing those specific rationalizations. Re-run. Agent complies. |
| **REFACTOR** | Find new rationalizations. Add counters. Re-test until bulletproof. |

## Claude Search Optimization (CSO)

**Description field rules:**
- Start with "Use when..."
- Third person
- ONLY triggering conditions — NEVER summarize workflow
- Under 500 characters preferred
- Include keywords: error messages, symptoms, tool names

**Why this matters:** If the description summarizes the workflow, Claude follows the description instead of reading the skill body. The skill becomes documentation Claude skips.

## Anti-Rationalization Patterns

### Interleaving (mandatory)
```
Rule: [constraint]
  You will be tempted to: [rationalization]
  Why that fails: [counter]
```

### Rationalization Table
| Excuse | Reality |
|--------|---------|
| "Too simple to need this" | Simple things break. The process is fast for simple cases. |
| "Emergency, no time" | Systematic approach is FASTER than guess-and-check. |

### Red Flags List
When you catch yourself thinking any of these, STOP:
- "Just this once..."
- "I'll do it properly next time..."
- "This case is different because..."

### Redundancy by Design
Core mandates appear 4+ times in different contexts. Agents rationalize past rules they only see once.

## Token Cost Justification

The triumvirate (all 3 agents validating) is expensive by design. Justification: the cost of NOT doing it (16 hours wasted CPU on Tellus, 6 weeks of bad cold email copy) dwarfs the token cost of 3 parallel dispatches. This is a design decision, not a bug.
