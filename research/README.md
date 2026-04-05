# Triumvirate v2 Research Index

**Date:** 2026-04-04
**Status:** Deep research phase — first principles rebuild
**Session:** Crystallize failure → research → spec

## Research Artifacts

| # | Title | Key Finding |
|---|-------|-------------|
| 001 | Multi-Agent Council Landscape | A2A protocol, heterogeneous models, peer-to-peer patterns |
| 002 | A2A Protocol Deep Dive | HTTP/JSON-RPC + SSE, Agent Cards, complementary to MCP |
| 003 | Go MCP Server | Official Go SDK exists, community SDKs battle-tested |
| 004 | Claude Agent SDK | Agent Teams, direct messaging, 90% improvement over single-agent |
| 005 | OpenAI Codex Agents SDK | Codex as MCP server, provider-agnostic, guardrails + tracing |
| 006 | WUPHF Agent Office | Slack-like channels for AI agents, debate as feature |
| 007 | Shared Memory Architectures | Knowledge graphs, vector DBs, context fabric — not per-agent silos |
| 008 | Agent Debate & Adversarial Truth | Structured argumentation, constitutional AI, MindMesh 7-agent debate |
| 009 | Gemini's Vision | Persistent daemon, embedded NATS, blackboard, speculative execution |
| 010 | Codex's Vision | 6-layer stack, Temporal, OPA governance, constitutional parliament |
| 011 | NATS Embedded in Go | Single binary with pub/sub, JetStream for persistence, zero network overhead |
| 012 | Toulmin Argumentation Schema | JSON schema for structured debate: claim + data + warrant + rebuttal |
| 013 | Temporal Workflow Engine | Durable execution, retry, compensation, Go SDK primary |
| 014 | Tree-sitter AST Code Ops | Edit code via AST nodes not text, incremental parsing, Go bindings |
| 015 | OPA Policy Engine | (pending write) Agent governance, Rego policies, embeds in Go |
| 016 | OpenTelemetry Observability | (pending write) Distributed tracing for agent tool calls, GenAI semantic conventions |
| 017 | CRDTs for Collaborative Editing | (pending write) Yjs/Automerge, lock-free concurrent editing, arxiv paper on multi-agent CRDTs |
| 018 | Speculative Execution | (pending write) Branch prediction for code, pre-compute multiple paths |

## Convergent Architecture (Both Twins Agree)

**Core:** Go binary (`triumvirated`) — persistent system daemon
**Messaging:** Embedded NATS with JetStream (pub/sub channels)
**Orchestration:** Temporal.io (durable workflows, retry, compensation)
**Protocols:** A2A (agent↔agent) + MCP (agent↔tools) + gRPC (internal)
**Memory:** Postgres + pgvector (shared context fabric, not per-agent silos)
**Governance:** OPA/Rego (policy-as-code for agent permissions)
**Observability:** OpenTelemetry (distributed tracing per agent turn)
**Debate:** Toulmin model JSON schema (structured argumentation)
**Code Ops:** Tree-sitter AST (phase 2+)
**Collaboration:** CRDTs (agents edit same files safely)

## Agent Roles (Redefined)
- **Claude (Opus 4.6):** Architect + risk analyst + structured reasoning
- **Gemini (Pro 2M):** Repository swallower + synthesizer + QA oracle
- **Codex (GPT-5.2):** Execution marshal + protocol guardian + failure surgeon

## Wild Ideas Worth Exploring
1. Speculative execution — pre-compute code paths before user confirms
2. Constitutional Parliament — parliamentary procedure for agent decisions
3. Firecracker microVMs — sandboxed execution in <50ms
4. WUPHF-style channels — topic-based agent communication (#review, #debug, #research)

## What Triggered This
nudge-reaper catastrophe (2026-04-03): 155 tests, zero E2E, wrong resume commands, hallucinated session notes, reaper killed its own work sessions. Crystallized into reality-check skill + mx-reality-check matrix. Led to "rebuild the foundation right."
