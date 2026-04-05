# Research 026: Blind Spot Resolutions (Batch 1)

## Blind Spot A: REPL State — CRITICAL FINDING
**`--input-format stream-json` is STATELESS.** Full conversation history must be resent with each turn as NDJSON. This is by design for pipeline scenarios (pipe output of one agent to another).

**BUT:** Interactive REPL mode IS stateful with auto-compaction. And `--resume <session_id>` resumes a specific conversation with full context.

**Architecture decision:** DON'T use raw stream-json for multi-turn conversations. Instead:
- Option A: Use `--resume <session_id>` with persistent sessions per agent
- Option B: Use interactive mode with stdin/stdout pipes (how the Agent SDK does it)
- Option C: Manage conversation state in the Go daemon, resend compressed history via stream-json
- Option D: Investigate `--input-format stream-json` with `--replay-user-messages` flag

**This needs more research.** The Agent SDK (hexdocs.pm) spawns Claude CLI and streams JSON events — how does IT maintain state?

## Blind Spot B: Mid-Generation Interruption — SOLVED
- Escape key sends AbortError signal to active stream
- Stops generation immediately, preserves partial content
- Keeps session alive and listening
- For subprocess: we can write the escape key byte to stdin
- Ctrl+C twice kills entirely — AVOID for subprocess management
- Need to test: does writing ESC (0x1B) to stdin pipe trigger AbortError?

## Blind Spot C: GC Pressure — SOLVED
**Winner: jsoniter (github.com/json-iterator/go)**
- Reads directly from io.Reader (unlike fastjson and easyjson)
- Forward-only Iterator API: readObject, readString, readArray, skip
- Minimal allocations, reusable Iterator instances
- Perfect for token-by-token parsing from CLI stdout pipes

**NOT suitable:** fastjson (no io.Reader support), easyjson (no streaming support)

## Blind Spot D: ANSI Escape Corruption — SOLVED
- Regex: `\x1b\[[0-9;]*m` strips common ANSI color codes
- Process line-by-line: strip ANSI → trim → attempt JSON unmarshal → fallback to text
- Go's regexp package handles this natively
- For paranoia: also check for `\x9B` (8-bit CSI) sequences
- bcicen/jstream useful for streaming multi-document JSON without interleaved text

## Sources
avasdream.com, github.com, blakecrosley.com, shipyard.build, medium.com, claude.com, reddit.com, jsoniter.com, go.dev, baeldung.com, stackoverflow.com, tutorialspoint.com
