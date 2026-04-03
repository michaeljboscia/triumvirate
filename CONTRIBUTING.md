# Contributing to Triumvirate

Issues and PRs welcome. Read this first.

---

## Project Structure

```
triumvirate/
├── mcp-server/              # Git submodule → inter-agent-mcp repo
│   ├── src/
│   │   ├── server.ts        # Entry point — McpServer instantiation
│   │   ├── unified-tools.ts # Tool registration (spawn, ask, dismiss, etc.)
│   │   ├── types.ts         # Shared type definitions
│   │   ├── gemini/          # Gemini CLI wrapper (controller, runtime, tools, model fallback)
│   │   ├── codex/           # Codex CLI wrapper (controller, tools)
│   │   ├── shared/          # Context detection, session logs, job store, scratchpad
│   │   ├── oracle-tools.ts  # Oracle engine (persistent knowledge daemons)
│   │   └── oracle-types.ts  # Oracle type definitions
│   ├── package.json
│   └── tsconfig.json
├── stenographer/            # Python — incremental session notes via local Ollama
│   ├── stenographer.py      # Core narrator
│   ├── session_log_path.py  # Versioned log path computation
│   ├── health_check.py      # Ollama + model health verification
│   ├── gap_fill.py          # Compaction gap-fill via Gemini CLI
│   ├── parsers/             # Transcript parsers per agent
│   └── prompts/             # Ollama system prompts
├── starter-kit/
│   ├── install.sh           # Interactive installer — copies hooks, skills, builds MCP server
│   ├── claude/
│   │   ├── hooks/           # 12 bash hooks (session-start, pre-compact, pre/post-tool-use)
│   │   ├── skills/          # 8 Claude skills (inter-agent-protocol, persist-or-fail, etc.)
│   │   ├── rules/           # Auto-loaded rule files
│   │   ├── settings.json    # Hook wiring for Claude Code
│   │   └── CLAUDE.md        # Behavioral contract template
│   ├── codex/               # Codex hooks, skills, AGENTS.md, config.toml
│   ├── gemini/              # Gemini hooks, GEMINI.md
│   └── shared/              # Cross-agent shared config
├── docs/                    # Reference documentation
├── examples/                # Usage examples
├── ARCHITECTURE.md          # How the daemon system works
└── SESSION_LOG_SPEC.md      # Cross-agent session log standard
```

