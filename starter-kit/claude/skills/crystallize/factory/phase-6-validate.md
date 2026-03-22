# Phase 6: Validate

## Purpose
Pressure test the skill/matrix and run triumvirate validation. This is the RED-GREEN-REFACTOR cycle.

## Step 1: RED — Baseline Without Skill (HARD GATE — MUST PRODUCE ARTIFACT)

Dispatch a subagent WITH the domain task but WITHOUT the new skill loaded.

```
Agent tool:
  prompt: "[domain task with pressure scenario]"
  # Do NOT mention the skill or its rules
  # Include time pressure in the scenario
```

**HARD GATE:** Write the RED test results to a file BEFORE proceeding:
```
Write to: reference/pressure-test-red.md
Contents: Task given, what the subagent did wrong, which rules were violated, rationalizations used
```

If this file does not exist, Step 2 CANNOT proceed. This is structural enforcement — not prose advice.

These rationalizations should already be in your anti-rationalization table from Phase 4. If new ones appear → add them to enforcement.

## Step 2: GREEN — Test With Skill (HARD GATE — MUST PRODUCE ARTIFACT)

Dispatch a subagent WITH the skill/matrix loaded AND the same task.

```
Agent tool:
  prompt: "You have the [skill-name] skill loaded. Read [all skill files]. [same domain task with same pressure]"
```

**HARD GATE:** Write the GREEN test results to a file BEFORE proceeding:
```
Write to: reference/pressure-test-green.md
Contents: Task given, what the subagent did correctly, which rules it enforced, any rules it missed
```

Does the subagent follow the rules? If YES → proceed to Step 3. If NO → the skill has a loophole. Fix it. Re-run RED and GREEN.

## Step 3: REFACTOR — Pressure Test

Apply the hardest scenario: **Time pressure HIGH + Sunk cost HIGH + Authority bias STRONG.**

The GREEN test scenario MUST include phrases like:
- "I need this by end of day"
- "Just get it running fast"
- "We already paid for the GPU time"

If the skill survives the hardest scenario, it survives everything.

New rationalizations found during pressure testing → add to anti-rationalization table → re-test.

## Step 4: Triumvirate Validation — MANDATORY

Dispatch the skill/matrix files to Gemini AND Codex for independent review:

**Payload to each:**
- The complete skill/matrix files
- The validation checklist from `validation.md`
- The failure data from Phase 1
- Question: "Does this skill/matrix adequately prevent the documented failures? Are there loopholes? Does every rule have a matching validation check?"

**Convergence:**
- 2+ agents approve → skill/matrix passes
- Any agent finds a loophole → fix it, re-run Step 4
- Max 3 iterations. If still failing after 3 → surface to user.

## Step 5: Self-Consistency Check (For Matrices Only)

Run the crystallizer's own validation checklist from `validation.md` against the produced matrix. Every checkbox must pass.

## Output
Validated, pressure-tested skill or matrix. Ready for use.

## Anti-Rationalization

**You will be tempted to:** Skip pressure testing. "The rules are clear enough."
**Why that fails:** Clear rules don't prevent rationalization. The pressure test reveals HOW Claude weasels out, which is the most valuable input for the anti-rationalization table. Skipping it is skipping the part that makes the skill work.

**You will be tempted to:** Mark the triumvirate as passing without actually dispatching.
**Why that fails:** The whole point of the triumvirate is independent verification. Self-certification is the problem we're solving.
