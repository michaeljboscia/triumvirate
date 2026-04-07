---
description: Convene a multi-model council (Claude + Gemini + Codex) for structured decision analysis
argument-hint: <question or decision to analyze>
---

# Triumvirate Council

You are orchestrating a structured multi-model decision analysis. Three architecturally different AI models — Claude (you), Gemini, and Codex — will independently analyze the same question. You will then synthesize their responses into a formal decision record.

**First**, read the skill definition and reference files:
- `~/.claude/skills/council/SKILL.md` — boundaries and when to use
- `~/.claude/skills/council/references/synthesis.md` — synthesis methodology
- `~/.claude/skills/council/references/output-format.md` — decision record template

**Then**, execute the workflow below.

---

## Step 1: Parse & Validate

The user's input is in `$ARGUMENTS`. If empty, ask what decision they need analyzed.

**Pre-flight check:** Does the question have enough structure?

A council question needs:
- A clear decision or trade-off (not debugging, not implementation)
- Enough context to reason about (project, constraints, stakes)

If too thin (e.g., "Should we use Redis?"), ask for:
- **Goal:** What are you trying to achieve?
- **Constraints:** Time, budget, tech stack, team size
- **Known facts:** What's already decided?
- **Key unknowns:** What are you uncertain about?

If the question is debugging or implementation, redirect:
> "This is better suited for direct work or `/plan`. Council is for strategic and architectural decisions."

## Step 2: Build Context Pack

Build a **Context Pack** — structured context that all three models receive. Do NOT summarize files yourself. Instead:

1. Identify 3-8 relevant file paths for this decision
2. Note current git branch and recent relevant commits
3. State the project's current architecture in 2-3 sentences (from CLAUDE.md, README, or session logs)
4. List hard constraints (deadlines, budget, tech stack locks)
5. Include user-stated known facts and unknowns

Format:
```
CONTEXT PACK
─────────────
Project: [name] at [full path]
Branch: [branch]
Architecture: [2-3 sentences]
Constraints: [list]
Relevant files (read these yourself for full context):
  - [path 1]
  - [path 2]
  - ...
Known facts: [from user input]
Key unknowns: [from user input]
```

## Step 3: Form Your Position (ISOLATED)

**Critical: You MUST write your analysis BEFORE reading sibling responses.**

Analyze the question. Your analytical lens:
- System-level architecture and second-order effects
- Developer experience and maintainability
- Domain-driven design implications

Write 300-600 words following this structure:
```
Assessment: [2-3 paragraphs]
Key Risks: [bulleted]
Assumptions: [bulleted]
Recommendation: [1-2 sentences]
Confidence: [High/Medium/Low] — [reason]
Least qualified to assess: [what]
```

Hold this in context. Do NOT revise after reading siblings' responses.

## Step 4: Dispatch to Siblings (Parallel)

Invoke the `inter-agent-protocol` skill if not already loaded. Spawn both daemons with `cwd` set to the project directory.

Send to both **in a single message** (parallel `ask_session` calls):

**To Gemini:**
```
Council question from Mike (via Claude). Give your independent analysis — 300-600 words, structured.

QUESTION: [user's question, verbatim]

[CONTEXT PACK from Step 2]

YOUR ANALYTICAL LENS: Focus on long-term system health, data flow integrity, external integration risks, operational costs at scale, and missing constraints nobody has named yet. You have full file access — read the relevant files yourself if you need deeper context.

FORMAT:
- Assessment (2-3 paragraphs)
- Key Risks (bulleted)
- Assumptions you're making (bulleted)
- Recommendation (1-2 sentences)
- Confidence: High/Medium/Low — reason
- What you're LEAST qualified to assess
```

**To Codex:**
```
Council question from Mike (via Claude). Give your independent analysis — 300-600 words, structured.

QUESTION: [user's question, verbatim]

[CONTEXT PACK from Step 2]

YOUR ANALYTICAL LENS: Focus on implementation feasibility, integration complexity, execution speed, type safety, code-level constraints, and real-world shipping risk. You have full file access — read the relevant files yourself if you need deeper context.

FORMAT:
- Assessment (2-3 paragraphs)
- Key Risks (bulleted)
- Assumptions you're making (bulleted)
- Recommendation (1-2 sentences)
- Confidence: High/Medium/Low — reason
- What you're LEAST qualified to assess
```

**Daemon failure:** If a sibling times out or errors, proceed with available responses. Note in the decision record: "[Model] was unavailable — partial council (2/3)."

## Step 5: Synthesize (Neutral Arbiter Mode)

**Role switch.** You are now the **Neutral Arbiter**, not a participant. Evaluate "Participant Claude's" analysis with the exact same scrutiny as Gemini's and Codex's. Do not unconsciously favor your own earlier position.

Read and follow:
@~/.claude/skills/council/references/synthesis.md

## Step 6: Write the Decision Record

Read and follow:
@~/.claude/skills/council/references/output-format.md

Write to:
```
~/projects/<current-project>/decisions/YYYY-MM-DD_<topic-slug>.md
```

Create `decisions/` if it doesn't exist.

Display the **Verdict** and **Key Tensions** in chat for immediate visibility. The full record is on disk.

## Step 7: Cleanup

Soft-dismiss both daemons (preserve session for follow-up questions).

If the user wants to challenge the verdict or ask follow-ups, use the existing daemons — don't re-run the full council.
