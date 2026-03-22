# Triumvirate Full Backport — Production Parity Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the open-source triumvirate repo to feature parity with the live operating environment so that cloning this repo gives a new user the complete multi-agent coordination system.

**Architecture:** Backport the Pythia oracle engine (~5,900 lines), 4 advanced hooks, 8 core Claude skills, production MCP server refinements (process group kill, runtime bridge), and config templates. All personal paths scrubbed. Oracle is opt-in (requires Gemini API key) but ships with the repo.

**Tech Stack:** TypeScript (MCP server), Bash (hooks), Python (stenographer), Markdown (skills/docs)

**Working Directory:** /Users/mikeboscia/projects/triumvirate
**Git Branch:** main
**Production Reference:** /Users/mikeboscia/.claude/mcp-servers/inter-agent

---

## File Structure

### New Files (Create)
```
mcp-server/src/oracle-tools.ts          — 17 MCP oracle tools (registry, state, manifest, corpus)
mcp-server/src/oracle-types.ts          — Types, constants, error codes, interfaces
mcp-server/src/gemini/runtime.ts        — Singleton daemon lifecycle, mutex, model fallback bridge
starter-kit/claude/hooks/session-start-v3.sh            — Orphan recovery (stale locks/saves)
starter-kit/claude/hooks/pre-tool-use-supabase-mcp-gate.sh — Supabase MCP SQL gate
starter-kit/claude/hooks/post-tool-use-oracle-pressure.sh  — Oracle context pressure monitor
starter-kit/claude/hooks/post-tool-use-mode-nudge.sh       — Execution mode suggestion
starter-kit/claude/skills/inter-agent-protocol/SKILL.md    — Core: inter-agent messaging protocol
starter-kit/claude/skills/context-before-action/SKILL.md   — Core: verify before declaring
starter-kit/claude/skills/documentation-standards/SKILL.md — Core: DNA-level docs enforcement
starter-kit/claude/skills/our-systematic-debugging/SKILL.md — Core: debugging methodology
starter-kit/claude/skills/persist-or-fail/SKILL.md         — Core: mandatory persistence
starter-kit/claude/skills/file-taxonomy/SKILL.md           — Core: file organization
starter-kit/claude/skills/crystallize/SKILL.md             — Core: failure crystallization
starter-kit/claude/skills/orchestrator-not-compute/SKILL.md — Core: orchestrate, don't compute
starter-kit/shared/mcp_config.json.example                 — MCP server registration template
starter-kit/claude/settings.local.json.example             — Oracle permissions template
starter-kit/claude/lessons/TEMPLATE.md                     — Lessons directory starter
starter-kit/stenographer/session-save-ctl.py               — Background save controller
starter-kit/stenographer/session-save-worker.py            — Background save worker
```

### Modified Files
```
mcp-server/src/gemini/server.ts:13-20   — Add oracle-tools import + registration
mcp-server/src/gemini/tools.ts           — Refactor to use runtime.ts bridge (not inline fallback)
mcp-server/src/shared/cli-executor.ts    — Add process group kill (detached spawn + group SIGKILL)
mcp-server/package.json:11-14           — Add async-mutex dependency
starter-kit/claude/settings.json:9-79   — Add 4 new hook registrations
starter-kit/install.sh                   — Add skills copy + oracle opt-in
ARCHITECTURE.md                          — Document oracle engine
README.md                               — Update feature list, add oracle section
```

---

## Phase 1: Oracle Engine Backport

### Task 1: Copy oracle-types.ts (pure types, zero deps)

**Files:**
- Create: `mcp-server/src/oracle-types.ts`
- Source: `/Users/mikeboscia/.claude/mcp-servers/inter-agent/src/oracle-types.ts`

- [ ] **Step 1: Copy source file**

```bash
cp /Users/mikeboscia/.claude/mcp-servers/inter-agent/src/oracle-types.ts \
   /Users/mikeboscia/projects/triumvirate/mcp-server/src/oracle-types.ts
```

- [ ] **Step 2: Scrub personal paths**

Search for any occurrence of `/Users/mikeboscia` or `mikeboscia` or `michaeljboscia` in the copied file. Replace with environment variable references or generic defaults.

