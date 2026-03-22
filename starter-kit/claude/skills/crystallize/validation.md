# Crystallizer Validation Checklists

## For Skills

Run this checklist before declaring a skill complete:

- [ ] **CSO compliant:** Description starts with "Use when..." and contains ONLY triggering conditions (no workflow summary)
- [ ] **Name valid:** Letters, numbers, hyphens only. Max 64 chars.
- [ ] **Anti-rationalization complete:** Every observed rationalization from failure data has a counter entry interleaved with the rule it violates
- [ ] **Reference populated:** At minimum, the structured failure context (what happened, cost, why rules exist)
- [ ] **RED phase done:** Baseline test without skill documented — agent failed as expected
- [ ] **GREEN phase done:** Same test with skill — agent complied
- [ ] **Pressure tested:** Scenario grid applied. At minimum S8 (all three HIGH)
- [ ] **Triumvirate validated:** Gemini + Codex independently approved the skill
- [ ] **Under 500 lines:** SKILL.md does not exceed 500 lines
- [ ] **Archival done:** Competing skills archived if matrix is 100% superset

## For Matrices

All skill checks above PLUS:

- [ ] **Gate routes correctly:** Tested with in-domain AND out-of-domain requests
- [ ] **Gate is thin:** Under 100 lines, routing + pointers only
- [ ] **Three modes work:** Build, Execute, and Direct modes tested
- [ ] **Rules individually testable:** Each rule can be checked as binary pass/fail
- [ ] **Anti-rationalization embedded:** Per-rule, not separated into its own section
- [ ] **Reference grounded:** Evidence/research linked to specific rules
- [ ] **Validation checklist complete:** Every enforcement rule has a matching binary check
- [ ] **Triumvirate second pass configured:** Instructions for dispatching output to siblings
- [ ] **System test passed:** All layers work together, not just individually
- [ ] **Files under 500 lines each:** Progressive disclosure for larger reference
- [ ] **Matrix prefix used:** Name starts with `mx-`
- [ ] **Archival done:** Competing skills archived, index.md updated

## Triumvirate Dispatch Template (for validation)

Send to BOTH Gemini and Codex:

"Review this [skill/matrix] against its validation checklist. The skill/matrix is at [path]. The checklist is at [path]. The original failure data is: [summary].

Questions:
1. Does this adequately prevent the documented failures?
2. Are there loopholes an agent could exploit?
3. Does every rule have a matching validation check?
4. Is anything missing?

Approve or flag issues."
