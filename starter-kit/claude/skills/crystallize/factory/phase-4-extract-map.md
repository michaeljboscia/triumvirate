# Phase 4: Extract Rules + Map Failure Modes

## Purpose
Pull rules from the proven system AND map failures to anti-rationalization entries. These two activities are ITERATIVE — they feed each other.

## Step 1: Identify the Proven System
What already works? This can be:
- Production code (e.g., v70 email-generator.ts)
- Battle-tested rules (e.g., EMAIL_WRITING_RULES.md)
- Curated research (e.g., MERCHANT-MENTAL-MODEL-RESEARCH.md)
- Hard-won experience codified in iron laws and lessons
- For prophylactic: external research and documentation

If no proven system exists and no research has been done → stop. Produce a skill or iron law and iterate. You're not ready for a matrix.

## Step 2+3 (Iterative): Extract ↔ Map

### Extract Rules
- Read the proven system completely
- Pull every constraint, rule, validation criterion
- Number each rule sequentially
- Each rule must be independently testable (can be expressed as a binary pass/fail check)

### Map Failure Modes
- For each failure in the Phase 1 failure log:
  - Which rule does this failure violate?
  - If no existing rule covers it → the proven system had an implicit rule. Add it.
  - What rationalization would Claude use to skip this rule? (often visible in the failure data itself)
  - Write the anti-rationalization entry interleaved with the rule:

```
Rule N: [constraint]
  You will be tempted to: [rationalization from failure data]
  Why that fails: [evidence from reference — quantified cost if possible]
```

### Iterate
- New rules from failure mapping → re-examine failure data for additional violations
- Continue until no new rules or rationalizations emerge

## Step 4: Research "The Right Way" — MANDATORY

**Skills that only say "don't do X" are finger-wagging, not enforcement.** Every rule must have three sections:
1. The constraint (what not to do)
2. The anti-rationalization (why you'll try to skip it)
3. **The right way** (how to actually do it correctly)

### Process
- For each tier/context in the matrix, run 2-3 Gemini MCP searches (`mcp__gemini__gemini-search`) for current best practices in that domain
- Search for: "best practices [domain] 2025 2026", "[specific tool/framework] production deployment", "[common task] correct approach"
- Distill the research into concrete, actionable "right way" playbooks per rule
- Include: specific commands, config snippets, decision trees, and recommended tools
- Write research findings to `reference/` directory before incorporating into rules

### Why This Step Exists
On 2026-03-20, the first version of `mx-gcp-operations` was all anti-patterns and anti-rationalization. The user pointed out: "not all finger wagging." Adding "right way" playbooks via Gemini research transformed the matrix from a list of warnings into an actionable operations manual.

### For Prophylactic (No Failure Data)
- Extract rules from research
- Leave anti-rationalization entries sparse: "No observed rationalizations yet — expect refinement"
- The reference layer is built from the research itself
- The "right way" section is the PRIMARY content for prophylactic skills

## Output
- Numbered rules with interleaved anti-rationalization AND "right way" playbooks
- Research findings written to `reference/` directory
- Identified gaps (rules that need more evidence)
- Pass to Phase 5