```bash
grep -n 'mikeboscia\|michaeljboscia\|/Users/' /Users/mikeboscia/projects/triumvirate/mcp-server/src/oracle-types.ts
```
Expected: No matches (types file is unlikely to have paths, but verify).

- [ ] **Step 3: Verify TypeScript compiles**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server && npx tsc --noEmit src/oracle-types.ts
```
Expected: No errors (zero runtime deps, pure type definitions).

- [ ] **Step 4: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add mcp-server/src/oracle-types.ts
git commit -m "feat(oracle): add oracle type definitions and constants

338 lines: OracleState, OracleManifest, OracleRegistryEntry, OracleResult<T>,
25 error codes, context window constants. Zero runtime dependencies."
```

---

### Task 2: Add async-mutex dependency + copy runtime.ts

**Files:**
- Modify: `mcp-server/package.json:11-14`
- Create: `mcp-server/src/gemini/runtime.ts`
- Source: `/Users/mikeboscia/.claude/mcp-servers/inter-agent/src/gemini/runtime.ts`

- [ ] **Step 1: Add async-mutex to package.json**

In `mcp-server/package.json`, add `"async-mutex": "^0.5.0"` to dependencies:

```json
"dependencies": {
  "@modelcontextprotocol/sdk": "^1.12.1",
  "async-mutex": "^0.5.0",
  "zod": "^3.24.2"
}
```

- [ ] **Step 2: Install dependencies**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server && npm install
```
Expected: async-mutex added to node_modules, package-lock.json updated.

- [ ] **Step 3: Copy runtime.ts**

```bash
cp /Users/mikeboscia/.claude/mcp-servers/inter-agent/src/gemini/runtime.ts \
   /Users/mikeboscia/projects/triumvirate/mcp-server/src/gemini/runtime.ts
```

- [ ] **Step 4: Scrub personal paths**

```bash
grep -n 'mikeboscia\|michaeljboscia\|/Users/' /Users/mikeboscia/projects/triumvirate/mcp-server/src/gemini/runtime.ts
```
Replace any hardcoded paths with `process.env.*` or generic defaults. Key patterns:
- `GEMINI_CLI` constant → should be `process.env.GEMINI_CLI_PATH || "gemini"`
- Session directories → should use `os.homedir()` not literal path

- [ ] **Step 5: Verify build**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server && npx tsc --noEmit
```
Expected: No errors. runtime.ts imports from `shared/cli-executor`, `shared/types`, `gemini/model-fallback` — all already in repo.

- [ ] **Step 6: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add mcp-server/package.json mcp-server/package-lock.json mcp-server/src/gemini/runtime.ts
git commit -m "feat(oracle): add GeminiRuntime singleton + async-mutex

632 lines: daemon lifecycle manager, OracleRuntimeBridge interface,
executeWithFallback/spawnWithFallback model rotation, toolMutex for
serialized state mutations, idle sweeper, ppid watchdog."
```

---

### Task 3: Copy oracle-tools.ts (the big one)

**Files:**
- Create: `mcp-server/src/oracle-tools.ts`
- Source: `/Users/mikeboscia/.claude/mcp-servers/inter-agent/src/oracle-tools.ts`

- [ ] **Step 1: Copy source file**

```bash
cp /Users/mikeboscia/.claude/mcp-servers/inter-agent/src/oracle-tools.ts \
   /Users/mikeboscia/projects/triumvirate/mcp-server/src/oracle-tools.ts
```

- [ ] **Step 2: Scrub personal paths**

```bash
grep -n 'mikeboscia\|michaeljboscia\|/Users/' /Users/mikeboscia/projects/triumvirate/mcp-server/src/oracle-tools.ts
```
Replace ALL matches. Common patterns:
- Registry paths → `path.join(os.homedir(), ".triumvirate", "oracle-registry.json")`
- State file paths → similar `os.homedir()` pattern
- Session log paths → use `process.env.AI_MEMORY_DIR || path.join(os.homedir(), ".ai-memory")`
- Any username references → remove or genericize

- [ ] **Step 3: Verify build**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server && npx tsc --noEmit
```
Expected: No errors. oracle-tools.ts imports from oracle-types.ts, gemini/runtime.ts, gemini/model-fallback.ts — all now present.

- [ ] **Step 4: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add mcp-server/src/oracle-tools.ts
git commit -m "feat(oracle): add 17-tool oracle engine

