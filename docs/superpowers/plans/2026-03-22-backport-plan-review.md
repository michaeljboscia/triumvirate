# Plan Review: 2026-03-22 Triumvirate Full Backport

**Reviewer:** Claude Opus 4.6
**Date:** 2026-03-22
**Verdict:** Issues Found -- 4 Critical, 3 Important, 4 Suggestions

---

## Critical Issues (Must Fix)

### C1: Task 13 -- Stenographer source file paths are wrong (filenames use hyphens, not underscores)

**Plan says (line 701):**
```
Source: /Users/mikeboscia/.triumvirate/stenographer/
```
Then references `session_save_ctl.py` and `session_save_worker.py` (underscores).

**Actual filenames on disk:**
```
/Users/mikeboscia/.triumvirate/stenographer/session-save-ctl.py
/Users/mikeboscia/.triumvirate/stenographer/session-save-worker.py
```

**Also:** The plan creates destination files with underscores (`session_save_ctl.py`, `session_save_worker.py`). The cp commands on lines 723-726 will fail because the source filenames don't exist.

**Fix:** Change all references from `session_save_ctl.py` / `session_save_worker.py` to `session-save-ctl.py` / `session-save-worker.py`, both in the cp commands and the destination paths.

---

### C2: Task 7 Step 4 -- Committing dist/ will fail because it is gitignored

**Plan says (line 382-388):** `git add mcp-server/dist/` then commit.

**Actual .gitignore contains:** `mcp-server/dist/`

`git add mcp-server/dist/` will silently add nothing (or error) because the directory is gitignored. The entire Task 7 Step 4 commit is a no-op or failure.

**Fix:** Either (a) remove the commit step entirely -- the build verification in Steps 1-3 is sufficient, or (b) if you intentionally want dist/ checked in for users who don't want to build, remove `mcp-server/dist/` from `.gitignore` first and document that decision. Option (a) is the standard open-source approach.

---

### C3: Task 5 -- Refactoring tools.ts is significantly more complex than described

**Plan says (line 261):** "remove inlined implementations, import from `./runtime.js`"

**Reality:** The repo tools.ts (1,161 lines) defines `GeminiSession` interface (line 168), `GEMINI_CLI` constant (line 57), `executeWithFallback()` (line 74, ~50 lines), and `spawnWithFallback()` (line 126, ~40 lines) inline. The production tools.ts (1,110 lines) imports all of these from `runtime.js`.

The refactor is not just "remove inlined implementations and add an import." It also requires:
1. Removing the `GeminiSession` interface definition (line 168 in repo)
2. Removing the `GEMINI_CLI` constant (line 57 in repo)
3. Changing the `cli-executor.js` import -- production only imports `formatError` and `OnProgress`, not `executeCli` or `spawnCliAsync` (those are used by runtime.ts now)
4. Removing local `mkdirSync`, `rmSync`, `tmpdir` imports that are no longer needed
5. Adding the `GEMINI_CLI` and `GeminiSession` imports from `./runtime.js`

**Also:** The production tools.ts line 59 has a hardcoded personal path:
```
"/Users/mikeboscia/.claude/SESSION_LOG_SPEC.md"
```
This path would need scrubbing if tools.ts is being diffed/aligned with production. The plan does not mention scrubbing tools.ts.

**Fix:** Add explicit sub-steps for each removal. Add a scrub step for any paths that might leak from the production diff. Consider providing the full target import block so the executor doesn't have to reverse-engineer the delta.

---

### C4: Task 12 -- Missing `oracle_decommission_cancel` from permissions template

**Plan says (line 673):** The `spawn_oracle` tool is in the "allow" list.
**Plan says (line 676-678):** Only `oracle_decommission_request` and `oracle_decommission_execute` are in the "ask" list.

**Missing:** `oracle_decommission_cancel` is a valid oracle tool (confirmed at line 4867 of oracle-tools.ts and in the commit message on line 206) but is not listed in either "allow" or "ask" in the permissions template.

**Fix:** Add `"mcp__inter-agent-gemini__oracle_decommission_cancel"` to either the "allow" or "ask" list. Since canceling a decommission is non-destructive (it prevents destruction), it belongs in "allow."

---

## Important Issues (Should Fix)

### I1: Task 4 -- Line number references for server.ts insertion are stale

**Plan says (line 215-229):**
- "Add after line 13 (`import { registerGeminiTools } from "./tools.js";`)"
- "Add after line 20 (`registerGeminiTools(server);`)"

