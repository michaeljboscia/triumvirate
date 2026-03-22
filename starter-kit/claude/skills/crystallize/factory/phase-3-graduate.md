# Phase 3: Graduate

## Purpose
Determine the appropriate output level: skill or skill matrix. Use the triumvirate.

## Triumvirate Dispatch — MANDATORY

Send the diagnosis (Phase 2 output) + failure data to all three agents. Each answers independently:

1. Should the output be a **skill** (uniform rules, no routing needed)?
2. Or a **skill matrix** (context-dependent rules, needs a gate for routing)?
3. What's the reasoning?

### The Gate Test
The single question that separates skill from matrix:

**Do the rules vary by context within this domain?**
- If rules apply the same way regardless of context → **skill**
- If different contexts need different rules (personas, tiers, modes, signal types) → **matrix**

### Convergence
- 2+ agents agree → that's the graduation level
- All 3 disagree → present to user with reasoning from each
- User confirms or overrides

### Present to User
"Based on the diagnosis, [2 of 3 / all 3] agents recommend a **[skill/matrix]**:
- Claude: [reasoning]
- Gemini: [reasoning]
- Codex: [reasoning]

The key question: do the rules for this domain vary by context, or are they uniform?

Confirm [skill/matrix], or override?"

## Output
Confirmed graduation level (skill or matrix) + user approval. Pass to Phase 4.
