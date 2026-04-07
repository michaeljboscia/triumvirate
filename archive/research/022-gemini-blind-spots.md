# Research 022: Gemini's Terrifying Blind Spots

## 4 Things That Will Bite Us

### A. The "REPL State" Illusion
Do CLIs maintain context in RAM between stdin inputs, or are they stateless binaries we keep warm? If we have to resend 200K tokens of history on every turn, we get massive serialization lag. MUST determine if `--input-format stream-json` supports multi-turn state or requires full history replay.

### B. Mid-Generation Interruption
If Claude is streaming wrong code and Gemini catches it on line 10 — how do we interrupt Claude without killing the process? SIGINT might crash the CLI. Need a clean "cancel generation" signal that keeps the subprocess alive and listening.

### C. JSON Parsing GC Pressure
3 subprocesses streaming tokens → Go unmarshals → publishes to NATS → subscribes → renders TUI. Thousands of allocations/sec. Go's GC will cause micro-stutters. Need zero-allocation JSON parsing (`valyala/fastjson` or `mailru/easyjson`) and `sync.Pool` buffer reuse.

### D. ANSI Escape Corruption
CLIs might output raw ANSI color codes even when asked for JSON. If raw ANSI enters the TUI engine, terminal grid corrupts. Need stream sanitization before parsing.

## Gemini's Updated Architecture: "Terminal Hypervisor"
- Go Daemon is a Subprocess Multiplexer
- Reads CLI stdout → parses token deltas → fires into NATS JetStream
- BubbleTea TUI renders 4-pane terminal at 60fps
- Human typing in bottom pane, 3 agents streaming in top panes

## Research Needed (Gemini's list)
1. CLI stdin persistent REPL state vs stateless + interrupt semantics
2. Zero-allocation JSON streaming in Go
3. BubbleTea concurrency limits / max updates per second
4. Claude Code stream-json exact payload format
5. CRDTs in terminal visualization