4946 lines: oracle_init, spawn_oracle, oracle_health, oracle_refresh,
oracle_sync_corpus, oracle_pressure_check, oracle_log_learning,
oracle_checkpoint, oracle_salvage, oracle_reconstitute,
oracle_quality_report, oracle_add_to_corpus, oracle_update_entry,
oracle_decommission_{request,cancel,execute}. Full state machine with
registry, manifests, locking, corpus loading, and TOTP decommission gates."
```

---

### Task 4: Wire oracle into gemini/server.ts

**Files:**
- Modify: `mcp-server/src/gemini/server.ts`

- [ ] **Step 1: Add oracle import and registration**

Add after `import { registerGeminiTools } from "./tools.js";`:

```typescript
import { registerOracleTools } from "../oracle-tools.js";
```

Add after `registerGeminiTools(server);`:

```typescript
registerOracleTools(server);
```

- [ ] **Step 2: Verify build**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server && npx tsc --noEmit
```
Expected: No errors.

- [ ] **Step 3: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add mcp-server/src/gemini/server.ts
git commit -m "feat(oracle): register oracle tools in gemini MCP server"
```

---

### Task 5: Refactor gemini/tools.ts to use runtime.ts bridge

**Files:**
- Modify: `mcp-server/src/gemini/tools.ts`
- Reference: `/Users/mikeboscia/.claude/mcp-servers/inter-agent/src/gemini/tools.ts`

- [ ] **Step 1: Diff the two versions to identify all differences**

```bash
diff /Users/mikeboscia/.claude/mcp-servers/inter-agent/src/gemini/tools.ts \
     /Users/mikeboscia/projects/triumvirate/mcp-server/src/gemini/tools.ts | head -200
```

The repo version inlines several definitions that production delegates to `runtime.ts`. Identify and remove:
1. The `GeminiSession` interface definition (moved to runtime.ts)
2. The `GEMINI_CLI` constant (moved to runtime.ts)
3. The `executeWithFallback()` function (moved to runtime.ts)
4. The `spawnWithFallback()` function (moved to runtime.ts)
5. Any now-unused `node:fs`/`node:os` imports that were only needed by the above

- [ ] **Step 2: Update imports**

Add to the import section of `mcp-server/src/gemini/tools.ts`:

```typescript
import { getGeminiRuntime, executeWithFallback, spawnWithFallback, type GeminiSession } from "./runtime.js";
```

Remove the 4 locally-defined implementations listed in Step 1. Remove unused stdlib imports.

- [ ] **Step 3: Scrub personal paths**

```bash
grep -n 'mikeboscia\|michaeljboscia\|/Users/' /Users/mikeboscia/projects/triumvirate/mcp-server/src/gemini/tools.ts
```
Known issue: production has a hardcoded `SESSION_LOG_SPEC_PATH` pointing to `/Users/mikeboscia/.claude/SESSION_LOG_SPEC.md`. Replace with:
```typescript
const SESSION_LOG_SPEC_PATH = process.env.SESSION_LOG_SPEC_PATH || "";
```

- [ ] **Step 4: Verify build + tool count unchanged**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server && npx tsc --noEmit
```
Expected: No errors.

Also verify all 10 Gemini tools are still registered:
```bash
grep -c 'server.tool(' /Users/mikeboscia/projects/triumvirate/mcp-server/src/gemini/tools.ts
```
Expected: 10 (send_message, get_response, spawn_daemon, ask_daemon, dismiss_daemon, list_daemons, list_scratchpad, write_scratchpad, list_jobs, summarize_transcript).

- [ ] **Step 4: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add mcp-server/src/gemini/tools.ts
git commit -m "refactor(gemini): use runtime.ts bridge instead of inline fallback

Removes duplicated executeWithFallback/spawnWithFallback. All daemon
lifecycle now routes through GeminiRuntime singleton. No tool changes."
```

---

## Phase 2: Production MCP Refinements

### Task 6: Harden cli-executor.ts with process group kill

**Files:**
- Modify: `mcp-server/src/shared/cli-executor.ts`
- Reference: `/Users/mikeboscia/.claude/mcp-servers/inter-agent/src/shared/cli-executor.ts`

- [ ] **Step 1: Diff the two cli-executor.ts files**

```bash
diff /Users/mikeboscia/.claude/mcp-servers/inter-agent/src/shared/cli-executor.ts \
     /Users/mikeboscia/projects/triumvirate/mcp-server/src/shared/cli-executor.ts | head -80
