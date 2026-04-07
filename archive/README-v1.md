# Triumvirate

**The first cross-model multi-agent fleet coordinator.**

A single Rust binary that orchestrates Claude, Gemini, and Codex as a dynamic fleet. Any number of any agent type, working in parallel on the same codebase, coordinated by one daemon, visible in one dashboard.

```bash
cd daemon && cargo run
# Dashboard at http://127.0.0.1:8080
```

---

## What It Does

You say: *"3 Claudes on auth, 2 Codexes on API, 1 Gemini researching."*

The daemon:
1. Spawns 6 persistent CLI subprocesses (each with its own git worktree)
2. Defines contracts before fan-out (interfaces, types, API shapes)
3. Assigns tasks from a shared task list with dependency tracking
4. Streams all output to a web dashboard in real-time
5. Merges results sequentially (one branch at a time, full context)
6. Tracks per-turn cost via Langfuse and Prometheus metrics
7. Captures session notes mechanically (no LLM summarization)

All agents share memory via SQLite. Decisions persist across sessions. The fleet gets smarter over time via a machine-readable lessons ledger.

---

## Architecture

```
triumvirate-agentd (single Rust binary)
|
+-- Agent Pool (N persistent subprocesses per type)
|   Claude: stream-json over stdio
|   Gemini: ACP JSON-RPC over stdio
|   Codex:  MCP JSON-RPC over stdio
|
+-- Message Fabric (Tokio broadcast/mpsc/watch channels)
+-- Workflow Engine (SQLite event-sourced state machine)
+-- Fleet Coordinator (worktrees + task list + sequential merge)
+-- Memory (SQLite WAL, daemon-extracted decisions)
+-- Stenographer (mechanical fact extraction)
+-- Governance (Cedar policy engine)
+-- Observability (Prometheus + Langfuse + OpenTelemetry)
+-- Dashboard (axum + Svelte + Tailwind, rust-embed)
```

No external dependencies. No Docker. No NATS. No Temporal. One binary.

---

## Requirements

