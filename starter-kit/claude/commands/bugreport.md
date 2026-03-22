---
description: Convert investigation findings into properly formatted GitHub issues
---

Convert investigation findings into properly formatted GitHub issues with clean Markdown structure.

**First**, read `~/.claude/skills/bugreport.md` for the complete workflow and issue template structure.

**Then**, follow these steps:

### Step 1: Gather Context
Ask the user to paste their investigation findings, queries, or context about the bug they discovered. This can include:
- Error messages
- SQL query results
- Investigation notes
- Sub-agent findings
- Data analysis

### Step 2: Extract Key Information
Parse the context to identify:
- **Problem:** What's broken (first paragraph usually)
- **Affected systems:** Domains, workflows, functions mentioned
- **Impact:** What this breaks (look for "missing", "failed", "doesn't work")
- **Root cause:** Why it's broken (look for "because", "due to", causation)
- **Investigation:** What needs checking (look for "check", "validate", "investigate")
- **Fix:** Proposed solutions (look for "fix", "solution", "should")
- **Workaround:** Temporary mitigation if mentioned

### Step 3: Ask Clarifying Questions
Use `AskUserQuestion` to get:

**Question 1: Issue Title**
- Header: "Title"
- Suggest a title based on the problem
- Options: 2-3 title variants + Other

**Question 2: Priority**
- Header: "Priority"
- Options:
  1. CRITICAL - Production down, data loss, blocking outbound
  2. HIGH - Major functionality broken, affects multiple domains
  3. MEDIUM - Feature degraded, workaround exists
  4. LOW - Minor issue, doesn't block work

**Question 3: Labels** (multiSelect: true)
- Header: "Labels"
- Suggest based on context:
  1. bug
  2. n8n
  3. supabase
  4. dataforseo
  5. orchestrator
  6. data-quality
  7. enhancement
  8. documentation

**Question 4: Repository**
- Header: "Repository"
- Options:
  1. gtm-machine-infrastructure (Recommended - for bugs/code)
  2. gtm-machine-docs (for documentation issues only)

### Step 4: Format the Issue Body
Structure the extracted information using the template from `~/.claude/skills/bugreport.md`:

```markdown
## Problem
[Extracted problem statement with key data points]

## Affected Domains/Systems
[List from context]

## Impact
[What this breaks]

Missing critical capabilities:
- ❌ [Capability 1]
- ❌ [Capability 2]

## Root Cause
[Why it's happening]

## Investigation Needed
[What to check]

## Proposed Fix
1. [Fix step 1]
2. [Fix step 2]

## Temporary Workaround
[Workaround if mentioned, otherwise: "None - requires fix"]

## Priority
**[PRIORITY_LEVEL]** - [Justification]
```

### Step 5: Create the Issue
Call `mcp__github__issue_write`:
- method: "create"
- owner: [from taxonomy.json or git remote]
- repo: [selected repository]
- title: [from user selection]
- body: [formatted issue body]
- labels: [selected labels]

### Step 6: Return Results
Show user:
```
✅ Bug report created!

Issue #[NUMBER]: [TITLE]
🔗 https://github.com/[owner]/[repo]/issues/[number]

Priority: [PRIORITY]
Labels: [LABELS]
```

**Important Guidelines:**
- ✅ Preserve specific data points (domain names, counts, percentages)
- ✅ Keep technical details (SQL queries, error messages)
- ✅ Use proper Markdown formatting (##, -, ✅/❌, code blocks)
- ✅ Suggest appropriate labels based on affected systems
- ❌ Don't truncate or summarize excessively
- ❌ Don't remove domain names or examples
- ❌ Don't skip sections - use "None" or "TBD" if missing
