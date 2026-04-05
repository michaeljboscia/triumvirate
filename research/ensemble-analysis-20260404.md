## Ensemble — Multi-Agent Collaboration Engine

**Repo:** https://github.com/michelhelsdingen/ensemble
**Author:** michelhelsdingen
**License:** MIT
**Language:** TypeScript (server + lib), Bash (agent scripts), Python (message serialization)
**Stars:** 141 | **Commits:** ~61 | **Forks:** 16
**Created:** 2026-03-20 | **Last updated:** 2026-04-05

---

### How real-time discussion works

Agents run in separate **tmux sessions**. Communication happens through a **file-based message bus** — `team-say.sh` appends JSON-lines messages to a shared file using Python `fcntl` file locking. `team-read.sh` reads the feed via the HTTP API (`/api/ensemble/teams/:id/feed`). Each message is a JSON object with `id`, `teamId`, `from`, `to`, `content`, `type`, and `timestamp`. This means agents work in sandboxed CLI environments but share a common feed file on disk.

### Communication protocol

- **Write path:** File-append with exclusive `fcntl.LOCK_EX` lock (no network required — works inside sandboxed agents).
- **Read path:** HTTP GET to the ensemble server, which reads the same message file.
- **Message format:** JSONL — one JSON object per line, UUID-identified, UTC-timestamped.
- **Addressing:** Each message has explicit `from` and `to` fields (agent names). Messages can target specific agents or broadcast.
- **Orchestration:** Server manages team lifecycle — spawn, monitor (watchdog), auto-disband on completion signals (regex pattern matching for "done"/"complete"/Dutch equivalents).

### Gemini integration maturity

**Experimental.** Gemini CLI is defined in `agents.json` with `--yolo` flag and `pasteFromFile` input method. The README explicitly warns: "Gemini CLI can join teams and send messages, but is experimental. It may stop responding due to free-tier rate limits or internal agent delegation issues in Gemini's TUI." It is a second-class citizen compared to the Claude+Codex pair.

### Multiple instances of the same agent

**Yes, supported.** The API accepts an array of agent objects in the `agents` field when creating a team. Each gets its own tmux session. Nothing prevents `[{program: "claude", role: "lead"}, {program: "claude", role: "worker"}]`. Agent spawning is per-entry, not per-program.

---

### Key architectural notes

- tmux is the session substrate — every agent is a tmux pane.
- Input delivery differs by agent: `sendKeys` (Claude, Aider) vs `pasteFromFile` (Codex, Gemini).
- Supports multi-host via SSH-based remote spawning.
- Ships a Claude Code `/collab` skill for one-command team launch.
- TUI monitor for live observation of agent conversations.
- Worktree isolation — can create git worktrees per agent to avoid conflicts.