**Actual server.ts in repo:**
- Line 13: `import { registerGeminiTools } from "./tools.js";` -- CORRECT
- Line 20: `registerGeminiTools(server);` -- CORRECT

These happen to be correct today. But the plan format "line 13" / "line 20" is fragile. If Task 1-3 somehow modified server.ts (they don't currently), these would drift.

**Verdict:** Currently correct but fragile. Consider using anchor text ("after the line containing `registerGeminiTools(server);`") instead of line numbers.

---

### I2: Task 14 -- Install script references `docs/setup/oracle.md` which is not created by any task

**Plan says (line 804):**
```bash
warn "To enable later, see docs/setup/oracle.md"
```

No task in the plan creates `docs/setup/oracle.md`. The file does not exist in the repo. This means the install script will reference a nonexistent document.

**Fix:** Either (a) add a task to create `docs/setup/oracle.md`, or (b) change the reference to point to an existing location (e.g., the oracle section in ARCHITECTURE.md created by Task 15, or the README section from Task 16).

---

### I3: Task 10 Step 4 -- crystallize skill subdirectories need explicit copy commands

**Plan says (line 571-578):** "The `crystallize` skill has `factory/`, `reference/`, `enforcement.md`, `validation.md` subdirectories. Copy if they exist."

**Actual contents of production crystallize directory:**
```
enforcement.md
factory/
reference/
SKILL.md
validation.md
```

The Step 2 `cp` command only copies `SKILL.md`. The plan says "Copy any subdirectories/files" but provides no commands. An executor following the plan literally would only copy SKILL.md and skip enforcement.md, validation.md, factory/, and reference/.

**Fix:** Change the Step 2 `cp` command for crystallize to `cp -r` on the entire directory, or add explicit cp commands for the subdirectories and extra files. The simplest fix is changing the loop:
```bash
cp -r "/Users/mikeboscia/.claude/skills/$skill/" \
   "/Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/$skill/"
```
But then you need to handle the other 7 skills potentially having extra files too (verify they don't, or use cp -r universally).

---

## Suggestions (Nice to Have)

### S1: Task 1 Step 3 -- `npx tsc --noEmit src/oracle-types.ts` may fail as a standalone check

Running `tsc --noEmit` on a single file does not respect tsconfig.json path mappings by default. It works for a truly zero-dep types file, but it's more reliable to run `npx tsc --noEmit` (project-wide) even at this early stage.

---

### S2: Tasks 1-6 commit messages include exact line counts

Commit messages like "338 lines" and "4946 lines" and "632 lines" will be wrong if scrubbing adds or removes lines. The executor would need to verify these counts after scrubbing and adjust the commit messages.

**Suggestion:** Remove exact line counts from commit messages, or mark them as approximate with "~" prefix.

---

### S3: Missing `docs/setup/oracle.md` from the New Files manifest

The File Structure section (lines 19-53) lists all new and modified files. `docs/setup/oracle.md` is referenced by the install script but absent from the manifest. If a task is added to create it, it should also appear in the manifest.

---

### S4: Task 13 -- Existing stenographer files in two locations

The repo has stenographer files in both:
- `/Users/mikeboscia/projects/triumvirate/stenographer/` (the runtime copy)
- `/Users/mikeboscia/projects/triumvirate/starter-kit/stenographer/` (the starter-kit template)

The plan copies session-save-ctl.py and session-save-worker.py only to `starter-kit/stenographer/`. Should they also be added to the root `stenographer/` directory for consistency with the existing files there?

---

## Verification Summary

| Check | Result |
|-------|--------|
| All source files exist | PASS (except stenographer filename mismatch) |
| Task dependencies ordered correctly | PASS |
| No circular dependency risk in imports | PASS -- oracle-types (pure types) -> runtime (singleton) -> oracle-tools (MCP tools) is a clean DAG |
| TypeScript build order | PASS -- types (T1) -> runtime (T2) -> tools (T3) -> server wiring (T4) -> refactor (T5) -> hardening (T6) -> build (T7) |
| Scrubbing steps present | PASS for Tasks 1-3, 8, 10, 13. MISSING for Task 5 (tools.ts refactor has a personal path in production) |
| Production server.ts already has oracle wired | PASS -- confirms Task 4 approach is correct |
| Shared directory parity | PASS -- production and repo have identical shared/ files |
| Gemini directory delta | PASS -- only missing file is runtime.ts (added by Task 2) |
| Hook source files exist | PASS -- all 4 hooks exist at the expected paths |
| Skill source files exist | PASS -- all 8 SKILL.md files exist at expected paths |
| dist/ is gitignored | FAIL -- Task 7 Step 4 will not work |
