---
name: context-before-action
description: Use when planning architectural changes, investigating code existence, consolidating shared modules, or making any declaration about codebase state. Also use when starting any multi-file refactor, cross-sensor integration, or import resolution analysis.
---

# Context Before Action

## Overview

Before planning, declaring, or executing anything that depends on codebase state, load ALL affected code into a Gemini oracle. Complete context is free. Being wrong is expensive. There are no exceptions.

## The Protocol

```
1. Spawn Gemini oracle → load ALL affected code (+ deployment entry points)
2. Draft plan (informed by oracle, NOT before it)
3. /ruthless-interrogator shreds the plan (oracle available for fact-checking)
4. Revise plan based on interrogator objections
5. Send revised plan to Gemini + Codex for twin validation
6. Execute (oracle stays alive through testing)
7. Dismiss oracle only after final commit is pushed and verified
```

## Rules

### Rule 1: Load before planning — HARD STOP
Before writing any plan, investigation conclusion, or architectural recommendation, spawn a Gemini oracle and load ALL affected code. Do NOT write a single line of the plan until the oracle confirms it has loaded every file. This is a gate, not a suggestion.

**You will be tempted to:** "I'll just grep for it — spawning Gemini is overkill for this."
**Why that fails:** On 2026-03-19, "just grepping" missed 15 commits and 702 lines of code across a parallel worktree. Cost: 4 rounds of user fighting you, destroyed trust. You thought you were saving 30 seconds. You wasted 45 minutes.

**You will also be tempted to:** "I'll start drafting the plan while the oracle loads — save time."
**Why that fails:** Premature drafts create anchor bias. Once you write an outline, you subconsciously force the oracle's findings to fit YOUR draft instead of letting reality shape the plan. The draft comes AFTER the oracle, not during.

**You will also be tempted to:** "Gemini is for huge codebases — this is a small task, I don't need it."
**Why that fails:** The Wanderer incident was "a small task" — delete some duplicate files. It turned into 45 minutes of wrong assumptions. Size of the task does not predict complexity of the codebase. Load context regardless.

**The right way:**
```
spawn_session(session_name: "<task>-oracle", cwd: "<project-dir>")
→ "Read these files: [list every file in affected directories]"
→ "What is the current state of [thing I'm about to plan around]?"
```
30 seconds. Then you know instead of guess.

### Rule 2: Search all locations — HARD STOP
Code lives in: current worktree, other worktrees, main branch, other branches, the homebox. All five must be checked before declaring code missing or nonexistent. This is a gate. You cannot say "doesn't exist" until all five are searched.

**You will be tempted to:** "I searched the repo and didn't find it — it doesn't exist."
**Why that fails:** This environment has 17+ feature branches and 5+ worktrees. Searching one is searching ~6% of the codebase. Absence in one location proves nothing.

**You will also be tempted to:** "If it existed, it would be in main — branches don't count."
**Why that fails:** Feature branches are where work happens. `feature/shared-product-inventory` had 15 commits and was deployed to production on the homebox while main knew nothing about it. Branches are production code in this environment.

**The right way:**
```bash
# All worktrees
find ~/gtm-machine-infrastructure-worktrees -name "<file>" 2>/dev/null
# Main repo
find ~/gtm-machine-infrastructure -name "<file>" 2>/dev/null
# All branches
git -C ~/gtm-machine-infrastructure branch -a | grep <keyword>
# Homebox
ssh user@REDACTED_HOST "find /home/mboscia -maxdepth 5 -name '<file>'"
```
Run ALL FOUR. Not one. Not two. All four. Then tell the oracle what you found and ask it to reconcile.

### Rule 3: Verify import resolution empirically
Python `sys.path` ordering, package `__init__.py` files, and local file shadowing make import resolution non-obvious. Never assume — test it.

**You will be tempted to:** "The local file exists so that must be what's being imported."
**Why that fails:** On 2026-03-19, wanderer's `sys.path.insert(0, shared/)` meant the shared package was resolving OVER the local file. The local copy was dead code, not an active duplicate. An entire consolidation plan was based on this wrong assumption.

