# Why Three Agents — A Case Study From Building This Repo

This document is not theoretical. Everything described here happened during the session that built and shipped this repository. A single Claude agent wrote the code. Then all three agents reviewed it. What they found — and what they would have missed alone — is the best argument for why this system exists.

---

## What a Single Agent Misses

Claude wrote 28 commits in one session: the oracle engine (5,900 lines of TypeScript), 4 hooks, 8 skills, 6 slash commands, an interactive installer, beginner mode, subscription-aware tuning, and comprehensive documentation. Productive? Absolutely. But Claude reviewing Claude's own work is like proofreading your own essay — you read what you meant to write, not what you actually wrote.

Here's what Claude shipped with full confidence before the twins looked at it:

**A remote code execution vulnerability.** Seven `execSync` calls in the oracle engine interpolated user-controlled MCP tool inputs directly into shell command strings. An oracle named `test$(rm -rf ~)` would have executed arbitrary shell commands. Claude wrote this code, copied it from production, scrubbed the paths, verified the TypeScript compiled, and committed it — without ever noticing the injection vector. Because Claude was focused on "does this build?" not "can this be exploited?"

**Documentation that contradicted itself.** The git workflow guide proudly stated: "No hook ever runs `git push`. Pushing is always manual and explicit." Meanwhile, the installer Claude had just written offered a beginner mode that sets `TRIUMVIRATE_AUTO_PUSH=1` — literally auto-pushing on every edit. Claude wrote both documents in the same session, hours apart, and never connected them.

**A feature that didn't work.** The Stenographer was marketed as a working feature across five documentation files. In reality, it had been broken for months — a 15-line missing function (`update_append_time`) that caused it to crash on every invocation. Claude had documented it as "experimental" in one session, then re-marketed it as working in the next, because no persistent memory recorded the bug. This was the 7th time the same issue was rediscovered.

**An installer that asked a question and threw away the answer.** The interactive installer asked users "Where do you keep your projects?", stored the answer in a variable, printed a confirmation message, and then... never saved it anywhere. The variable evaporated when the script finished. The next session start would look in the default location regardless of what the user said.

**Wrong defaults in reference docs.** The configuration reference claimed the Stenographer model default was `qwen2.5:32b` (19GB) while the installer guided users to download `qwen2.5:7b` (4.4GB). A user following the installer would have a model the backend didn't expect. The token gate threshold was documented as `50` when the actual code used `200`.

**A hardcoded macOS path.** The Gemini CLI path defaulted to `/opt/homebrew/bin/gemini` — a Homebrew-specific location that doesn't exist on Linux. Anyone installing on a Linux machine would get a "command not found" error with no useful message.

None of these are exotic bugs. They're the mundane, predictable failures of a single perspective working fast.

---

## What Gemini Found (and Why)

Gemini's strength is **reading comprehension at scale.** It loaded all 11 documentation files simultaneously — README, ARCHITECTURE, 6 guides, CLAUDE.md, settings.json, and the full install script — and cross-referenced them against each other.

A human reviewer reads documents sequentially. By the time they finish the git workflow guide, they've forgotten the exact wording of the installer's beginner mode prompt. Gemini held all 11 documents in its 2M-token context window at once and pattern-matched across them.

**What it caught:**

1. **The git push contradiction.** Gemini flagged the exact lines: `git-workflow.md:309` says "no hook ever runs git push" while `install.sh` writes `TRIUMVIRATE_AUTO_PUSH=1`. It identified this as a trust violation — documentation that promises safety guarantees the code doesn't honor.

2. **The installer's phantom project directory.** Gemini traced the `$PROJECTS_DIR` variable through the entire script and found it was set, used in a confirmation message, and never persisted. It identified this as a user experience failure — the installer gives the illusion of configuration.

3. **The Ollama model default mismatch.** Gemini compared the configuration reference defaults against the installer's interactive menu and found three documents claiming different defaults for the same variable.

4. **Personal paths in committed plan files.** Gemini scanned beyond the requested files and found the `docs/superpowers/plans/` directory contained hundreds of raw `/Users/mikeboscia/.claude/...` paths and explicit business name references. These were implementation plans that should never have been committed to a public repo.