```

Identify the process group kill changes:
1. `detached: true` on spawn options
2. `proc.unref()` after spawn
3. `process.kill(-proc.pid, "SIGKILL")` with fallback to `proc.kill("SIGKILL")`

- [ ] **Step 2: Apply the 3 changes**

For each `spawn()` call in cli-executor.ts:
- Add `detached: true` to spawn options
- Add `proc.unref()` after spawn
- Replace `proc.kill("SIGKILL")` with:
```typescript
try {
  process.kill(-proc.pid!, "SIGKILL");
} catch {
  proc.kill("SIGKILL");
}
```

- [ ] **Step 3: Verify build**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server && npx tsc --noEmit
```
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add mcp-server/src/shared/cli-executor.ts
git commit -m "fix(cli-executor): process group kill prevents hanging subprocesses

Spawn with detached:true + unref, kill via process group (-pid) with
fallback to direct kill. Prevents orphaned gemini/codex child processes
on timeout."
```

---

### Task 7: Build and verify full MCP server

**Files:**
- Build output: `mcp-server/dist/` (gitignored — users build locally via `npm run build`)

- [ ] **Step 1: Full build**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server && npm run build
```
Expected: Clean compilation, all .js + .d.ts files in dist/.

- [ ] **Step 2: Verify oracle files in dist/**

```bash
ls -la /Users/mikeboscia/projects/triumvirate/mcp-server/dist/oracle-*.js
ls -la /Users/mikeboscia/projects/triumvirate/mcp-server/dist/gemini/runtime.js
```
Expected: All three files present.

- [ ] **Step 3: Smoke test — server starts without crashing**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server
timeout 3 node dist/gemini/server.js 2>&1 || true
```
Expected: No crash errors. Process exits on timeout (no stdin transport connected).

Note: `dist/` is in `.gitignore` — no commit needed. Users run `npm run build` during install.

---

## Phase 3: Advanced Hooks

### Task 8: Copy 4 advanced hooks

**Files:**
- Create: `starter-kit/claude/hooks/session-start-v3.sh`
- Create: `starter-kit/claude/hooks/pre-tool-use-supabase-mcp-gate.sh`
- Create: `starter-kit/claude/hooks/post-tool-use-oracle-pressure.sh`
- Create: `starter-kit/claude/hooks/post-tool-use-mode-nudge.sh`
- Source: `/Users/mikeboscia/.claude/hooks/`

- [ ] **Step 1: Copy all 4 hooks**

```bash
for hook in session-start-v3.sh pre-tool-use-supabase-mcp-gate.sh post-tool-use-oracle-pressure.sh post-tool-use-mode-nudge.sh; do
  cp "/Users/mikeboscia/.claude/hooks/$hook" \
     "/Users/mikeboscia/projects/triumvirate/starter-kit/claude/hooks/$hook"
done
```

- [ ] **Step 2: Scrub personal paths in all 4**

```bash
for hook in session-start-v3.sh pre-tool-use-supabase-mcp-gate.sh post-tool-use-oracle-pressure.sh post-tool-use-mode-nudge.sh; do
  grep -n 'mikeboscia\|michaeljboscia\|/Users/' \
    "/Users/mikeboscia/projects/triumvirate/starter-kit/claude/hooks/$hook"
done
```
Replace all matches with `$HOME` or generic references.

- [ ] **Step 3: Ensure all scripts are executable**

```bash
chmod +x /Users/mikeboscia/projects/triumvirate/starter-kit/claude/hooks/session-start-v3.sh
chmod +x /Users/mikeboscia/projects/triumvirate/starter-kit/claude/hooks/pre-tool-use-supabase-mcp-gate.sh
chmod +x /Users/mikeboscia/projects/triumvirate/starter-kit/claude/hooks/post-tool-use-oracle-pressure.sh
chmod +x /Users/mikeboscia/projects/triumvirate/starter-kit/claude/hooks/post-tool-use-mode-nudge.sh
```

- [ ] **Step 4: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add starter-kit/claude/hooks/
git commit -m "feat(hooks): add 4 advanced hooks from production

- session-start-v3.sh: orphan recovery (stale locks/saves cleanup)
- pre-tool-use-supabase-mcp-gate.sh: blocks MCP SQL without fresh backup
- post-tool-use-oracle-pressure.sh: context pressure monitoring for oracles
- post-tool-use-mode-nudge.sh: suggests execution mode after 15+ tool calls"
```

---

### Task 9: Update settings.json with new hook registrations

**Files:**
- Modify: `starter-kit/claude/settings.json:9-79`

- [ ] **Step 1: Add session-start-v3.sh to SessionStart**

After the `post-compact-recovery.sh` entry (line 27), add:

```json
,
{
  "matcher": "*",
  "hooks": [
    {
      "type": "command",
      "command": "~/.claude/hooks/session-start-v3.sh"
    }
  ]
}
```

- [ ] **Step 2: Add supabase-mcp-gate to PreToolUse**

After the `bash-guard.sh` entry (line 58), add:

```json
,
{
  "matcher": "mcp__supabase__apply_migration|mcp__supabase__execute_sql",
  "hooks": [
    {
      "type": "command",
      "command": "~/.claude/hooks/pre-tool-use-supabase-mcp-gate.sh"
    }
  ]
}
```

- [ ] **Step 3: Add oracle-pressure and mode-nudge to PostToolUse**

After the `token-gate.sh` entry (line 78), add:

```json
,
{
  "matcher": "*",
  "hooks": [
    {
      "type": "command",
      "command": "~/.claude/hooks/post-tool-use-oracle-pressure.sh"
    }
  ]
},
{
  "matcher": "*",
  "hooks": [
    {
      "type": "command",
      "command": "~/.claude/hooks/post-tool-use-mode-nudge.sh"
    }
  ]
}
```

- [ ] **Step 4: Also update PostToolUse matcher to include Agent**

The existing `post-tool-use.sh` matcher is `"Edit|Write|Bash"` (line 62). Production uses `"Edit|Write|Bash|Agent"`. Update to match.

- [ ] **Step 5: Validate JSON syntax**

```bash
python3 -c "import json; json.load(open('/Users/mikeboscia/projects/triumvirate/starter-kit/claude/settings.json'))"
```
Expected: No errors.

- [ ] **Step 6: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add starter-kit/claude/settings.json
git commit -m "feat(settings): register 4 advanced hooks + fix Agent matcher

SessionStart: +session-start-v3.sh (orphan recovery)
PreToolUse: +supabase-mcp-gate.sh (MCP SQL safety)
PostToolUse: +oracle-pressure.sh, +mode-nudge.sh, fix Agent matcher"
```

---

## Phase 4: Core Claude Skills

### Task 10: Create claude skills directory with 8 core skills

**Files:**
- Create: `starter-kit/claude/skills/` directory
- Create: 8 SKILL.md files (one per skill directory)
- Source: `/Users/mikeboscia/.claude/skills/`

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p /Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/{inter-agent-protocol,context-before-action,documentation-standards,our-systematic-debugging,persist-or-fail,file-taxonomy,crystallize,orchestrator-not-compute}
```

- [ ] **Step 2: Copy each SKILL.md**

```bash
for skill in inter-agent-protocol context-before-action documentation-standards our-systematic-debugging persist-or-fail file-taxonomy crystallize orchestrator-not-compute; do
  cp "/Users/mikeboscia/.claude/skills/$skill/SKILL.md" \
     "/Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/$skill/SKILL.md"
done
```

- [ ] **Step 3: Scrub personal paths/references in all skills**

```bash
for skill in inter-agent-protocol context-before-action documentation-standards our-systematic-debugging persist-or-fail file-taxonomy crystallize orchestrator-not-compute; do
  echo "=== $skill ==="
  grep -n 'mikeboscia\|michaeljboscia\|Binary Anvil\|binary.anvil\|binaryenvil' \
    "/Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/$skill/SKILL.md" || echo "(clean)"
done
```
Replace all matches with generic references. Business names → "[your-company]". Usernames → "[your-username]".

- [ ] **Step 4: Copy crystallize subdirectories**

The `crystallize` skill has additional files beyond SKILL.md. Copy them all:

```bash
cp /Users/mikeboscia/.claude/skills/crystallize/enforcement.md \
   /Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/crystallize/enforcement.md
cp /Users/mikeboscia/.claude/skills/crystallize/validation.md \
   /Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/crystallize/validation.md
cp -r /Users/mikeboscia/.claude/skills/crystallize/factory \
   /Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/crystallize/factory
cp -r /Users/mikeboscia/.claude/skills/crystallize/reference \
   /Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/crystallize/reference
```

Scrub personal paths in all copied files:
```bash
grep -rn 'mikeboscia\|michaeljboscia\|Binary Anvil' \
  /Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills/crystallize/
```

- [ ] **Step 5: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add starter-kit/claude/skills/
git commit -m "feat(skills): add 8 core operating skills for Claude

- inter-agent-protocol: messaging, peer review, auto-escalation
- context-before-action: verify codebase state before declaring
- documentation-standards: DNA-level docs enforcement
- our-systematic-debugging: debugging methodology
- persist-or-fail: mandatory persistence for compute results
- file-taxonomy: file organization decision tree
- crystallize: failure → skill matrix crystallization
- orchestrator-not-compute: orchestrate, don't raw-compute"
```

---

## Phase 5: Config Templates

### Task 11: Create mcp_config.json template

**Files:**
- Create: `starter-kit/shared/mcp_config.json.example`

- [ ] **Step 1: Write the template**

Create a minimal MCP config that wires up the two inter-agent servers:

```json
{
  "mcpServers": {
    "inter-agent-gemini": {
      "command": "node",
      "args": ["<TRIUMVIRATE_PATH>/mcp-server/dist/gemini/server.js"],
      "env": {
        "GEMINI_API_KEY": "${GEMINI_API_KEY}"
      }
    },
    "inter-agent-codex": {
      "command": "node",
      "args": ["<TRIUMVIRATE_PATH>/mcp-server/dist/codex/server.js"],
      "env": {}
    }
  }
}
```

Add comments explaining:
- Replace `<TRIUMVIRATE_PATH>` with actual clone path
- This goes in `~/.claude.json` (Claude Code) or equivalent
- Optional: add other MCP servers (supabase, github, etc.)

- [ ] **Step 2: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add starter-kit/shared/mcp_config.json.example
git commit -m "docs: add MCP server config template"
```

---

### Task 12: Create settings.local.json template (oracle permissions)

**Files:**
- Create: `starter-kit/claude/settings.local.json.example`

- [ ] **Step 1: Extract oracle tool permissions from production**

```bash
grep 'oracle' /Users/mikeboscia/.claude/settings.local.json
```

Create an example file with the oracle MCP tool permissions that would go in the user's `settings.local.json`:

```json
{
  "permissions": {
    "allow": [
      "mcp__inter-agent-gemini__oracle_init",
      "mcp__inter-agent-gemini__oracle_health",
      "mcp__inter-agent-gemini__oracle_refresh",
      "mcp__inter-agent-gemini__oracle_sync_corpus",
      "mcp__inter-agent-gemini__oracle_pressure_check",
      "mcp__inter-agent-gemini__oracle_log_learning",
      "mcp__inter-agent-gemini__oracle_checkpoint",
      "mcp__inter-agent-gemini__oracle_salvage",
      "mcp__inter-agent-gemini__oracle_reconstitute",
      "mcp__inter-agent-gemini__oracle_quality_report",
      "mcp__inter-agent-gemini__oracle_add_to_corpus",
      "mcp__inter-agent-gemini__oracle_update_entry",
      "mcp__inter-agent-gemini__spawn_oracle"
    ],
    "ask": [
      "mcp__inter-agent-gemini__oracle_decommission_request",
      "mcp__inter-agent-gemini__oracle_decommission_cancel",
      "mcp__inter-agent-gemini__oracle_decommission_execute"
    ]
  }
}
```

Note: decommission tools require manual approval (destructive).

- [ ] **Step 2: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add starter-kit/claude/settings.local.json.example
git commit -m "docs: add oracle permissions template for settings.local.json"
```

---

### Task 13: Create lessons template + stenographer workers

**Files:**
- Create: `starter-kit/claude/lessons/TEMPLATE.md`
- Create: `starter-kit/stenographer/session-save-ctl.py`
- Create: `starter-kit/stenographer/session-save-worker.py`
- Source: `/Users/mikeboscia/.triumvirate/stenographer/`

- [ ] **Step 1: Create lessons template**

```markdown
# Lessons Learned

> Real-time capture of errors, abandoned approaches, and non-obvious decisions.
> Claude adds entries here during sessions. Review periodically.

## Template

### YYYY-MM-DD: [Title]
**Context:** What were you trying to do?
**What happened:** What went wrong / what was surprising?
**Lesson:** What to do differently next time.
**Applies to:** [scope — e.g., "all Supabase migrations", "batch processing"]
```

- [ ] **Step 2: Copy stenographer workers**

```bash
cp /Users/mikeboscia/.triumvirate/stenographer/session-save-ctl.py \
   /Users/mikeboscia/projects/triumvirate/starter-kit/stenographer/session-save-ctl.py
cp /Users/mikeboscia/.triumvirate/stenographer/session-save-worker.py \
   /Users/mikeboscia/projects/triumvirate/starter-kit/stenographer/session-save-worker.py
```

- [ ] **Step 3: Scrub personal paths**

```bash
grep -n 'mikeboscia\|michaeljboscia\|/Users/' \
  /Users/mikeboscia/projects/triumvirate/starter-kit/stenographer/session-save-ctl.py \
  /Users/mikeboscia/projects/triumvirate/starter-kit/stenographer/session-save-worker.py
```
Replace all matches.

- [ ] **Step 4: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add starter-kit/claude/lessons/ starter-kit/stenographer/session-save-ctl.py starter-kit/stenographer/session-save-worker.py
git commit -m "feat: add lessons template + stenographer background workers

- lessons/TEMPLATE.md: structured format for real-time lesson capture
- session-save-ctl.py: controller for background save orchestration
- session-save-worker.py: worker process for async session saves"
```

---

## Phase 6: Install Script + Documentation

### Task 14: Update install.sh for skills and oracle

**Files:**
- Modify: `starter-kit/install.sh`

- [ ] **Step 1: Read the full install.sh to understand structure**

```bash
wc -l /Users/mikeboscia/projects/triumvirate/starter-kit/install.sh
cat /Users/mikeboscia/projects/triumvirate/starter-kit/install.sh
```

- [ ] **Step 2: Add skills installation section**

After the hooks installation section, add a new section:

```bash
# ── Claude Skills ──────────────────────────────────────────────
info "Installing Claude skills..."
SKILLS_SRC="$SCRIPT_DIR/claude/skills"
SKILLS_DST="$HOME/.claude/skills"

if [[ -d "$SKILLS_SRC" ]]; then
  mkdir -p "$SKILLS_DST"
  for skill_dir in "$SKILLS_SRC"/*/; do
    skill_name="$(basename "$skill_dir")"
    if [[ -d "$SKILLS_DST/$skill_name" ]]; then
      warn "Skill '$skill_name' already exists — skipping (won't overwrite)"
    else
      cp -r "$skill_dir" "$SKILLS_DST/$skill_name"
      ok "Installed skill: $skill_name"
    fi
  done
