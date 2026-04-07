# NOTICE

Triumvirate v2 daemon includes implementation patterns informed by open-source prior art.

## Referenced Projects

| Project | Source | License | What Was Informed |
|---------|--------|---------|-------------------|
| Temporal | `temporalio/temporal` | Apache 2.0 | Event-sourced workflow persistence, crash recovery, retry with backoff, compensation patterns |
| Ruflo | `ruvnet/ruflo` | Open source | Multi-model agent routing, cost-optimized model selection, swarm coordination |
| Clash | `nicholasgasior/clash` | Open source | Real-time git worktree conflict detection between parallel agents |
| swarms-rs | `swarms-rs` | Open source | Rust agent lifecycle management, supervisor patterns |
| Claude Agent Teams | Anthropic | Proprietary | Git worktree isolation, shared task list with dependency tracking, peer-to-peer mailbox messaging |
| Flotilla / agentic-fleet-hub | `UrsushoribilisMusic/agentic-fleet-hub` | Open source | Cross-model peer review as mandatory gate, structured lessons ledger with confidence scores, MISSION_CONTROL.md shared state pattern |
| ensemble | `michelhelsdingen/ensemble` | MIT | JSONL file-based message bus with fcntl locking, tmux session management for agent subprocesses |
| RunDiffusion Agents | `rundiffusion/RunDiffusion-Agents` | Apache 2.0 | YAML governance control plane, agent-manages-agents pattern, per-tenant model policy enforcement |
| AgentsMesh | `AgentsMesh/AgentsMesh` | BSL-1.1 | gRPC+mTLS control plane architecture, channel-based pub/sub for terminal streaming (studied, not used in production per BSL-1.1 terms) |

## Attribution Notes

This project does not vendor source code from any of the projects above.
Where patterns were adapted, implementation was re-authored in Rust for this codebase.
Inline attribution comments (e.g., `// Adapted from Ruflo's cost-router`) appear at specific adaptation points in the source.
