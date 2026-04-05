# AgentsMesh Analysis — 2026-04-04

**Repo:** https://github.com/AgentsMesh/AgentsMesh
**Stars:** 1,267 | **Forks:** 121 | **Commits:** 914 | **Open Issues:** 33
**Language:** Go (backend/runner/relay) + Next.js/TypeScript (frontend)
**License:** BSL-1.1 (change date 2030-02-28, converts to GPL-2.0-or-later)
**Created:** 2026-02-28

## Architecture

Control plane / data plane split. Four Go services + two Next.js frontends:

- **Backend** — Go (Gin + GORM). Auth, org/team management, pod lifecycle, task management. PostgreSQL + Redis + MinIO.
- **Runner** — Self-hosted Go daemon on your infra. Connects to Backend via gRPC+mTLS, to Relay via WebSocket. Runs agents in isolated PTY sandboxes.
- **Relay** — Terminal relay cluster. Low-latency WebSocket pub/sub between runners and browsers.
- **Web** — Next.js dashboard. Web terminal, kanban board, topology visualization.
- **Web-Admin** — Internal admin console.

Coordination happens through "channels" and "pod bindings" — agents in AgentPods communicate via a channel abstraction, and the topology is visualized in real-time.

## Fleet Support (N instances of same agent type)

Yes. AgentPods are remote AI workstations — you spin up multiple concurrent pods, each running any supported agent (Claude Code, Codex CLI, Gemini CLI, Aider, OpenCode, or custom). Multiple pods of the same agent type is the core use case. Self-hosted runners can be deployed across your own infrastructure.

## Dashboard

Yes. Full Next.js web console with:
- Web terminal (streaming via Relay)
- Kanban task board with ticket-pod binding and PR/MR integration
- Real-time collaboration topology visualization
- Admin console (separate app) for user/org/runner management and audit logs

## Shared Memory

Not explicitly addressed in the README or project structure. Coordination is through channels and pod bindings — a pub/sub messaging layer — not a shared memory store. Each AgentPod runs in its own isolated PTY sandbox. No evidence of a shared context/memory system between agents.

## File Conflicts

Handled via Git worktree isolation. Each AgentPod gets its own Git worktree, so agents work on separate branches/directories. Conflicts are resolved through standard Git merge/PR workflows, not a custom conflict resolution layer.

## Comparison to Triumvirate

AgentsMesh is an infrastructure/platform play — remote workstations, gRPC control plane, enterprise multi-tenancy. Triumvirate is a local-first coordination protocol with NATS + Temporal + OPA. Key differences:
- AgentsMesh: cloud-hosted pods, web dashboard, enterprise RBAC/SSO, BSL-1.1 license
- Triumvirate: local daemons, CLI-first, policy engine, open protocol design
- AgentsMesh has no shared memory or policy engine — coordination is channel-based pub/sub
- AgentsMesh does not appear to have deterministic workflow orchestration (no Temporal equivalent)