- **Rust 1.93+** (`rustup update stable`)
- **Node.js 20+** (for Svelte dashboard build)
- At least one agent CLI installed:
  - `claude` — [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
  - `gemini` — [Gemini CLI](https://github.com/google-gemini/gemini-cli)
  - `codex` — [Codex CLI](https://github.com/openai/codex)

The daemon degrades gracefully — you can run with 1, 2, or all 3 agent types.

---

## Quick Start

```bash
git clone https://github.com/michaeljboscia/triumvirate-agentd
cd triumvirate-agentd/daemon
cargo build
cargo run
```

Open `http://127.0.0.1:8080` in your browser.

### Configuration

```bash
cp config/default.toml ~/.triumvirate/config.toml
# Edit to enable/disable agents, set port, configure Langfuse
```

### Testing

```bash
cargo test                     # Unit tests
cargo test --features mock     # Integration tests with mock CLIs
cargo clippy -- -D warnings    # Lint
```

---

## Features (31)

| Category | Features |
|----------|----------|
| **Agent Connectivity** | Per-agent native JSON adapters (Claude stream-json, Gemini ACP, Codex MCP), agent pool (N instances), provider abstraction (CLI + API backends), health monitoring, auto-restart |
| **Coordination** | Git worktree isolation, contracts-first fan-out, shared task list with dependencies, sequential merge, peer messaging |
| **Workflows** | Conversation, debate (Toulmin model), fleet orchestration, crash recovery via SQLite event sourcing |
| **Memory** | SQLite WAL shared store, daemon-extracted decisions with human confirmation, cross-session persistence |
| **Observability** | Prometheus /metrics, Langfuse LLM tracing, per-turn/task/fleet cost attribution, mechanical stenographer |
| **Dashboard** | Tasks view (executive), agents view (debug), dynamic grid, quota meters, routing log, workflow panel, cost panel |
| **Governance** | Cedar policy engine, human approval gates for destructive operations |
| **Learning** | Machine-readable lessons ledger with confidence decay (informed by Flotilla) |

Full PRD with acceptance criteria: [`docs/v2/PRD.md`](docs/v2/PRD.md)

---

## Documentation

| Doc | Purpose |
|-----|---------|
| [`SPEC.md`](SPEC.md) | Architecture spec — 7 REQs, 17 Goat Rodeo decisions |
| [`docs/v2/PRD.md`](docs/v2/PRD.md) | 31 features with FEAT-IDs and acceptance criteria |
| [`docs/v2/IMPLEMENTATION_PLAN.md`](docs/v2/IMPLEMENTATION_PLAN.md) | 8 phases, 60+ steps |
| [`docs/v2/BACKEND_STRUCTURE.md`](docs/v2/BACKEND_STRUCTURE.md) | SQLite schema, REST API, WebSocket protocol, agent protocols |
| [`docs/v2/TECH_STACK.md`](docs/v2/TECH_STACK.md) | Version-locked dependencies |
| [`docs/v2/TEST_PLAN.md`](docs/v2/TEST_PLAN.md) | 190+ test cases across 7 sections |
| [`docs/v2/DESIGN_SYSTEM.md`](docs/v2/DESIGN_SYSTEM.md) | Visual tokens for the dashboard |
| [`daemon/BUILD.md`](daemon/BUILD.md) | Build guide |

---

## How It Was Built

This project was designed and built by a human (Mike Boscia) coordinating three AI agents:

- **Claude** (Opus 4.6) — architecture, spec, Goat Rodeo, documentation, code review
- **Gemini** (Pro 2M) — research, twin review, competitive analysis
- **Codex** (GPT-5.2) — implementation, from scaffold to v0.1.0 release

The spec went through a 6-round Goat Rodeo (pressure test) before a single line of implementation code was written. 13 canonical documents were produced. Codex built the entire daemon from the documentation while Claude and Gemini continued refining the spec and researching competitors.

Total time from spec to shipped v0.1.0: **one session**.

---

## Prior Art & Acknowledgments

Triumvirate was informed by these open-source projects. Specific patterns borrowed are attributed inline in the source code.

| Project | What We Learned | License |
|---------|----------------|---------|
| [Temporal](https://github.com/temporalio/temporal) | Workflow engine patterns, crash recovery, event sourcing | Apache 2.0 |
| [Ruflo](https://github.com/ruvnet/ruflo) | Multi-model agent routing, cost optimization | Open source |
| [Clash](https://github.com/nicholasgasior/clash) | Real-time git worktree conflict detection | Open source |
| [swarms-rs](https://github.com/swarms-rs) | Rust agent lifecycle management | Open source |
| [Claude Agent Teams](https://docs.anthropic.com) | Worktree isolation, shared task list, peer-to-peer mailbox | Anthropic |
| [Flotilla / agentic-fleet-hub](https://github.com/UrsushoribilisMusic/agentic-fleet-hub) | Cross-model peer review, structured lessons ledger | Open source |
| [ensemble](https://github.com/michelhelsdingen/ensemble) | JSONL file-based message bus, tmux session management | MIT |
| [RunDiffusion Agents](https://github.com/rundiffusion/RunDiffusion-Agents) | YAML governance control plane | Apache 2.0 |
| [AgentsMesh](https://github.com/AgentsMesh/AgentsMesh) | gRPC+mTLS control plane, channel-based pub/sub | BSL-1.1 |

---

## Competitive Landscape

| | AgentsMesh | Flotilla | ensemble | RunDiffusion | **Triumvirate** |
|---|---|---|---|---|---|
| Language | Go + Next.js | Python | TS + Bash | Shell | **Rust** |
| Shared memory | No | PocketBase | No | No | **SQLite WAL** |
| Dynamic fleet | Yes | No | Yes | Per-tenant | **Yes** |
| Workflow engine | No | No | No | No | **Yes** |
| Cost tracking | No | No | No | No | **Yes (Langfuse)** |
| Lessons ledger | No | **Yes** | No | No | **Yes** |
| Single binary | No | No | No | No | **Yes** |

---

## Research

37+ research artifacts documenting the design process: [`research/`](research/)

Topics: multi-agent coordination landscape, CLI deep dives (Claude/Gemini/Codex), Toulmin argumentation, embedded workflow engines, Cedar policy, competitive analysis (AgentsMesh, Flotilla, ensemble, RunDiffusion).

---

## License

[FSL-1.1-ALv2](LICENSE) (Functional Source License 1.1, Apache 2.0 Future License)