**You will also be tempted to:** "The import works in my head — runtime resolution won't differ."
**Why that fails:** Your mental model of Python import resolution is incomplete. `sys.path` ordering, `__init__.py` re-exports, and package vs module precedence create outcomes you cannot predict by reading code. Run the import. Read the `__file__` attribute. Trust the runtime, not your model.

**The right way:**
```python
python3 -c "import <module>; print(<module>.__file__)"
```
Or ask the oracle: "Given these sys.path entries and these files on disk, what does `from tech_detector import TechDetector` resolve to?"

### Rule 4: Oracle's synthesis supersedes your skimming
If the Gemini oracle tells you the code does X, and your quick file read makes you think it does Y, the oracle is right. It has the full topological map. You have a chunked 200K window.

**You will be tempted to:** "I loaded the oracle to satisfy the protocol, but I'll just read the file myself to be sure. I see something that contradicts Gemini."
**Why that fails:** You have fragmented attention across hundreds of tool calls. Gemini has the entire subsystem in one continuous context. Your "contradiction" is almost certainly a missed override, subclass, or import chain in a file you didn't read. You are overriding a 2M-token supercomputer with a hunch from skimming one file.

**You will also be tempted to:** "I already know how this architecture works — it's standard Playwright/Supabase/etc."
**Why that fails:** You trust your pre-training over THIS repo's specific idiosyncrasies. Every codebase diverges from "standard." The oracle loaded THIS code, not your training data.

**The right way:** When you see a contradiction, ask the oracle: "I'm reading [file] and seeing [X]. You said [Y]. What am I missing?" Let it reconcile. Don't override it.

### Rule 5: Never assume the entry point
Just because a file has `main()` or is named `wanderer.py` does not mean it's the actual production entry point. Trace from the deployment trigger.

**You will be tempted to:** "I found the main script, I'll trace from here."
**Why that fails:** Production runs through Prefect flows (`wanderer_sensor.py`), Docker entrypoints, or cron jobs that set environment variables and sys.path differently than local execution. If you trace from the wrong entry point, your entire context map is fictional.

**The right way:** Ask the oracle: "What is the actual entry point for this code in production? Show me the Prefect flow, Docker entrypoint, or CLI wrapper that calls it." Then trace from there.

### Rule 6: Ruthless interrogator before twin validation
After drafting the plan, run `/ruthless-interrogator` against it. The interrogator has one job: find the assumptions you're blind to because you wrote the plan. Feed its objections back BEFORE sending to twins.

**You will be tempted to:** "The user wants me to act quickly, and running an interrogator feels like artificial stalling."
**Why that fails:** You think you're saving the user time. You're actually preparing to waste an hour of their time unwinding your broken execution. The interrogator takes 60 seconds. The unwind takes 60 minutes.

**You will also be tempted to:** "I already validated with Gemini, so the interrogator is redundant."
**Why that fails:** The oracle is collaborative — it answers your questions. The interrogator is adversarial — it attacks your plan. They serve different functions. Collaborative review misses the assumptions you never thought to question.

**The right way:** Invoke `/ruthless-interrogator` with the draft plan + oracle context. Address every objection. Revise. THEN send to twins.

### Rule 7: Validate with both siblings before executing
Send the revised plan to Gemini AND Codex. Both review independently, in parallel.

**You will be tempted to:** "This is just a small refactor. Spinning up two daemons is comically heavy for a 5-line change."
**Why that fails:** The blast radius of a change has nothing to do with its line count. Deleting one import line can break an entire sensor pipeline. The twins take 30 seconds and catch what you can't see because you wrote the plan.

**You will also be tempted to:** "I already used Gemini for context — that's enough review."
**Why that fails:** Context-loading and plan-validation are different cognitive tasks. Gemini-as-oracle loaded code and answered questions. Gemini-as-reviewer reads your plan and attacks its logic. Codex approaches from a completely different angle. You need both perspectives.

