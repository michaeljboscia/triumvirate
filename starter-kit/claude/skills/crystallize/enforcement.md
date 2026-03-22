# Crystallizer Pipeline — Enforcement

## The 6 Phases

| Phase | File | Triumvirate? | Purpose |
|-------|------|-------------|---------|
| 1. Capture | `factory/phase-1-capture.md` | No (interview) | Gather failure data, search artifacts, confirm boundary |
| 2. Diagnose | `factory/phase-2-diagnose.md` | **YES** | Root cause at mental model level |
| 3. Graduate | `factory/phase-3-graduate.md` | **YES** | Determine output level (skill vs matrix) |
| 4. Extract + Map | `factory/phase-4-extract-map.md` | No (construction) | Pull rules from proven system, map failures to anti-rationalization |
| 5. Structure | `factory/phase-5-structure.md` | No (construction) | Build the skill or matrix files |
| 6. Validate | `factory/phase-6-validate.md` | **YES** | Pressure test + triumvirate validation |

## Graduation Logic

The path only goes UP.

| Signal | Graduate To |
|---|---|
| Iron law + lesson exist for same failure class | Skill (automatic candidate) |
| Technique with steps Claude skips under pressure | Skill (with anti-rationalization) |
| Domain with context-dependent rules (needs routing) | Skill matrix |

**The gate test:** Uniform rules → skill. Context-dependent rules → matrix.

## Three Intake Shapes

Phase 1 adapts based on the failure pattern:

| Pattern | Timescale | Phase 1 Approach |
|---|---|---|
| **Acute** | Hours | Read session log, gather artifacts from one timeframe |
| **Chronic** | Weeks/months | Gemini daemon (2M context) aggregates across sessions |
| **Prophylactic** | Before failure | Build from research, expect iterative refinement |

## Anti-Rationalization Structure

Anti-rationalization is embedded IN enforcement rules, interleaved per-rule:

```
Rule: [the constraint]
  You will be tempted to: [exact rationalization]
  Why that fails: [counter with evidence]
```

NEVER separate anti-rationalization from rules. Claude skips separate sections.

## Iterative Refinement

Skills and matrices are living artifacts. When a new failure surfaces:
1. Add the failure to reference/ (new evidence)
2. Add the rationalization to the enforcement rule it violated
3. Re-run pressure test for the affected rule