**Key distinction:** `mcp-server/` is a git submodule pointing to [`inter-agent-mcp`](https://github.com/michaeljboscia/inter-agent-mcp). PRs that change MCP server code go to that repo, not this one. PRs to everything else go here.

---

## Building the MCP Server

```bash
cd mcp-server
npm install
npm run build
```

Requires Node.js 20+. Success = clean TypeScript compilation, zero errors. The build output lands in `mcp-server/dist/`. The server runs as `node dist/server.js` via stdio transport — no HTTP, no deployment.

If you cloned without `--recurse-submodules`:

```bash
git submodule update --init --recursive
```

---

## Adding a New MCP Tool

MCP server changes go to the [inter-agent-mcp](https://github.com/michaeljboscia/inter-agent-mcp) repo. Fork that, not this one.

The tool registration pattern lives in `src/unified-tools.ts`. Every tool follows the same shape:

1. Define the tool in `registerUnifiedTools()` using `server.tool()`:

```typescript
server.tool(
  "tool_name",
  "Description of what the tool does.",
  {
    // Zod schema for parameters
    param: z.string().describe("What this param is."),
    timeout_ms: z.number().min(1000).max(MAX_TIMEOUT_MS).optional()
      .describe(`Timeout in ms (default: ${DEFAULT_TIMEOUT_MS}).`),
  },
  async (args) => {
    // Route to the correct backend controller
    // Return { content: [{ type: "text", text: "result" }] }
  }
);
```

2. If the tool targets a specific agent, route by daemon ID prefix (`gd_` = Gemini, `cd_` = Codex). See `routeByDaemonId()` in `unified-tools.ts`.

3. If the tool needs new controller methods, add them to `src/gemini/controller.ts` or `src/codex/controller.ts`.

4. Rebuild: `npm run build`. Test by spawning the server and calling the tool via Claude Code.

---

## Adding a New Hook

Hooks are bash scripts in `starter-kit/claude/hooks/`. Claude Code fires them at specific lifecycle events.

1. Write your hook script. Convention: name it `<event>-<description>.sh`. Events are:
   - `session-start` — fires on session open (and on compaction recovery)
   - `pre-compact` — fires before context window compaction
   - `pre-tool-use` — fires before a tool call (matcher selects which tools)
   - `post-tool-use` — fires after a tool call

2. Make it executable: `chmod +x starter-kit/claude/hooks/your-hook.sh`

3. Wire it in `starter-kit/claude/settings.json`. Add an entry under the appropriate event:

```json
{
  "matcher": "Edit|Write",
  "hooks": [
    {
      "type": "command",
      "command": "~/.claude/hooks/your-hook.sh"
    }
  ]
}
```

The `matcher` field is a pipe-delimited list of tool names, or `*` for all tools. Hooks receive context via stdin (JSON with tool name, arguments, and session metadata).

4. Update `starter-kit/install.sh` if the hook needs to be copied during installation.

5. Document the hook in `docs/hooks/`.

For Codex hooks: `starter-kit/codex/hooks/`. For Gemini hooks: `starter-kit/gemini/hooks/`.

---

## Adding Terminal Emulator or Agent Support

Triumvirate currently supports three agents (Claude Code, Gemini CLI, Codex CLI). Adding a new agent backend requires changes in the MCP server repo:

1. Create `src/<agent>/` with at minimum:
   - `controller.ts` — manages daemon lifecycle (spawn, ask, dismiss)
   - `tools.ts` — agent-specific tool registrations (if any)

2. Pick a daemon ID prefix (two letters + underscore, e.g., `gd_`, `cd_`). Register it in `routeByDaemonId()` in `unified-tools.ts`.

3. Implement the `AgentController` interface — must support `spawn()`, `ask()`, `dismiss()` at minimum.

4. Wire the controller into `server.ts` and `registerUnifiedTools()`.

5. Add starter-kit support:
   - Create `starter-kit/<agent>/` with the agent's config files, hooks, and behavioral contract
   - Update `install.sh` to handle the new agent
   - Add transcript parser in `stenographer/parsers/` if the agent produces transcripts

Adding terminal emulator support (iTerm2, Wezterm, etc.) is a separate concern — hooks and the Stenographer interact with the filesystem, not terminal APIs. If your emulator needs specific integration, open an issue describing the use case.

---

## Testing

**Current state:** Ad-hoc smoke tests. The `mcp-server/src/` directory has several `test-*.mjs` files that exercise oracle functionality by mocking controllers and running scenarios. These are manual — run them with `node test-<name>.mjs`.

**Aspiration:** A proper test suite with:
- Unit tests for controller logic (spawn, ask, dismiss lifecycle)
- Integration tests that verify tool registration and parameter validation
- End-to-end tests that spawn real Gemini/Codex daemons (requires CLIs installed)

If you're adding a feature, include at minimum a smoke test that exercises the happy path. If you're fixing a bug, include a test that reproduces the failure before the fix.

No test framework has been chosen yet. If you want to set one up, open an issue first so we can agree on the approach.

---

## The Twin Review Process

PRs are reviewed by Gemini and Codex. This is the actual workflow, not a gimmick.

When a PR is submitted:
1. A human does initial triage
2. Gemini reviews for architecture, context coherence, and session log compatibility
3. Codex reviews for code correctness, type safety, and edge cases
4. Both post structured review comments on the PR

The agents have caught real bugs this way — daemon cross-talk, git commit ordering issues, hardcoded filters that broke multi-agent log versioning. If you get review comments from agents, treat them like you would any other reviewer's feedback.

You do not need to set this up yourself. The maintainers run the reviews.

---

## Code Conventions

**MCP server (`mcp-server/`):** TypeScript. Strict mode. Zod for parameter validation. No external HTTP dependencies — everything runs as a local stdio process.

**Stenographer (`stenographer/`):** Python. No framework. Talks to Ollama via HTTP localhost. No pip dependencies beyond standard library (uses `urllib` not `requests`).

**Hooks (`starter-kit/*/hooks/`):** Bash. POSIX-compatible where possible. Must be idempotent — hooks can fire multiple times. Use `jq` for JSON parsing (assumed available). Always `set -uo pipefail` at the top (no `-e` — hooks must fail open, never block Claude Code).

**Skills (`starter-kit/claude/skills/`):** Markdown files with structured prompt content. Follow the existing format — look at `inter-agent-protocol/` for the pattern.

**General:**
- No linter or formatter is enforced yet. Match the style of the file you're editing.
- Fully qualified file paths in all agent-facing content (CLAUDE.md, skills, hook output). Relative paths break on compaction.
- Commit messages: short, imperative. "Add daemon timeout retry" not "Added daemon timeout retry logic to the unified tools file."

---

## Submodule Workflow

The MCP server is a git submodule. This matters for PRs:

- **Changing hooks, skills, stenographer, docs, install.sh** — PR goes to this repo (`triumvirate`).
- **Changing MCP server code** — PR goes to [`inter-agent-mcp`](https://github.com/michaeljboscia/inter-agent-mcp). Once merged there, we update the submodule pointer here.
- **Changing both** — two PRs. Server PR first, then a triumvirate PR that bumps the submodule ref.

To update the submodule to latest:

```bash
cd mcp-server
git pull origin main
cd ..
git add mcp-server
git commit -m "Bump mcp-server submodule"
```

---

## License

By contributing, you agree that your contributions will be licensed under the project's [FSL-1.1-ALv2](LICENSE) license (Functional Source License, auto-converting to Apache 2.0 after two years per release).