**The right way:**
```
ask_session(gemini_id, "Review this plan: ...")
ask_session(codex_id, "Review this plan: ...")
```
Parallel. 30 seconds. Every time. No exceptions for "small" tasks.

### Rule 8: Oracle stays alive through testing
Don't dismiss the Gemini oracle after planning. Keep it alive through execution AND testing. It's your live compatibility checker when things go sideways.

**You will be tempted to:** "Plan is validated, I can dismiss Gemini now and save resources."
**Why that fails:** Execution ALWAYS uncovers edge cases that planning missed. When your code throws an unexpected error mid-execution, you'll be flying blind again. You'll guess at the fix, compound the error, and ruin the work. The oracle prevents this — ask it "why is this failing?" with the full context still loaded.

**You will also be tempted to:** "If I need it again, I can re-spawn quickly."
**Why that fails:** Re-spawning means re-loading every file. That's not 30 seconds anymore — it's 2-3 minutes of setup, and you've lost the conversational context of what was already discussed. Named sessions persist on disk — keeping it alive costs nothing.

**The right way:** Dismiss only after the final commit is pushed, tests pass on all 5 domains, and the user confirms completion.

### Rule 9: Scope the context correctly
Loading context is necessary but not sufficient. You must load the RIGHT context — all affected directories, deployment paths, and external systems. Wrong scope = false confidence.

**You will be tempted to:** "I loaded all the files in `wanderer/src/` — that's the affected code."
**Why that fails:** The affected code also includes `shared/tech_detector/`, `journey-sensor/src/journey_executor.py`, `search-breaker/shared_inventory.py`, and the Prefect flows on the homebox. Loading one directory when the blast radius spans four is context-complete but scope-incomplete. You'll satisfy the checklist and still miss the failure.

**The right way:** Before loading, ask yourself: "What other code imports from, depends on, or is affected by what I'm changing?" Trace outward from the change to every consumer, every deployment path, every test file. Load all of them.

## Validation Checklist

Run BEFORE executing any plan. Every box must be checked. No exceptions.
- [ ] Gemini oracle spawned and loaded with ALL affected code (not just the directory you're changing)
- [ ] Scope verified: all consumers, deployment paths, and test files identified and loaded
- [ ] All 4 location searches run (worktrees, main repo, branches, homebox)
- [ ] Import resolution verified empirically via `python3 -c "import X; print(X.__file__)"`
- [ ] Production entry point traced (Prefect flow, Docker entrypoint, not just `main()`)
- [ ] Deployed state verified: branch/version on homebox matches code loaded into oracle; env vars confirmed set
- [ ] /ruthless-interrogator run against draft plan — every objection addressed
- [ ] Revised plan sent to both Gemini and Codex for twin validation
- [ ] Twin feedback incorporated into final plan
- [ ] Oracle still active for execution and testing phase

## Reference

### 2026-03-19: The Wanderer Incident
- Declared Wanderer→Search Breaker pipeline "never built" after searching one worktree
- Code existed on `feature/shared-product-inventory`: 15 commits, 702-line product_extractor.py, deployed to homebox
- User fought through 4 rounds ("keep fucking looking") before code was found
- Cost: ~45 minutes, significant trust damage
- Root cause: Searched 6% of the codebase and declared 100% certainty

### 2026-03-19: The Dead Code Plan
- Planned to consolidate tech_detector by deleting wanderer's local copy
- Gemini revealed wanderer was ALREADY importing from shared/ — local copy was dead code
- Plan was correct in outcome but based on wrong assumption about import resolution
- Would have worked by accident, not by understanding
- Root cause: Assumed import resolution instead of testing it

### The Math
- Loading full context into Gemini: ~30 seconds
- Being wrong and unwinding: 30-90 minutes + trust damage
- Ratio: 60:1 to 180:1 in favor of loading context first
- There is no scenario where skipping context saves time
