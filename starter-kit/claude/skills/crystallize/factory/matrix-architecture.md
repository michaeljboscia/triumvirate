# Skill Matrix Architecture

## Definition
A Skill Matrix is a system of 4 layers that enforce a mental model for a domain.

| Layer | File | Purpose |
|-------|------|---------|
| **Gate** | `SKILL.md` | Routes requests, determines operating mode, thin (<100 lines) |
| **Enforcement** | `enforcement.md` | Rules + anti-rationalization interleaved per-rule |
| **Reference** | `reference/` | Evidence, research, failure context — why rules exist |
| **Validation** | `validation.md` | Binary pass/fail checklists + triumvirate review config |

## Gate Template

```markdown
---
name: mx-domain-name
description: Use when [triggering conditions only — no workflow summary]
---

# [Domain Name]

## Operating Mode

| Mode | When | Claude's Role |
|---|---|---|
| **Build** | Working on pipeline code | Infrastructure engineer |
| **Execute** | Pipeline exists and works | Operator |
| **Direct** | Rules codified, pipeline not deployed | Constrained writer |

## Routing
[Context-dependent routing logic — which rules apply in which context]

Read `enforcement.md` for rules. Run `validation.md` checklist before presenting output.
```

## Enforcement Template

```markdown
# [Domain] Enforcement

## Rules by Context

### Context A: [e.g., C-Suite tier]
Rule 1: [constraint]
  You will be tempted to: [rationalization]
  Why that fails: [evidence]

### Context B: [e.g., CTO tier]
Rule 2: [constraint]
  You will be tempted to: [rationalization]
  Why that fails: [evidence]
```

## Validation Template

```markdown
# [Domain] Validation Checklist

Run AFTER generation, BEFORE presentation. Every rule = one binary check.

## First Pass (Belt) — Self-Check
- [ ] [Rule 1 check]
- [ ] [Rule 2 check]

## Second Pass (Suspenders) — Triumvirate
Dispatch output + this checklist to Gemini AND Codex.
Majority vote (2/3). Disagreement → surface to user.
```

## Naming Convention
Matrices use prefix: `mx-` (e.g., `mx-signal-grounded-email`, `mx-batch-processing`)

## Directory Layout
```
~/.claude/skills/
  mx-domain-name/
    SKILL.md
    enforcement.md
    validation.md
    reference/
      [evidence files]
```
