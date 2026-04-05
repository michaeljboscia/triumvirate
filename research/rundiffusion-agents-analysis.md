# RunDiffusion Agents — Analysis

**Repo:** https://github.com/rundiffusion/RunDiffusion-Agents
**License:** Apache-2.0 | **Stars:** 20 | **Forks:** 5 | **Commits:** 6 (public since 2026-03-25)
**Languages:** Shell (45%), JavaScript (24%), TypeScript (15%), Python (12%), Dockerfile (4%)
**Status:** Production at RunDiffusion since Feb 2026. Runs their entire fleet on a single $600 Mac Mini M4.

## Architecture

Single Docker Compose stack per host. Each tenant gets its own container with Traefik routing. The core service (`services/rundiffusion-agents/`) bundles OpenClaw (their agent runtime), a dashboard, Filebrowser, and terminal launchers for each agent type (Claude, Codex, Gemini, Hermes).

## YAML Control Plane

One YAML file governs the entire fleet. Per-tenant config controls:
- **Version pins** — lock each tenant to a specific OpenClaw release
- **Secret injection** — API keys at host level, never in images
- **Model governance** — allowlists, primary model, fallback chains
- **Agent-to-model binding** — which model powers each tenant's operator
- **Route-level feature flags** — enable/disable Gemini, Claude, Codex per tenant
- **Provider policy** — toggle auth hydration per provider

Four config layers with override precedence (details in docs/configuration.md).

## Multi-Model Routing

Models are governed per-tenant: an `allowed` list (allowlist), a `primary` default, and a `fallbacks` array for chain-of-fallback. Each agent within a tenant can be bound to a specific model. Routes (Gemini, Claude, Codex, Hermes) are feature-flagged independently.

## Dashboard / UI

Yes. A built-in dashboard at `services/rundiffusion-agents/dashboard/` served by `dashboard_server.js`. Also includes Filebrowser for file management and full terminal access per agent.

## Fleet Governance

Agents are self-managing — they create, repair, upgrade, and audit other agents. Two Claude Code skills ship with the repo: `rundiffusion-host-agent-manager` (multi-tenant with Traefik) and `rundiffusion-standalone-agent-manager` (single-agent). These skills handle deployment, tenant creation, health checks, and troubleshooting. Reconciliation scripts (`reconcile_openclaw_state.js`, `reconcile_filebrowser_permissions.js`) keep runtime state consistent.

## Relevance to Triumvirate

This is the closest open-source analog to what we're building. Key differences: they use Shell/JS (we're going Go + NATS + Temporal + OPA), their YAML control plane is simpler than our planned OPA policy engine, and they're Docker-compose-native vs our planned Kubernetes-first approach. Their agent-manages-agents pattern is worth studying.
