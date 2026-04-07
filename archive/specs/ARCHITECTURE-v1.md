# Triumvirate v2 — Architecture

> **Note:** This document supersedes the original v1 architecture (Node.js MCP server). v1 was retired in April 2026 after the nudge-reaper catastrophe. See `SPEC.md` for the full story.

## Overview

Triumvirate is a single Rust binary (`triumvirate-agentd`) that coordinates multiple AI coding agents (Claude, Gemini, Codex) as a dynamic fleet.

```
Human (browser at :8080)
  |
  v
triumvirate-agentd
  |
  +-- Agent Pool ---------- N persistent subprocesses per type
  |   |                     (Claude stream-json, Gemini ACP, Codex MCP)
  |   +-- Supervisor ------- auto-restart with backoff
  |   +-- Health Monitor --- liveness + readiness checks
  |
  +-- Message Fabric ------- Tokio broadcast/mpsc/watch channels
  |                          (NATS-shaped topics for future swap)
  |
  +-- Workflow Engine ------ SQLite event-sourced state machine
  |   |                     (informed by Temporal source, Apache 2.0)
  |   +-- ConversationWorkflow
  |   +-- DebateWorkflow
  |   +-- FleetWorkflow
  |
  +-- Fleet Coordinator ---- git worktrees + contracts-first + task list
  |   |                     (informed by Claude Agent Teams, Ruflo, Clash)
  |   +-- Worktree Manager
  |   +-- Shared Task List (SQLite, dependency tracking)
  |   +-- Sequential Merge
  |   +-- Peer Messaging
  |
  +-- Memory --------------- SQLite WAL (memories, sessions, decisions, lessons)
  +-- Stenographer --------- Mechanical extraction (no LLM summarization)
  +-- Governance ----------- Cedar policy engine (Rust-native)
  +-- Observability -------- Prometheus /metrics + Langfuse + OpenTelemetry
  +-- Dashboard ------------ axum + Svelte + Tailwind (rust-embed)
```

## Key Design Decisions

All decisions were made through a 6-round Goat Rodeo (pressure test) with twin review from Gemini and Codex. Full decision ledger: `research/033-goatrodeo-r2-decision-ledger.md`.

| Decision | What | Why |
|----------|------|-----|
| GR2-D1 | Purpose-built workflow engine | Temporal can't embed in Rust. We need 5% of what Temporal does. Source is Apache 2.0 — read, learn, adapt. |
| GR2-D2 | Per-agent native JSON adapters | PTY breaks Gemini (ANSI noise), unnecessary for Claude (stream-json exists). Each CLI speaks structured JSON natively. |
| GR2-D3 | Tokio channels, not NATS | NATS can't embed in Rust. Tokio channels are zero-overhead for in-process routing. Topic enum maps to NATS subjects if we ever need the swap. |
| GR4-D3 | Worktrees + contracts + task list | Convergent pattern from Claude Agent Teams, Ruflo, Clash, and community. Applied cross-model. |
| REQ-7 | Dynamic multi-agent fleet | The killer feature. N instances of any type, on demand. Nobody else has this with cross-model coordination. |

## Canonical Documentation

All design docs live in `docs/v2/`. The spec is at the repo root (`SPEC.md`).
