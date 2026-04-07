# Research 002: Google A2A (Agent-to-Agent) Protocol

**Query:** Google A2A protocol specification, how it works, SDK, vs MCP

## THIS IS THE MISSING PIECE

A2A is the horizontal communication layer. MCP is the vertical tool layer. We have MCP. We need A2A.

## How A2A Works
- **Client-server model** over HTTP/HTTPS + JSON-RPC 2.0
- **Agent Cards** — JSON metadata at `/.well-known/agent.json` advertising capabilities
- **Tasks and Messages** — structured JSON with "parts" (text, files, multimodal, structured data)
- **Communication patterns:**
  - `tasks/send` — synchronous request/response (quick tasks)
  - `tasks/sendSubscribe` — SSE streaming for long-running tasks
- **Opaque agents** — internal implementation hidden, only protocol matters
- **Security** — HTTPS + modern TLS, auth/authz built in

## A2A vs MCP
| | A2A | MCP |
|---|---|---|
| **Direction** | Horizontal (agent↔agent) | Vertical (agent↔tools) |
| **Purpose** | Agents communicate and delegate to each other | Agent connects to tools, memory, data |
| **Analogy** | Team communication | Individual agent's toolkit |

**They are COMPLEMENTARY.** A robust system uses MCP for tool access and A2A for agent-to-agent communication.

## SDK
- Official Python SDK available
- Native A2A support in Google ADK (Agent Development Kit)
- Open source under Linux Foundation
- **No official Go SDK yet** — opportunity or build from spec

## What This Means for Triumvirate
Our current approach (shell out to CLI, parse stdout) is a hack. A2A gives us:
1. Agent discovery via Agent Cards
2. Structured task submission with progress tracking
3. Streaming responses via SSE
4. Health checks built into the protocol
5. Standard that Google, Anthropic, and OpenAI are all adopting

## Sources
cybage.com, googleblog.com, trickle.so, platformengineering.com, huggingface.co, a2a-protocol.org, descope.com, clarifai.com, stride.build, merge.dev, deeplearning.ai
