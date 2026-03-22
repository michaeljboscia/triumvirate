---
description: Start Manus-style file-based planning for complex multi-phase tasks
---

Start file-based planning for a complex task. Creates `task_plan.md`, `findings.md`, and `progress.md` in the scratchpad.

**First**, read `~/.claude/skills/task-planning.md` — this contains the full methodology and templates.

**Then**, parse the user's input. Expected formats:
```
/plan                           # Start planning, ask what the task is
/plan <task description>        # Start planning for the specified task
```

**Execute the planning workflow:**

1. **Create planning files** in the session scratchpad directory
2. **Guide through Phase 1** (Requirements & Discovery)
3. **Re-read task_plan.md before major decisions** (attention anchoring)
4. **Update status after each phase completes**
5. **Log all errors encountered** (prevents repeating failures)
6. **Don't stop until all phases are complete**

**Integration with memory system:**
- After task completion, significant learnings should be stored via claude-mem
- The planning files are working memory; claude-mem is long-term memory
