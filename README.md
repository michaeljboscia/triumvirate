# Triumvirate

[![Rust CI](https://github.com/michaeljboscia/triumvirate/actions/workflows/rust.yml/badge.svg)](https://github.com/michaeljboscia/triumvirate/actions)
[![Version](https://img.shields.io/github/v/release/michaeljboscia/triumvirate)](https://github.com/michaeljboscia/triumvirate/releases)
[![License: FSL-1.1-ALv2](https://img.shields.io/badge/License-FSL--1.1--ALv2-blue.svg)](LICENSE)

**Three AI models. One daemon. A methodology for building software with all of them at once.**

Claude, Gemini, and Codex, each with different strengths, working on the same codebase, coordinated by a single Rust daemon, visible in real time from inside your editor.

## Quick Start

```bash
git clone https://github.com/michaeljboscia/triumvirate
cd triumvirate/daemon
cargo build --release && cargo run --release
```

That's it. One binary. No Docker. No NATS. No cloud services. Register it in Claude Code:

```bash
# Add to ~/.claude.json under mcpServers:
"triumvirate": { "command": "/path/to/triumvirate", "args": ["mcp"] }
```

Then from any Claude session: `spawn a Gemini session called 'research'`

> **Requirements:** Rust 1.82+ and at least one agent CLI ([Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Gemini CLI](https://github.com/google-gemini/gemini-cli), or [Codex CLI](https://github.com/openai/codex)). Works with 1, 2, or all 3.

---

## The Problem

You're using Claude to write code. You want Gemini to research something. You want Codex to implement a plan in parallel. So you open three terminal windows, copy-paste context between them, lose track of which one is doing what, and eventually something goes sideways because nobody can see what anyone else is doing.

Triumvirate replaces the terminal windows. One daemon holds the context, so the agents can actually see each other work.

---

## What It Does Today

**The daemon:** a Rust MCP server that spawns and manages persistent agent sessions.

```
You:     "spawn a Gemini session called 'research'"
Claude:  → spawn_session(agent: "gemini", session_name: "research")
         ✓ Session research spawned

You:     "ask research to analyze our auth middleware"
Claude:  → ask_session(session_name: "research", prompt: "...")
         → Gemini: turn started
         → Gemini: calling read_file (src/middleware/auth.rs)
         → Gemini: calling read_file (src/middleware/jwt.rs)
         → Gemini: generating response
         → Gemini: responded (12,847 in / 1,203 out / 8,400 cached, 2 tools, 4.1s)
         "The auth middleware has three issues..."

You:     "spawn a Codex session and have it fix what Gemini found"
```

Persistent sessions. Live streaming. Cross-model coordination. From inside Claude, with no window switching.

**The methodology:** skills that use the daemon to run structured multi-agent processes.

| Skill | What It Does |
|-------|-------------|
| **`/goatrodeo`** | Multi-round spec review. Spawns Gemini and Codex as adversarial reviewers. They interrogate your spec, auto-resolve what they agree on, and surface only what needs human judgment. Produces battle-tested requirements with traceable REQ-IDs. |
| **`/postrodeo`** | Post-build retrospective. Audits what was built against what was specced. Spawns twins to review code diffs. Produces a completion matrix, deviation analysis, and lessons. |
| **`/design-goatrodeo`** | Design-specific variant. Pressure-tests visual specs, UX flows, and information architecture. |

**The operating environment:** a starter kit that wires all three agents together.

```bash
cd starter-kit && ./install.sh
```

Hooks, configs, skills, and session notes for Claude, Codex, and Gemini. Plus a local stenographer that captures session notes via Ollama, at zero cloud cost.

---

## The Loop

This is how software gets built with Triumvirate:

```
         ┌─────────────────────────────────────────┐
         │                                         │
    Spec ──→ /goatrodeo ──→ Implementation ──→ /postrodeo
         │   (Gemini +       (Codex builds      (twins audit   │
         │    Codex review    from the spec)      the code)     │
         │    the spec)                                         │
         │                                         │
         └──── lessons feed back into next cycle ──┘
```

The goatrodeo runs *through the daemon*. It spawns Gemini and Codex as daemon sessions to review your spec. The postrodeo does the same for code review. The daemon is the infrastructure the methodology runs on.

**This project was built using this loop.** The Flow State feature (live agent streaming) went through a 4-round goatrodeo: 27 requirements, 29 auto-resolves, 5 human decisions, all running through the daemon. Then Codex implemented all 6 phases from the resulting spec while the human took a nap.

The tool that coordinates agents was built by agents coordinating through the tool. It is recursion with a paycheck.

---

## What's Shipped

### Daemon (`daemon/`)

| Feature | What It Does |
|---------|-------------|
| **Persistent sessions** | Spawn named Gemini/Codex sessions that survive across requests. Resume conversations. |
| **Live streaming** | See tool calls, file reads, commands, and response generation as they happen, not after. |
| **Stuck detection** | Catches agents that idle >60s, loop the same tool >5x, or freeze. Surfaces it immediately. |
| **Token visibility** | Every response includes input/output/cached token counts and duration. |
| **Shared memory** | Agents read and write to a shared key-value store. Decisions persist across sessions. |
| **Scratchpad** | Quick cross-agent notes. Agent A writes, Agent B reads. |
| **Outbox** | Event log of every agent interaction: who asked what, who responded, when. |
| **Fallback outbox** | If an agent fails, the request is saved to disk. Nothing is lost. Retry later. |
| **Verbosity control** | `quiet`, `standard`, `detailed`, `raw`: choose your noise level. |

### Skills (`skills/`)

| Skill | Lines | What It Does |
|-------|-------|-------------|
| `/goatrodeo` | ~900 | Industrialized spec review. Interrogation rounds, live research, twin review, auto-resolve, decision ledger. |
| `/postrodeo` | ~600 | Build retrospective. Completion matrix, deviation analysis, Layer 6 semantic check, twin code review. |
| `/design-goatrodeo` | ~500 | Design spec variant. Visual standards, UX flows, information architecture. |

### Starter Kit (`starter-kit/`)

Full installer for multi-agent development:
- **Claude:** hooks (session lifecycle, token gating, artifact protection), skills, CLAUDE.md
- **Codex:** hooks (session recovery, pre-compact), config, AGENTS.md
- **Gemini:** hooks (session recovery, pre-compact), GEMINI.md
- **Stenographer:** local session notes via Ollama (zero cloud cost)
- **MCP server registration:** wires agents to communicate through the daemon

### Stenographer (`stenographer/`)

Standalone Python tool. Reads agent transcripts, feeds them to a local Ollama model, and appends narrative session notes. Triggered automatically by hooks when transcript growth crosses a threshold.

---

## Architecture

```
triumvirate-daemon (single Rust binary, 13 crates)
│
├── MCP Server (rmcp): 35+ tools
│   Sessions, ABE dispatch, fleet, knowledge, review, tokens
│
├── HTTP API (axum): REST + Prometheus + WebSocket
│   Same tools as REST endpoints, plus /metrics, /ws, /api/tokens/*
│
├── ABE (Autonomous Build Enforcement)
│   ├── Fleet dispatch: N workers in isolated git worktrees
│   ├── Task tracker: state machine with 4-signal completion detection
│   ├── Contract enforcement: file scope, commit format, test gates
│   └── Failure classification: auto-retry with escalation
│
├── Token Economics (token-economics crate)
│   SQLite storage, session scanner (Claude/Codex/Gemini JSONL),
│   cost attribution by build/session/task
│
├── Agent Adapter (agent-adapter crate)
│   ├── GeminiStreamParser : parses stream-json NDJSON live
│   ├── CodexExecParser     : parses exec --json JSONL live
│   └── StuckDetector       : idle, loop, freeze detection
│
├── Daemon Core: metrics, observability bus, session lifecycle
│
└── Fallback Outbox: disk-persisted retry queue
```

13 crates. 150+ tests. Zero external services. Your laptop is the datacenter.

---

## Installation Options

| Option | Command | Build Required |
|--------|---------|----------------|
| **Daemon only** | `cd daemon && cargo build --release` | Yes (Rust) |
| **Skills only** | `cp skills/claude/*.md ~/.claude/skills/` | No |
| **Full stack** | `cd starter-kit && ./install.sh` | Yes (Rust) |

The full stack installer sets up: daemon + skills + hooks + configs + stenographer for all three agents.

---

## What's Next

v3.2.0 shipped the build dispatch half. What's left is polish and scale.

| Feature | Status | What It Unlocks |
|---------|--------|----------------|
| **Worktree-isolated Codex swarms** | **Shipped (v3.1)** | N Codex agents, each in its own git worktree. Contract enforcement, timeout, retry. |
| **Plan-aware task assignment** | **Shipped (v3.1)** | ABE reads a plan, assigns tasks by wave, respects dependencies. |
| **Token economics** | **Shipped (v3.2)** | Per-session, per-build, per-task cost tracking across all 3 agents. |
| **Full observability** | **Shipped (v3.2)** | `#[instrument]` spans on all ABE functions, 20 Prometheus metrics, structured logging. |
| **Cross-model code review** | **Shipped (v3.1)** | Agents review each other's code via peer-review queue. |
| **Dashboard** | Planned | Web UI for watching all agent sessions in real time. |
| **Fleet scaling** | Planned | Multiple orchestrators, mixed worker types, concurrent builds. |
| **Cedar governance** | Planned | Policy-based approval gates for destructive operations. |

The daemon is the coordination layer. The methodology runs on top of it. The agents do the work.

See [`ROADMAP.md`](ROADMAP.md) for the full plan.

---

## How It Was Built

This project is designed and built by a human (Mike Boscia) coordinating three AI agents:

- **Claude** (Opus 4.6): architecture, spec review, goatrodeo, documentation
- **Gemini** (Pro 2M context): research, competitive analysis, adversarial review
- **Codex** (GPT-5.2): implementation, from scaffold to shipped

Every feature goes through the loop: spec, goatrodeo, implementation, postrodeo. The agents review each other's work before and after building. The goatrodeo runs through the daemon. The implementation runs through the daemon. The tool is built by the process it enables.

37 research artifacts documenting the design process are in `archive/research/`.

---

## The Ecosystem

Triumvirate doesn't work alone. It's one layer in a stack of tools that collaborate in the same session:

| Tool | Role | How It Fits |
|------|------|-------------|
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | Primary development agent | The cockpit. You sit here. Triumvirate and Pythia plug in as MCP servers. Skills like `/goatrodeo` run from here. |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | Research + adversarial review | 2M context window. Goatrodeo spawns Gemini sessions for spec interrogation. Gemini search provides live web research during review rounds. |
| [Codex CLI](https://github.com/openai/codex) | Implementation engine | Builds from specs. Goatrodeo spawns Codex for implementer-perspective review. Then Codex builds the thing it just reviewed. |
| [Pythia](https://github.com/michaeljboscia/pythia) | Local code search | MCP server that indexes entire projects: code, docs, SQL, config, research. Agents query Pythia before making changes. Available in the same Claude session as Triumvirate. |
| [Ollama](https://ollama.com) | Local LLM for session notes | Stenographer feeds transcripts to a local model. Zero cloud cost. Zero token spend. |
| [MCP](https://modelcontextprotocol.io) | The protocol | Everything connects through MCP. Triumvirate is an MCP server. Pythia is an MCP server. Claude Code is the MCP client. One protocol, many tools, same session. |

The daily workflow: Claude Code has Triumvirate and Pythia both registered as MCP servers. You search code with Pythia, spawn agent sessions with Triumvirate, and run goatrodeos that use both, all without leaving the editor. The tools compose because they share a protocol.

---

## Prior Art

No source code was vendored from any of these projects. Where patterns were adapted, implementation was re-authored in Rust. Inline attribution comments appear at adaptation points in the source. Full details in [`NOTICE.md`](NOTICE.md).

| Project | License | What We Adapted |
|---------|---------|----------------|
| [Temporal](https://github.com/temporalio/temporal) | Apache 2.0 | Event-sourced workflow persistence, crash recovery, retry with backoff |
| [Ruflo](https://github.com/ruvnet/ruflo) | MIT | Multi-model agent routing, cost-optimized model selection, swarm coordination patterns |
| [Clash](https://github.com/nicholasgasior/clash) | MIT | Real-time git worktree conflict detection between parallel agents |
| [swarms-rs](https://github.com/swarms-rs) | Apache 2.0 | Rust agent lifecycle management, supervisor patterns |
| [Flotilla](https://github.com/UrsushoribilisMusic/agentic-fleet-hub) | Open source | Cross-model peer review as mandatory gate, structured lessons ledger, shared state patterns |
| [ensemble](https://github.com/michelhelsdingen/ensemble) | MIT | JSONL file-based message bus with fcntl locking, tmux session management |
| [RunDiffusion Agents](https://github.com/rundiffusion/RunDiffusion-Agents) | Apache 2.0 | YAML governance control plane, agent-manages-agents pattern |
| [AgentsMesh](https://github.com/AgentsMesh/AgentsMesh) | BSL-1.1 | gRPC+mTLS control plane architecture (studied only, not used in production per BSL-1.1 terms) |
| [Claude Agent Teams](https://docs.anthropic.com) | Anthropic | Git worktree isolation, shared task list with dependency tracking, peer-to-peer mailbox messaging |

---

## Acknowledgments

Triumvirate exists because three companies built AI agents good enough to coordinate:

- **Anthropic:** Claude Code and the MCP protocol that connects everything
- **Google:** Gemini CLI with stream-json and 2M context
- **OpenAI:** Codex CLI with exec --json and sandboxed execution

The agents are theirs. The coordination is ours. The methodology emerged from using all three together every day and finding out what works.

---

## License

[FSL-1.1-ALv2](LICENSE) (Functional Source License 1.1, Apache 2.0 Future License)
