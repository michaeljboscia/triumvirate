# Triumvirate

[![Rust CI](https://github.com/michaeljboscia/triumvirate/actions/workflows/rust.yml/badge.svg)](https://github.com/michaeljboscia/triumvirate/actions)
[![Version](https://img.shields.io/github/v/release/michaeljboscia/triumvirate)](https://github.com/michaeljboscia/triumvirate/releases)
[![License: FSL-1.1-ALv2](https://img.shields.io/badge/License-FSL--1.1--ALv2-blue.svg)](LICENSE)

**Five AI agents. One daemon. A methodology for building software with all of them at once, and a trust layer that checks whether they actually did the work.**

Claude, Codex, Antigravity (Gemini), Grok and DeepSeek, each with different strengths, working on the same codebase, coordinated by a single Rust daemon, visible in real time from inside your editor.

The hard part of multi-agent development is not dispatch. It is knowing whether a peer that said "looks good" read anything at all. Triumvirate answers that mechanically: a review that opened no files is rejected, a reviewer that skimmed one line is rejected, and code can be validated by a different agent that writes the tests without ever seeing the implementation.

## Quick Start

```bash
git clone https://github.com/michaeljboscia/triumvirate
cd triumvirate/daemon
cargo build --release && cargo run --release
```

That's it. One binary. No Docker. No NATS. No cloud services. Register it in Claude Code:

```bash
# Add to ~/.claude.json under mcpServers:
"triumvirate": { "command": "/path/to/triumvirate", "args": ["mcp"] }
```

Then from any Claude session: `spawn a Gemini session called 'research'`

> **Requirements:** Rust 1.82+ and at least one agent CLI: [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Codex CLI](https://github.com/openai/codex), [Antigravity](https://antigravity.google) or the [Gemini CLI](https://github.com/google-gemini/gemini-cli), or the [grok CLI](https://x.ai). DeepSeek needs no CLI: it is reached over HTTP.
>
> Works with one agent or with all of them. The parts that need more than one say so: peer review
> needs a second agent, and blind validation needs one that is not the agent that wrote the code.

---

## The Problem

You're using Claude to write code. You want Gemini to research something. You want Codex to implement a plan in parallel. So you open three terminal windows, copy-paste context between them, lose track of which one is doing what, and eventually something goes sideways because nobody can see what anyone else is doing.

Triumvirate replaces the terminal windows. One daemon holds the context, so the agents can actually see each other work.

---

## What It Does Today

**The daemon:** a Rust MCP server that spawns and manages persistent agent sessions.

```
You:     "spawn a Gemini session called 'research'"
Claude:  → spawn_session(agent: "gemini", session_name: "research")
         ✓ Session research spawned

You:     "ask research to analyze our auth middleware"
Claude:  → ask_session(session_name: "research", prompt: "...")
         → Gemini: turn started
         → Gemini: calling read_file (src/middleware/auth.rs)
         → Gemini: calling read_file (src/middleware/jwt.rs)
         → Gemini: generating response
         → Gemini: responded (12,847 in / 1,203 out / 8,400 cached, 2 tools, 4.1s)
         "The auth middleware has three issues..."

You:     "spawn a Codex session and have it fix what Gemini found"
```

Persistent sessions. Live streaming. Cross-model coordination. From inside Claude, with no window switching.

**The methodology:** skills that use the daemon to run structured multi-agent processes.

| Skill | What It Does |
|-------|-------------|
| **`/goatrodeo`** | Multi-round spec review. Spawns Gemini and Codex as adversarial reviewers. They interrogate your spec, auto-resolve what they agree on, and surface only what needs human judgment. Produces battle-tested requirements with traceable REQ-IDs. |
| **`/postrodeo`** | Post-build retrospective. Audits what was built against what was specced. Spawns twins to review code diffs. Produces a completion matrix, deviation analysis, and lessons. |
| **`/design-goatrodeo`** | Design-specific variant. Pressure-tests visual specs, UX flows, and information architecture. |

**The verification:** blind validation, when the thing being checked is code.

```
You:     "blind validate the worktree for task-7"
Claude:  → blind_validate(impl_worktree: ".triumvirate/worktrees/task-7",
                          contract: "...", base_ref: "abc123",
                          worker_agent: "codex", package_dir: "daemon")

         → claude selected as validator (not codex: an agent cannot validate its own code)
         → validator works in /tmp/triumvirate-blind-validator/<job>/v
           (the implementation is not on its disk)
         → writes tests/blind_validation.rs from the contract alone
         → tests run against the worktree      : 4 passed
         → same tests run against the base ref : 4 FAILED
         → nothing else broke, nothing deleted

         ✓ accepted: red before the change, green after
```

The verdict is a test run, not an opinion. A validator that never wrote a test, wrote tests that
were already green, or read outside its own directory is refused rather than believed.

**The operating environment:** a starter kit that wires all the agents together.

```bash
cd starter-kit && ./install.sh
```

Hooks, configs, skills, and session notes for Claude, Codex and Antigravity. Plus a local stenographer that captures session notes via Ollama, at zero cloud cost.

---

## The Loop

This is how software gets built with Triumvirate:

```
         ┌─────────────────────────────────────────┐
         │                                         │
    Spec ──→ /goatrodeo ──→ Implementation ──→ /postrodeo
         │   (Gemini +       (Codex builds      (twins audit   │
         │    Codex review    from the spec)      the code)     │
         │    the spec)                                         │
         │                                         │
         └──── lessons feed back into next cycle ──┘
```

The goatrodeo runs *through the daemon*. It spawns peers as daemon sessions to review your spec. The postrodeo does the same for code review. The daemon is the infrastructure the methodology runs on.

Since the trust layer landed, the loop has a fourth step that is not optional in practice:
nothing is committed until the panel has seen it. That is not a rule the daemon enforces on a
human, it is a working practice, and it exists because there is no CI here yet. The panel is the
only gate.

**This project was built using this loop.** The Flow State feature (live agent streaming) went through a 4-round goatrodeo: 27 requirements, 29 auto-resolves, 5 human decisions, all running through the daemon. Then Codex implemented all 6 phases from the resulting spec while the human took a nap.

The tool that coordinates agents was built by agents coordinating through the tool. It is recursion with a paycheck.

---

## What's Shipped

### Daemon (`daemon/`)

| Feature | What It Does |
|---------|-------------|
| **Persistent sessions** | Spawn named sessions for any of the five agents that survive across requests. Resume conversations. |
| **Live streaming** | See tool calls, file reads, commands, and response generation as they happen, not after. |
| **Stuck detection** | Catches agents that idle >60s, loop the same tool >5x, or freeze. Surfaces it immediately. |
| **Token visibility** | Every response includes input/output/cached token counts, duration, and self-reported cost where the CLI gives one. |
| **Shared memory** | Agents read and write to a shared key-value store. Decisions persist across sessions. |
| **Scratchpad** | Quick cross-agent notes. Agent A writes, Agent B reads. |
| **Outbox** | Event log of every agent interaction: who asked what, who responded, when. |
| **Fallback outbox** | If an agent fails, the request is saved to disk. Nothing is lost. Retry later. |
| **Verbosity control** | `quiet`, `standard`, `detailed`, `raw`: choose your noise level. |

### The trust layer (`daemon/`)

The part that makes multi-agent review mean something. Each of these exists because a peer
review passed when it should not have, and the incident is recorded in the source at the point
of the fix.

| Gate | What It Does | Why |
|------|-------------|-----|
| **Sight gate** | A dispatch marked `require_sight` is REJECTED if the agent made zero tool calls. | On 2026-09-01 three peers were given filesystem access to review one brief. One made zero tool calls, graded nine citations from memory, and opened with "the claims below were subjected to rigorous sourcing". A human caught it by noticing the output had no links. |
| **Named sources** | `required_sources` demands a successful READ of specific paths. A search is not a read. | Counting tool calls passes on one `pwd`. Naming the sources turns "did it do anything" into "did it request the evidence". |
| **Whole-file reads** | A named source is only satisfied by a read of the WHOLE file. `head`, `tail`, `cut`, a pager, or a read carrying `limit`/`offset` does not count. | A one-line peek satisfied the gate and returned the proof token without the work ever entering the model's context. |
| **Parser allowlist** | Only parsers verified to record tool calls can satisfy the gate. An unverified parser fails closed. | A gate that cannot see tool calls cannot tell "did not look" from "cannot report". Blaming the agent for a blind instrument is its own defect. |
| **Mandatory peer review** | `TRIUMVIRATE_REQUIRE_PEER_REVIEW=1` dispatches a real reviewer and blocks the turn on REJECT or on an unreadable verdict. | It used to write "approved" to a database row and dispatch nothing. It was a rubber stamp that had never reviewed anything. |
| **Proof of read** | The artifact is written to disk with a daemon-minted nonce on the last line, and is NOT pasted into the prompt. The reviewer must return the nonce. | A reviewer that reads only the first lines cannot produce it. |
| **Verdict authority** | An `approve` requires the review row to be `in_progress` and the submitter to be the assigned reviewer. A review the daemon is conducting is not writable by any client. | Naming the assigned reviewer in a request body is a claim, not an identity. |
| **Blind validation** | A DIFFERENT agent writes tests from the contract, in a directory that does not contain the implementation, and they are run against both the worktree and the pre-change tree. | Every other gate proves a reviewer LOOKED. None proves it JUDGED. On code you can: the tests run, and the answer is an exit code rather than an opinion. |

Blind validation states its own limit rather than overselling it: it catches a tautology written
against existing code, it does not catch a weak test written against a new API, and nothing in
it does. That would need a mutation run, and it is not built.

### Skills (`skills/`)

| Skill | Lines | What It Does |
|-------|-------|-------------|
| `/goatrodeo` | ~900 | Industrialized spec review. Interrogation rounds, live research, twin review, auto-resolve, decision ledger. |
| `/postrodeo` | ~600 | Build retrospective. Completion matrix, deviation analysis, Layer 6 semantic check, twin code review. |
| `/design-goatrodeo` | ~500 | Design spec variant. Visual standards, UX flows, information architecture. |

### Starter Kit (`starter-kit/`)

Full installer for multi-agent development:
- **Claude:** hooks (session lifecycle, token gating, artifact protection), skills, CLAUDE.md
- **Codex:** hooks (session recovery, pre-compact), config, AGENTS.md
- **Gemini:** hooks (session recovery, pre-compact), GEMINI.md
- **Stenographer:** local session notes via Ollama (zero cloud cost)
- **MCP server registration:** wires agents to communicate through the daemon

### Stenographer (`stenographer/`)

Standalone Python tool. Reads agent transcripts, feeds them to a local Ollama model, and appends narrative session notes. Triggered automatically by hooks when transcript growth crosses a threshold.

---

## Architecture

```
triumvirate-daemon (single Rust binary, 13 crates)
│
├── MCP Server (rmcp): 56 tools
│   Sessions, ABE dispatch, fleet, knowledge, review, blind validation, tokens
│
├── HTTP API (axum): REST + Prometheus + WebSocket
│   Same tools as REST endpoints, plus /metrics, /ws, /api/tokens/*
│
├── Trust layer
│   ├── Sight gate          : require_sight, required_sources, whole-file reads
│   ├── Peer review engine  : real dispatch, verdict authority, inflight cap
│   └── Blind validation    : a different agent writes the tests, blind, and they run
│
├── ABE (Autonomous Build Enforcement)
│   ├── Fleet dispatch: N workers in isolated git worktrees
│   ├── Worker argv per agent: codex, claude and grok can each take a worktree
│   ├── Task tracker: state machine with 4-signal completion detection
│   ├── Contract enforcement: file scope, commit format, test gates
│   └── Failure classification: auto-retry with escalation
│
├── Token Economics (token-economics crate)
│   SQLite storage, session scanner, cost attribution by build/session/task
│
├── Agent Adapter (agent-adapter crate)
│   ├── CodexExecParser     : exec --experimental-json JSONL
│   ├── GeminiStreamParser  : stream-json NDJSON
│   ├── AgyStreamParser     : Antigravity stream-json
│   ├── GrokStreamParser    : grok streaming-json
│   ├── ClaudeStreamParser  : claude stream-json, with tool calls
│   └── StuckDetector       : idle, loop, freeze detection
│
├── Daemon Core: metrics, observability bus, session lifecycle
│
└── Fallback Outbox: disk-persisted retry queue
13 crates. 799 tests passing offline, plus 96 opt-in tests behind live-agent flags. Zero external services. Your laptop is the datacenter.

Every parser is written against a CAPTURED live transcript, committed as a fixture, not against
vendor documentation. Several of them exist because the documented shape and the observed shape
disagreed.

---

## Installation Options

| Option | Command | Build Required |
|--------|---------|----------------|
| **Daemon only** | `cd daemon && cargo build --release` | Yes (Rust) |
| **Skills only** | `cp skills/claude/*.md ~/.claude/skills/` | No |
| **Full stack** | `cd starter-kit && ./install.sh` | Yes (Rust) |

The full stack installer sets up: daemon, skills, hooks, configs and the stenographer for Claude, Codex and Antigravity.

---

## What's Next

The dispatch half and the trust half both ship. What is left is coverage and scale.

| Feature | Status | What It Unlocks |
|---------|--------|----------------|
| **Worktree-isolated swarms** | **Shipped** | N workers, each in its own git worktree. codex, claude and grok can each take one. Contract enforcement, timeout, retry. |
| **Plan-aware task assignment** | **Shipped** | ABE reads a plan, assigns tasks by wave, respects dependencies. |
| **Token economics** | **Shipped** | Per-session, per-build, per-task cost tracking across every agent. |
| **Full observability** | **Shipped** | `#[instrument]` spans on all ABE functions, 20 Prometheus metrics, structured logging. |
| **Cross-model code review** | **Shipped** | Agents review each other's code, and the review is checked rather than trusted. |
| **The trust layer** | **Shipped** | Sight gate, named sources, whole-file reads, mandatory review with a real dispatch, proof of read, verdict authority, blind validation. |
| **Mutation-based validation** | Not built | The honest gap in blind validation. Break the implementation deliberately and require the blind tests to go red. Until then a weak test against a NEW API passes. |
| **Live tool captures for every parser** | Partial | Only `read_file` appears in a live grok fixture. `run_terminal_command`, `search_replace`, `grep` and one Unknown are still to capture. |
| **CI** | Not built | There is no CI on this repo. The peer panel is currently the only gate between a change and `main`, which is why review runs before commit rather than after. |
| **Dashboard** | In progress | Web UI for watching all agent sessions in real time. |
| **Fleet scaling** | Planned | Multiple orchestrators, mixed worker types, concurrent builds. |
| **Cedar governance** | Planned | Policy-based approval gates for destructive operations. |

Two things are deliberately NOT claimed. ABE and fleet task completion do not enter the review
gate at all, so work can still leave the building without a reviewer. And `TRIUMVIRATE_REQUIRE_PEER_REVIEW`
is opt-in.

The daemon is the coordination layer. The methodology runs on top of it. The agents do the work.

See [`ROADMAP.md`](ROADMAP.md) for the full plan.

---

## How It Was Built

This project is designed and built by a human (Mike Boscia) coordinating a panel of AI agents:

- **Claude:** architecture, spec review, goatrodeo, documentation, implementation
- **Codex:** implementation, and the sharpest reviewer of authorisation and concurrency
- **Antigravity (Gemini):** research, adversarial review, and the best at asking whether a test
  could actually fail
- **Grok:** the reviewer most willing to say that a fix closed the two cases you named and not
  the family they belong to
- **DeepSeek:** consulted for method questions. It has no filesystem through the bridge, so it
  is never a sighted reviewer.

Every feature goes through the loop: spec, goatrodeo, implementation, postrodeo. Nothing gets
committed without peer review first, which functions as an internal pull request. The agents
review each other's work, and now the daemon checks that the review happened.

**What the panel is worth, measured rather than asserted.** Across seven review rounds on the
trust layer, 29 defects were found by peers that would otherwise have shipped. Six of those were
inside fixes for the peers' own earlier findings. Two separate rounds found that a fix closed the
specific cases named and not the class, including one case where Grok refused its own proposed
mitigation as implemented. Twice the panel disagreed with itself and the disagreement was the
signal: in one round Grok read the same code Codex had flagged and called it correct, having
checked both fixtures, neither of which contained the malformed input Codex had in mind.

Every fix is mutation tested: the fix is broken on purpose and the test must go red. That step
caught four tests of mine that were green for the wrong reason, including one whose input
happened to contain an even number of quotes so a parser bug cancelled itself out.

37 research artifacts documenting the design process are in `archive/research/`.

---

## The Ecosystem

Triumvirate doesn't work alone. It's one layer in a stack of tools that collaborate in the same session:

| Tool | Role | How It Fits |
|------|------|-------------|
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | Primary development agent | The cockpit. You sit here. Triumvirate and Pythia plug in as MCP servers. Skills like `/goatrodeo` run from here. |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | Research + adversarial review | 2M context window. Goatrodeo spawns Gemini sessions for spec interrogation. Gemini search provides live web research during review rounds. |
| [Codex CLI](https://github.com/openai/codex) | Implementation engine | Builds from specs. Goatrodeo spawns Codex for implementer-perspective review. Then Codex builds the thing it just reviewed. |
| [Pythia](https://github.com/michaeljboscia/pythia) | Local code search | MCP server that indexes entire projects: code, docs, SQL, config, research. Agents query Pythia before making changes. Available in the same Claude session as Triumvirate. |
| [Ollama](https://ollama.com) | Local LLM for session notes | Stenographer feeds transcripts to a local model. Zero cloud cost. Zero token spend. |
| [MCP](https://modelcontextprotocol.io) | The protocol | Everything connects through MCP. Triumvirate is an MCP server. Pythia is an MCP server. Claude Code is the MCP client. One protocol, many tools, same session. |

The daily workflow: Claude Code has Triumvirate and Pythia both registered as MCP servers. You search code with Pythia, spawn agent sessions with Triumvirate, and run goatrodeos that use both, all without leaving the editor. The tools compose because they share a protocol.

---

## Prior Art

No source code was vendored from any of these projects. Where patterns were adapted, implementation was re-authored in Rust. Inline attribution comments appear at adaptation points in the source. Full details in [`NOTICE.md`](NOTICE.md).

| Project | License | What We Adapted |
|---------|---------|----------------|
| [Temporal](https://github.com/temporalio/temporal) | Apache 2.0 | Event-sourced workflow persistence, crash recovery, retry with backoff |
| [Ruflo](https://github.com/ruvnet/ruflo) | MIT | Multi-model agent routing, cost-optimized model selection, swarm coordination patterns |
| [Clash](https://github.com/nicholasgasior/clash) | MIT | Real-time git worktree conflict detection between parallel agents |
| [swarms-rs](https://github.com/swarms-rs) | Apache 2.0 | Rust agent lifecycle management, supervisor patterns |
| [Flotilla](https://github.com/UrsushoribilisMusic/agentic-fleet-hub) | Open source | Cross-model peer review as mandatory gate, structured lessons ledger, shared state patterns |
| [ensemble](https://github.com/michelhelsdingen/ensemble) | MIT | JSONL file-based message bus with fcntl locking, tmux session management |
| [RunDiffusion Agents](https://github.com/rundiffusion/RunDiffusion-Agents) | Apache 2.0 | YAML governance control plane, agent-manages-agents pattern |
| [AgentsMesh](https://github.com/AgentsMesh/AgentsMesh) | BSL-1.1 | gRPC+mTLS control plane architecture (studied only, not used in production per BSL-1.1 terms) |
| [Claude Agent Teams](https://docs.anthropic.com) | Anthropic | Git worktree isolation, shared task list with dependency tracking, peer-to-peer mailbox messaging |

---

## Acknowledgments

Triumvirate exists because several companies built AI agents good enough to coordinate, and
good enough to catch each other:

- **Anthropic:** Claude Code and the MCP protocol that connects everything
- **Google:** Antigravity and the Gemini CLI, stream-json and a large context window
- **OpenAI:** Codex CLI with `exec --experimental-json` and a real sandbox
- **xAI:** grok CLI with streaming-json and per-turn cost reporting
- **DeepSeek:** an HTTP sibling for method questions at a fraction of the cost

The agents are theirs. The coordination is ours. The methodology emerged from using them
together every day and finding out what works, mostly by finding out what quietly does not.

---

## License

[FSL-1.1-ALv2](LICENSE) (Functional Source License 1.1, Apache 2.0 Future License)
