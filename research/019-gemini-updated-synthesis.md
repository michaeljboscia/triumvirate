# Research 019: Gemini's Updated Synthesis (Post-Research)

## Key New Patterns

### AST-Gated CRDT
Agents write to Yjs CRDT buffer → Tree-sitter validates AST → only valid code flushes to disk. Solves the semantic conflict problem.

### OPA-Governed NATS Bus
OPA embedded as NATS middleware interceptor. Every message evaluated against Rego policies in microseconds BEFORE delivery. Example: `deny if agent == "codex" and action == "delete_file" and risk_level == "high" without human_approval`.

### Pragmatic Speculation (The Pivot)
Full speculative execution deferred. Instead: Gemini pre-fetches relevant docs and constraints the moment user starts typing, populates NATS topic before Claude even starts designing.

### Gemini as Chief Inquisitor
Primary role: FALSIFY Claude's claims. Cross-reference against 2M-token codebase + docs. Publish Toulmin Rebuttals to prevent bad code from ever being written.

## MVP Phasing
1. Core Daemon (Go + NATS + OPA + OTel)
2. Toulmin Event Bus (schemas, topics, policy enforcement)
3. Tree-sitter/CRDT Memory Fabric (virtual workspace)
4. Temporal Agent Loop (Parliament Saga workflow)

## Key Quote
"We are taking the development environment away from the IDE and giving it to the Daemon. The IDE just becomes a dumb terminal viewing the Tree-sitter validated CRDT buffer that the Triumvirate is constantly debating and mutating in real-time."
