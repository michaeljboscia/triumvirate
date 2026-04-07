# Research 016: OpenTelemetry — Agent Observability

**Every agent turn, every tool call, every debate exchange — traced.**

## What OTel Gives Us
- Distributed tracing across agent interactions (parent-child spans)
- GenAI semantic conventions: token counts, cost, model info, latency
- Session ID attributes for correlating multi-agent task activities
- Go SDK with manual instrumentation for custom agent logic
- OpenLLMetry: specialized toolkit for LLM observability
- Integrates with Langfuse (which we already use!)

## How It Maps to Triumvirate
```
[DebateWorkflow] ← root span
  [Claude: propose_architecture] ← child span (tokens: 4200, cost: $0.12, latency: 3.2s)
  [Gemini: challenge_warrant] ← child span (tokens: 6800, cost: $0.00, latency: 1.8s)
  [Codex: verify_implementation] ← child span (tokens: 3100, cost: $0.08, latency: 4.1s)
  [Vote: 2-1 accept] ← child span
```

Every decision has a trace. Every failure has a path. No more "what happened?"

## Sources
dynatrace.com, networkworld.com, oneuptime.com, opentelemetry.io, cisco.com, coralogix.com, traceloop.com, langfuse.com
