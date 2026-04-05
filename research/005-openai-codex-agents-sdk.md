# Research 005: OpenAI Codex & Agents SDK

**Query:** OpenAI Codex agent SDK API 2025-2026 programmatic access

## Key Findings

### Agents SDK
- Released March 2025 (Python), July 2025 (TypeScript)
- Replaced experimental "Swarm" framework
- Production-grade: guardrails, tracing, observability
- **Provider-agnostic** — can use non-OpenAI models

### Codex Integration
- Codex CLI can be exposed as **MCP server** for programmatic access
- Enables multi-agent dev workflows: refactoring, feature rollouts, testing
- Subagent workflows supported — main agent spawns specialized subagents in parallel
- GPT-5.2-Codex model (Dec 2025)

### Architecture
- Agent loops: prompt → tool calls → reasoning → loop completion
- Built-in tools: WebSearch, FileSearch, CodeInterpreter, ImageGeneration
- Handoffs between specialized agents
- Gating logic between agent stages

## What This Means for Triumvirate
Codex can be controlled via MCP — we don't need to shell out to `codex` CLI. The Agents SDK provides the programmatic interface we've been missing. We should:
1. Expose Codex as MCP server
2. Use the Agents SDK for structured task submission
3. Get real error handling and observability for free

## Sources
openai.com, medium.com, datasciencedojo.com, analyticsvidhya.com, sidbharath.com, gurusup.com, reddit.com, intuitionlabs.ai
