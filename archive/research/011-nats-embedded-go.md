# Research 011: NATS Embedded in Go — The Nervous System

**Confirmed:** NATS server embeds directly into a Go binary. No external process. No Docker. Single binary deployment.

## Key Facts
- `github.com/nats-io/nats-server/v2/server` — embed the server
- `github.com/nats-io/nats.go` — client library
- In-process communication without network interface — zero network overhead
- JetStream for persistence and at-least-once delivery
- Millions of messages per second — performance negligible vs external NATS
- Hierarchical subjects with wildcards (`events.agent.*`, `debate.>`)
- Queue groups for load balancing across subscribers

## Why This Is Perfect for Triumvirate
- Single `triumvirated` binary boots NATS internally
- Agents publish to topics: `debate.architecture`, `review.code`, `task.execute`
- JetStream gives us message history (debate replay, audit trail)
- No network hops for local agent communication
- Subjects map directly to our "channels" concept

## Sources
karanpratapsingh.com, dev.to, reddit.com, medium.com, natsbyexample.com, go.dev, nonstopio.com
