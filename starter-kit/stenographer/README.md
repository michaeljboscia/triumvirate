# Stenographer — Zero-Cost Incremental Session Notes

Local Ollama-powered session notes engine that reads transcript deltas and writes rolling narrative logs. Zero API cost, zero context window impact.

## Quick Context (For AI)

Stenographer replaces cloud-based full-transcript summarization with delta-only local inference. It reads only new content since the last save, feeds it to a local Ollama model, and appends timestamped narrative paragraphs to a rolling session log. Called automatically by the token gate hook when transcript growth crosses a threshold.

## Why This Exists

The old approach (Gemini-based `pre-compact.sh`) re-read the entire transcript on every save — burning 69M Gemini tokens in 4 days with 92 redundant saves. Stenographer fixes this by:

1. **Delta-only extraction** — Only reads bytes added since the last cursor position
2. **Local inference** — Uses Ollama (qwen2.5:32b) instead of cloud APIs
3. **Background execution** — Runs via `disown` so the agent is never blocked
4. **Two-phase state** — Cursor advances only after successful log append (no data loss)

## Architecture

```
Token Gate (post-tool-use-token-gate.sh)
  │
  ├── Transcript grows past threshold (~50K tokens)
  │
  └── Spawns stenographer.py in background
        │
        ├── 1. Acquire mkdir-based lock (macOS compatible)
        ├── 2. Load state → get last cursor position
        ├── 3. Parser extracts delta (claude/gemini/codex)
        ├── 4. Health-check Ollama → POST /api/generate
        ├── 5. Append timestamped section to session log
        └── 6. Advance cursor ONLY after successful append
```

## Supported Transcript Formats

| Agent | Format | Cursor Type | Parser |
|-------|--------|-------------|--------|
| Claude | JSONL (byte-range) | Byte offset | `parsers/claude.py` |
| Gemini | JSON dict (single file) | Message index | `parsers/gemini.py` |
| Codex | JSONL (byte-range) | Byte offset | `parsers/codex.py` |

## Usage

```bash
# Automatic (called by token gate hook)
# No manual action needed — runs in background

# Manual invocation
python3 stenographer.py --agent claude --transcript /path/to/transcript.jsonl

# Dry run (extract delta without calling Ollama)
python3 stenographer.py --agent claude --transcript /path/to/transcript.jsonl --dry-run

# Reset cursor (reprocess from beginning)
python3 stenographer.py --agent claude --transcript /path/to/transcript.jsonl --reset

# Override model
python3 stenographer.py --agent claude --transcript /path/to/transcript.jsonl --model llama3.1:70b
```

## Prerequisites

- **Python 3.9+** (uses `zoneinfo` for timezone-aware timestamps)
- **Ollama** installed and running (`brew install ollama` / `curl -fsSL https://ollama.ai/install.sh | sh`)
- **A model pulled** — default is `qwen2.5:32b` (19GB). Smaller options:
  - `qwen2.5:14b` — 8.7GB, faster, slightly lower quality
  - `qwen2.5:7b` — 4.4GB, fast, adequate for basic notes
  - Set via `STENOGRAPHER_MODEL` env var or `--model` flag

## Configuration

All via environment variables (set before starting your agent):

| Variable | Default | Purpose |
|----------|---------|---------|
| `STENOGRAPHER_MODEL` | `qwen2.5:32b` | Ollama model for generation |
| `STENOGRAPHER_TIMEOUT` | `180` | Seconds to wait for Ollama response |
| `STENOGRAPHER_NUM_CTX` | `65536` | Context window size for Ollama |
| `OLLAMA_HOST` | `http://localhost:11434` | Ollama API base URL |

Token gate thresholds (in `post-tool-use-token-gate.sh`):

| Variable | Default | Purpose |
|----------|---------|---------|
| `TOKEN_GATE_THRESHOLD_KB` | `200` | Save after this many KB of growth (~50K tokens) |
| `TOKEN_GATE_COOLDOWN_SECS` | `300` | Minimum seconds between saves |
| `TOKEN_GATE_CHECK_EVERY_N` | `15` | Check size every N tool calls |

## File Layout

```
~/.triumvirate/
├── stenographer-state.json    # Cursor positions per agent
├── stenographer.log           # Structured execution log
├── locks/                     # mkdir-based per-transcript locks
└── session-logs/              # Generated session logs (fallback location)

~/.ai-memory/stenographer/     # Primary session log location (if ~/.ai-memory exists)
```

## How It Integrates

The token gate hook (`post-tool-use-token-gate.sh`) fires after every tool call. Every N calls, it checks if the transcript has grown past the threshold. When it has:

1. Updates its own state (prevents double-trigger)
2. Launches `stenographer.py` in a background subshell with `disown`
3. Emits `additionalContext` so the agent knows a save happened

Stenographer runs completely independently — if Ollama is down or the model isn't pulled, it logs the error and exits without advancing the cursor (retry on next trigger).

## Security

- **Secret redaction** — API keys, tokens, passwords, and private keys are scrubbed before reaching the model
- **No network calls** — All inference is local via Ollama (nothing leaves your machine)
- **No context cost** — Runs in a separate process, never touches the agent's context window
