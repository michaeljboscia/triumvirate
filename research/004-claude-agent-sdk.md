# Research 004: Claude Agent SDK — Multi-Agent Orchestration

**Query:** Anthropic Claude Agent SDK multi-agent parallel agents communication

## Key Capabilities

### Agent SDK (formerly Claude Code SDK, rebranded Sept 2025)
- General-purpose framework for autonomous agents
- Orchestrator-worker architecture (Research feature)
- Subagents operate in isolated context windows, work in parallel
- 90.2% improvement over single-agent systems in research tasks

### Agent Teams (Feb 2026, Opus 4.6)
- Fully independent Claude Code instances
- **Direct inter-agent messaging** — teammates can communicate with lead and each other
- Shared task boards and messaging channels
- This is what we're ALREADY using but through the CLI hack

### Integration
- Uses MCP for tool access + A2A for agent communication
- Microsoft Agent Framework compatibility — can compose with Azure OpenAI agents
- Subagents can't spawn their own subagents (limitation)

### Cost Reality
- Multi-agent systems use ~15x more tokens than single-agent
- Tasks must justify the cost

## What This Means
We don't need to reinvent the agent layer. Claude Agent SDK + Agent Teams are the native primitives. The triumvirate should be a THIN ORCHESTRATION LAYER that coordinates these primitives across providers, not a replacement for them.

## Sources
anthropic.com, zenml.io, constellationr.com, shinzo.ai, ksred.com, claude.com, timdietrich.me, medium.com