fi
```

- [ ] **Step 3: Add oracle opt-in prompt**

After the MCP server build section, add:

```bash
# ── Oracle Engine (opt-in) ─────────────────────────────────────
echo ""
read -p "Enable Pythia Oracle Engine? (requires Gemini API key) [y/N]: " enable_oracle
if [[ "$enable_oracle" =~ ^[Yy] ]]; then
  info "Oracle engine enabled — oracle tools will be registered in the MCP server"
  info "Add oracle permissions to ~/.claude/settings.local.json (see settings.local.json.example)"
  ok "Oracle ready"
else
  info "Oracle engine skipped — basic inter-agent tools only"
  warn "To enable later, see ARCHITECTURE.md § Oracle Engine"
fi
```

- [ ] **Step 4: Add stenographer workers to stenographer install section**

Ensure `session-save-ctl.py` and `session-save-worker.py` are copied alongside the existing stenographer files.

- [ ] **Step 5: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add starter-kit/install.sh
git commit -m "feat(install): add skills copy + oracle opt-in + stenographer workers"
```

---

### Task 15: Update ARCHITECTURE.md with oracle documentation

**Files:**
- Modify: `ARCHITECTURE.md`

- [ ] **Step 1: Add oracle section**

Add a new section after the existing MCP server documentation:

```markdown
## Oracle Engine (Pythia)

The oracle system provides **persistent Gemini knowledge daemons** that survive
session boundaries and context compaction. An oracle holds research documents,
codebase context, and accumulated learnings in Gemini's 2M token window.

### Architecture
- **Registry:** `~/.triumvirate/oracle-registry.json` — tracks all oracles
- **State:** Per-oracle state file with interaction history
- **Manifest:** Corpus definition (static entries + live sources)
- **Runtime Bridge:** `OracleRuntimeBridge` interface decouples oracle-tools
  from Gemini daemon lifecycle

### Tool Categories
1. **Lifecycle:** oracle_init, spawn_oracle, oracle_decommission_*
2. **Health:** oracle_health, oracle_pressure_check, oracle_quality_report
3. **Corpus:** oracle_sync_corpus, oracle_add_to_corpus, oracle_update_entry
4. **Persistence:** oracle_checkpoint, oracle_salvage, oracle_reconstitute
5. **Learning:** oracle_log_learning, oracle_refresh
```

- [ ] **Step 2: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add ARCHITECTURE.md
git commit -m "docs: add oracle engine architecture section"
```

---

### Task 16: Update README.md with new features

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update feature list**

Add oracle engine, advanced hooks, and skills to the feature list in README.md. Update the "What's Included" section to reflect all new components.

- [ ] **Step 2: Add skills section**

Document the 8 core skills and explain how to add custom skills.

- [ ] **Step 3: Update install instructions**

Note the oracle opt-in during install and the skills directory.

- [ ] **Step 4: Commit**

```bash
cd /Users/mikeboscia/projects/triumvirate
git add README.md
git commit -m "docs: update README with oracle engine, skills, advanced hooks"
```

---

## Phase 7: Final Verification

### Task 17: End-to-end verification

- [ ] **Step 1: Clean build**

```bash
cd /Users/mikeboscia/projects/triumvirate/mcp-server
rm -rf dist/ node_modules/
npm install
npm run build
```
Expected: Clean build, no errors, no warnings.

- [ ] **Step 2: Verify no personal paths leaked**

```bash
grep -r 'mikeboscia\|michaeljboscia\|Binary Anvil\|binary.anvil' \
  /Users/mikeboscia/projects/triumvirate/ \
  --include='*.ts' --include='*.sh' --include='*.py' --include='*.md' --include='*.json' \
  --exclude-dir=node_modules --exclude-dir=.git --exclude-dir=dist --exclude-dir=session-logs
