# Research 007: Shared Memory & Persistent State for Multi-Agent Systems

**Query:** How multiple AI agents maintain shared context and long-term memory

## Memory Architecture (Mirrors Human Cognition)
1. **Working Memory** — ephemeral, per-session context (our current context window)
2. **Short-term Memory** — Redis/cache for recent interactions (our MEMORY.md system)
3. **Long-term Memory** — persistent across sessions:
   - **Episodic** — specific interaction histories (our session logs)
   - **Semantic** — factual knowledge and relationships (our skills/lessons)
   - **Procedural** — learned behaviors and workflow patterns (our skills)

## Shared Context Mechanisms
- **Knowledge Graphs** — common semantic foundation, agents modify/query shared graph
- **"Agentic Mesh"** — persistent shared workspace with enforced schemas
- **MCP as memory interface** — translates agent intentions into database operations
- **Context Engineering** — explicit lifecycle management: ingest → normalize → index → retrieve → compose → evaluate → persist

## What We Have vs. What Exists
| What We Have | What's State of Art |
|---|---|
| MEMORY.md flat files | Vector DB + Graph DB + Relational |
| Per-agent memory silos | Shared knowledge graphs |
| Session logs as text | Episodic memory with semantic search |
| Skills as markdown | Procedural memory with retrieval |

## Key Insight
Our agents don't share memory. Claude has MEMORY.md, Gemini has its CLI state, Codex has threads. There's no shared context fabric. When Claude asks Gemini a question, Gemini doesn't know what Claude has been working on unless we explicitly pass it.

**The triumvirate needs a shared memory layer — not per-agent silos.**

## Sources
sparkco.ai, tigerdata.com, oracle.com, mindra.co, amazon.com, mongodb.com, arxiv.org
