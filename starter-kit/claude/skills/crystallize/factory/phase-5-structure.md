# Phase 5: Structure

## Purpose
Build the skill or matrix files from the extracted rules and mapped failures.

## For Skills (Uniform Rules, No Routing)

### File Structure
```
~/.claude/skills/
  skill-name/
    SKILL.md         # Description + rules + anti-rationalization (all inline)
```

### SKILL.md Template
```markdown
---
name: skill-name-here
description: Use when [specific triggering conditions — trigger-only, NO workflow summary]
---

# Skill Name

## Overview
[Core principle in 1-2 sentences]

## Rules

### Rule 1: [Name]
[Constraint]

**You will be tempted to:** [rationalization]
**Why that fails:** [evidence from reference]

### Rule 2: [Name]
[Continue pattern...]

## Validation Checklist
Run BEFORE presenting output:
- [ ] [Binary check for Rule 1]
- [ ] [Binary check for Rule 2]
- [ ] [Continue...]

## Reference
[Failure context: what happened, hours/resources wasted, why the rules exist]
```

### CSO Rules
- Description starts with "Use when..."
- Description contains ONLY triggering conditions
- NEVER summarize the workflow in the description
- Include keywords Claude would search for (error messages, symptoms, tools)

## For Matrices (Context-Dependent Rules, Needs Routing)

Read `factory/matrix-architecture.md` for the full matrix template.

### File Structure
```
~/.claude/skills/
  mx-matrix-name/
    SKILL.md              # Gate: routing + mode detection (~80 lines)
    enforcement.md        # Rules overview + context routing
    validation.md         # Binary checklists
    reference/            # Evidence, research, failure context
```

## Archival

After building the skill or matrix, check for competing skills:

1. Does any existing skill cover the same domain?
2. Is the new skill/matrix a 100% superset of that skill's domain?
   - YES → archive it
   - NO → leave it active (the new artifact only covers a subset)

### Same-root archival (skills in ~/.claude/skills/)
```bash
mv ~/.claude/skills/<old-skill> ~/.claude/skills/archived/<old-skill>
```

### Cross-root archival (skills in ~/.agents/skills/)
```bash
mv ~/.agents/skills/<old-skill>/SKILL.md ~/.agents/skills/<old-skill>/SKILL.md.archived
```

### Update archived index
Add entry to `~/.claude/skills/archived/index.md`:
```
| <skill-name> | <date> | <matrix-name> | <reason> |
```

## Output
Complete skill or matrix files on disk. Pass to Phase 6.