```
Expected: ZERO matches. If any found, fix immediately.

- [ ] **Step 3: Verify file count**

```bash
echo "=== Oracle Engine ==="
wc -l /Users/mikeboscia/projects/triumvirate/mcp-server/src/oracle-*.ts
wc -l /Users/mikeboscia/projects/triumvirate/mcp-server/src/gemini/runtime.ts

echo "=== Hooks ==="
ls /Users/mikeboscia/projects/triumvirate/starter-kit/claude/hooks/*.sh | wc -l

echo "=== Skills ==="
find /Users/mikeboscia/projects/triumvirate/starter-kit/claude/skills -name 'SKILL.md' | wc -l

echo "=== Stenographer ==="
ls /Users/mikeboscia/projects/triumvirate/starter-kit/stenographer/*.py | wc -l
```

Expected:
- Oracle: ~5,900 lines across 3 files
- Hooks: 12 scripts (8 original + 4 new)
- Skills: 8 SKILL.md files
- Stenographer: 8+ Python files

- [ ] **Step 4: Verify git is clean**

```bash
cd /Users/mikeboscia/projects/triumvirate && git status
```
Expected: Clean working tree, all changes committed.

---

## Summary

| Phase | Tasks | New Lines (approx) | Description |
|-------|-------|-------------------|-------------|
| 1 | 1-5 | ~5,900 | Oracle engine (types, runtime, tools, server wiring) |
| 2 | 6-7 | ~50 | CLI executor hardening + build |
| 3 | 8-9 | ~550 | 4 advanced hooks + settings.json |
| 4 | 10 | ~1,500 | 8 core Claude skills |
| 5 | 11-13 | ~200 | Config templates + lessons + stenographer workers |
| 6 | 14-16 | ~150 | Install script update + docs |
| 7 | 17 | 0 | End-to-end verification |
| **Total** | **17 tasks** | **~8,350 lines** | |
