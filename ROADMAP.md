# Triumvirate Roadmap

**Last updated:** 2026-04-05
**Current version:** v0.1.0

---

## v1.0 — Daily Driver (Now → 2 weeks)

The goal: Triumvirate replaces the current inter-agent MCP server as the everyday tool for coordinating Claude, Gemini, and Codex.

| Item | Status | Description |
|------|--------|-------------|
| Core daemon | Shipped (v0.1.0) | Rust binary, 3 agent connectors, message fabric, web dashboard |
| Workflow engine | Shipped | SQLite event-sourced state machine, conversation + debate + fleet workflows |
| Fleet coordination | Shipped | Worktrees, shared task list, sequential merge, peer messaging |
| Prometheus metrics | Shipped | /metrics endpoint with per-agent histograms and counters |
| Langfuse integration | Shipped | Every agent turn traced with tokens, cost, latency |
| Cost attribution | Shipped | Per-turn, per-task, per-fleet dollar cost in dashboard |
| Mock CLIs | Shipped | Deterministic test doubles for all 3 agents |
| Cross-model peer review | Planned | Agents can't approve their own work (Flotilla pattern) |
| Lessons ledger (FEAT-031) | Planned | Machine-readable lessons with confidence decay |
| E2E tests with real CLIs | Planned | Acceptance gate against actual Claude/Gemini/Codex |
| Chaos testing | Planned | Kill agents mid-fleet, corrupt SQLite, disconnect WebSocket |
| Session resume | Planned | Kill daemon, restart, conversation continues at exact point |
| Doc comment cleanup | Planned | FEAT-IDs and /// comments on all public APIs |

---

## v1.5 — Hardened (Weeks 3-4)

The goal: You can run this 8 hours a day without babysitting it.

| Item | Description |
|------|-------------|
| Cedar governance enforced | Destructive ops blocked without dashboard approval |
| Quota battle-tested | 5-hour rolling windows, auto-fallback, no surprise throttling |
| Context window management | Automatic context rotation for long-running agents |
| Agent specialization | Different system prompts per instance ("you are an architect" vs "you are a reviewer") |
| Fleet templates | Save and reuse fleet compositions ("auth team", "research team") |
| OpenTelemetry distributed tracing | Full trace correlation across agents with GenAI semantic conventions |
| Graceful shutdown hardened | SIGTERM → save state → terminate agents → close SQLite. Zero data loss. |
| Dashboard v2 | Polished Svelte UI matching DESIGN_SYSTEM.md pixel-perfect |

---

## v2.0 — The Product (Month 2)

The goal: Other people can use this. Not just you.

| Item | Description |
|------|-------------|
| Installer | `curl -fsSL install.sh \| sh` — downloads binary, configures ~/.triumvirate, detects CLIs |
| Documentation for humans | Getting started guide, tutorial, FAQ — not just canonical docs for AI agents |
| Plugin system | Add new agent types without modifying the binary. Mistral, Llama, local Ollama models. |
| API mode first-class | Direct Anthropic/OpenAI/Google API integration with prompt caching. Not just CLI fallback. |
| Remote agents | Fleet members running on GCE, RunPod, Vast.ai — not just local machine |
| NATS upgrade | Swap Tokio channels for real NATS when cross-process messaging is needed for remote agents |
| Persistent fleet templates | Saved configurations: "auth team" = {claude: 2, codex: 2, gemini: 1} |
| Agent memory profiles | Per-agent-type memory injection — Codex gets implementation context, Gemini gets research context |

---

## v2.5 — Market-Driven Features (Month 2-3)

The goal: Build what the market is screaming for. These came from competitive research and developer pain point analysis (April 2026).

| Item | Description | Market Signal |
|------|-------------|---------------|
| Codebase indexing | Agents understand the whole repo — dependencies, imports, architecture. Think Pythia built into the daemon. | #1 most requested feature across every survey |
| Budget ceiling mode | "Spend max $5 on this fleet task." Daemon tracks cost real-time, pauses agents at budget limit. | Developers hate unpredictable token spend |
| Simulation/dry-run | Run fleet against mock CLIs first. Show estimated cost + time before burning real quota. "~$2.40, ~15 min. Go?" | Reduces fear of launching fleets |
| Agent output validation gates | Automated compile + test + lint between fleet member completion and merge. Machine verification, not just peer review. | Trust in AI output is #1 concern |
| VS Code extension | Fleet status, task assignment, agent output streaming inside the IDE. Not everyone wants a browser tab. | IDE integration is table stakes |
| Automated conflict resolution | Intelligent merge for simple conflicts (two agents add to same list). Human escalation only for architectural disagreements. | Current worktree merge is manual-only |
| Agent testing gym | Simulated environment to test agent behaviors and fleet interactions before production. | Enterprises need pre-deployment validation |

---

## v3.0 — Platform (Month 3+)

The goal: Triumvirate is infrastructure that teams and products build on.

| Item | Description |
|------|-------------|
| Multi-user | Multiple humans sharing the same daemon, each with their own fleets and permissions |
| Web deployment | Dashboard accessible from anywhere, not just localhost |
| Team workspaces | Shared memory across human team members + their agent fleets |
| Workflow marketplace | Share fleet templates, Cedar policies, workflow definitions, agent specializations |
| Cost optimization engine | Auto-route tasks to cheapest capable model. Research → Gemini (free). Implementation → Codex. Architecture → Claude. |
| A2A protocol compliance | Interoperate with Google's Agent-to-Agent protocol for external agent integration |
| Streaming API | External applications consume Triumvirate's fabric events via WebSocket/SSE |
| Self-hosting guide | Docker image, Kubernetes manifest, Terraform module for cloud deployment |
| Predictable pricing mode | Flat-rate billing option for teams. Not token chaos. |

---

## Principles

1. **Single binary stays single.** External dependencies are upgrades, not requirements. NATS is optional. Temporal is never coming back. The binary runs on its own.

2. **CLI-first, API-second.** Subscriptions are the primary path. API is the fallback. We maximize what users already pay for.

3. **Mechanical over magical.** No LLM summarization. No hallucinated session notes. Facts extracted from structured data, verified against git and tool results.

4. **Borrow, attribute, improve.** We study Ruflo, Clash, Flotilla, ensemble, AgentsMesh, RunDiffusion. We borrow patterns. We attribute everything. We build something better than any of them alone.

5. **The fleet gets smarter.** Lessons ledger, confidence decay, cross-model peer review. Every session teaches the fleet something. Mistakes don't repeat.

---

## Prior Art Being Tracked

| Project | Monitoring For |
|---------|---------------|
| Claude Agent Teams | New coordination primitives from Anthropic |
| AgentsMesh | Enterprise features we might need |
| Flotilla | Lessons ledger improvements, peer review patterns |
| Ruflo | Multi-model routing optimizations |
| ensemble | Simplicity patterns we can learn from |
| RunDiffusion | Governance model evolution |
| OpenAI Swarm | If it matures beyond experimental |
| Google ADK | Gemini-native agent composition |