**What Gemini is bad at:** It didn't look at the TypeScript code. It didn't run anything. It didn't think about security. Its review was entirely text-based — consistency, accuracy, completeness. If a doc said "12 hooks" and the settings.json configured 11, Gemini caught it. But it wouldn't have found the shell injection in a million years, because that requires understanding code execution semantics, not document cross-referencing.

---

## What Codex Found (and Why)

Codex's strength is **code-level reasoning.** It reads TypeScript, Bash, and Python the way Gemini reads English — looking for patterns that don't add up.

**What it caught:**

1. **The shell injection vulnerability (Critical).** Codex identified all 7 `execSync` calls in `oracle-tools.ts` where `params.name`, `params.reason`, `params.role`, and `entryLabel` — all user-controlled MCP tool inputs — were interpolated directly into shell command strings without sanitization. It flagged this as RCE (remote code execution) and recommended `execFile` with args arrays or input sanitization.

2. **Missing dot in `.pythia` registry path (High).** The idle sweep in `runtime.ts` looked for the oracle registry at `~/pythia/registry.json` instead of `~/.pythia/registry.json`. One missing character meant the entire idle sweep feature was silently disabled on every installation. Codex caught this by tracing the path constant through the codebase and comparing it against the canonical path in `oracle-tools.ts`.

3. **Unawaited async dismiss in idle sweep (High).** `dismissDaemon()` returns a Promise, but the idle sweep called it without `await` inside a try/catch. This meant errors were silently swallowed (the catch block never triggered for rejected promises), and the dismiss might not complete before the next loop iteration mutated the same state.

4. **Bash `local` outside a function (Medium).** The beginner mode auto-commit block used `local` variable declarations in a `case` branch — not inside a function. This is valid in some shells but prints warnings in others, and the stderr output could interfere with the hook's JSON protocol.

5. **Hardcoded macOS Homebrew path.** `GEMINI_CLI` defaulted to `/opt/homebrew/bin/gemini` instead of just `"gemini"` (which would use PATH resolution). This would break on every non-macOS-Homebrew system.

**What Codex is bad at:** It didn't read the documentation. It didn't check if the README accurately described the features. It didn't notice the git push contradiction or the wrong config defaults. Codex sees code, not prose. Ask it "is this function safe?" and it's excellent. Ask it "does the README match reality?" and it has nothing to say.

---

## Why the Combination Works

| Failure Class | Claude Alone | + Gemini | + Codex |
|--------------|-------------|----------|---------|
| Shell injection (RCE) | Missed | Missed | **Caught** |
| Doc contradictions | Missed | **Caught** | Missed |
| Wrong config defaults | Missed | **Caught** | Missed |
| Leaked personal paths | Missed | **Caught** | Missed |
| Missing `.pythia` dot | Missed | Missed | **Caught** |
| Unawaited async | Missed | Missed | **Caught** |
| Bash `local` bug | Missed | Missed | **Caught** |
| Hardcoded OS path | Missed | Missed | **Caught** |
| Installer phantom var | Missed | **Caught** | Missed |
| Broken Stenographer | Missed (7 times) | Missed | Missed* |

*The Stenographer bug was found by Claude itself during an honest "does this actually work?" investigation prompted by the user. Neither twin would have caught it because it required running the code end-to-end, not reading it.

**The pattern:**

- **Claude** is the builder. Fast, productive, gets 90% right. But it has the author's blind spot — it can't objectively evaluate its own output.
- **Gemini** is the document reviewer. It holds everything in context simultaneously and finds inconsistencies that sequential reading misses. It catches lies — places where documentation promises something the code doesn't deliver.
- **Codex** is the code auditor. It reasons about execution semantics, security boundaries, type safety, and platform portability. It catches bugs that look correct to a human reader but fail at runtime.

No single agent found more than 40% of the issues. Together, they found all of them.

---

## The Meta Point

This repository exists to help people coordinate three AI agents. The repository itself was built by one AI agent and reviewed by all three. The review process caught a security vulnerability, four documentation lies, two disabled features, and three code bugs.

If you're reading this and thinking "I'll just use Claude, it's good enough" — it is, 90% of the time. This document is about the other 10%. The 10% where your installer silently ignores user input, your docs promise safety guarantees that aren't real, and your MCP tools have an RCE vulnerability that would have been caught by any junior security engineer reading the code for the first time.

Three perspectives. Three skill sets. One codebase that's actually ready to ship.

That's the point.
