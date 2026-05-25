# Migration Analysis: Google Gemini CLI to Antigravity CLI for Subprocess Orchestration

> **Provenance:** Authored by **Gemini** (deep research). Source = Google Doc `1BcTLwz8m0ck-DvenhfyGSMqoH0u9vwYyTnxTv-yjjs8`, ingested 2026-05-23. This is one of two independent research briefs on the same migration; the other is the **Claude-web** brief `Migration Brief…agy.md` in this directory. Authorship was confirmed by Gemini's own side-by-side comparison of the two briefs (it claims the API-key / agy-acp / hooks / plan-mode coverage as its own). Escape artifacts from the Google Docs export have been cleaned; substance is unchanged.
>
> ⚠️ **Verification note:** This brief's recommended headless-auth path — "agy respects the `ANTIGRAVITY_API_KEY` env var" — is contradicted by GitHub Issue #78 (open, filed 2026-05-21), which *requests* that Google add `ANTIGRAVITY_API_KEY`/`GEMINI_API_KEY` support. As of late May 2026 that env var is **not** implemented. Treat the API-key section below as aspirational, not working. See `triumvirate-agy-migration-synthesis.md` for the full adjudication.

The impending deprecation of the Google Gemini command-line interface, universally known as the gemini-cli, marks a definitive shift in Google's approach to terminal-based artificial intelligence tooling. Scheduled to cease serving requests for consumer and free-tier users on June 18, 2026, the legacy binary is being entirely supplanted by the new antigravity-cli, distributed under the binary name `agy`. For individual developers interacting directly with a terminal, this transition represents a lateral move with upgraded graphical capabilities. However, for systems engineers and maintainers of local orchestration daemons—such as the Rust-based "Triumvirate" application—this deprecation constitutes a severe architectural disruption. Triumvirate currently operates by wrapping the gemini-cli as a spawned subprocess, passing arguments via command-line flags, and orchestrating complex asynchronous workflows by intercepting and parsing structured stdio streams.

The antigravity-cli is not a simple drop-in replacement for the gemini-cli. While the legacy tool functioned primarily as a lightweight, thin client for the Gemini API, the new `agy` binary operates as a fully-fledged, stateful agent orchestration harness that shares its core engine with the Antigravity 2.0 desktop environment. This architectural divergence has introduced profound changes to how the binary handles non-interactive execution, pseudo-terminal detection, standard output flushing, and session authentication. The current 1.0.0 release possesses critical regressions for headless subprocess automation, including a severe bug that silently drops standard output when executed outside of a physical terminal emulator.

## The Architectural Paradigm Shift

The legacy gemini-cli was explicitly optimized for direct API interactions, offering robust support for traditional shell scripting, piping, and headless execution via structured data payloads. It operated on a mostly stateless premise, relying on the caller to provide context and manage conversation identifiers.

Conversely, the antigravity-cli is engineered in Go and functions as the lightweight Terminal User Interface (TUI) surface for the broader Antigravity platform. It is designed to orchestrate autonomous background tasks, manage modular subagents, and maintain a persistent, bidirectional synchronization with the Antigravity 2.0 desktop application. Because the binary is inherently designed to manage rich TUI states, terminal sandboxing, and complex local file manipulations, its headless execution pathways have been deprioritized in the initial release cycle. Integrating with `agy` requires the Triumvirate daemon to interact with a highly opinionated, stateful engine rather than a passive data pipe.

The daemon can no longer assume that the subprocess will passively read from the standard input buffer, nor can it rely on the subprocess to emit predictable, continuous streams of data. Instead, the daemon must be rewritten to actively manage the execution environment, spoofing terminal interfaces where necessary, and handling bulk data payloads synchronously.

## Binary Invocation and Command-Line Semantics

### Headless Prompt Execution
Legacy: `-p` / `--prompt`, optionally coupled with stdin piping. The binary consumed the prompt flag, appended any data piped via stdin, processed inference, and exited.

Antigravity: supports `-p` as an alias but its primary explicit headless flag is `--print`. `agy` is strict: the prompt must be passed as an explicit argument directly following the flag (`agy -p "Prompt text"`). Piping the prompt body through stdin while using the headless flag results in a fatal "flag needs an argument" error. This forces all prompts to be formatted as string literals in the argument vector, potentially necessitating chunking for large prompts to avoid ARG_MAX limits.

### Output Formatting and Structured Data
Legacy: `--output-format` accepted `text`, `json`, `stream-json`. This was the cornerstone of daemon integration.

Antigravity: native support for `--output-format stream-json` is entirely absent in v1.0.0. Documentation indicates that while `--output-format json` *may* be recognized for bulk payload returns, the core `agy -p` / `--print` command defaults to emitting unformatted plain text to stdout. The removal of streaming JSON forces a fundamental rewrite of the parsing logic.

### Safety Boundaries and Approval Modes
Legacy: `--approval-mode plan` enforced a rigid read-only mode (blocking write/execute tool calls at the policy level while allowing reads). `--yolo` / `--approval-mode yolo` auto-approved all actions.

Antigravity: lacks an equivalent to `--approval-mode plan` for non-interactive runs, a severe security vulnerability for scripted automation processing untrusted inputs — in non-interactive `-p` execution, `agy` will auto-approve tool calls (including file-writing tools) unless explicitly constrained. To force-bypass all permissions in headless mode (replicating legacy YOLO), `agy` uses `--dangerously-skip-permissions`. `agy` also integrates a native OS sandbox layer (nsjail on Linux, sandbox-exec on macOS, AppContainer on Windows) via the `--sandbox` flag.

### Context Management and Workspace Boundaries
Legacy `--include-directories` → new `--add-dir` to mount local directories into the active session context.

### Comprehensive Flag Mapping Reference

| Functionality | Legacy (gemini-cli) | New (agy) | Notes / Migration Status |
| :-- | :-- | :-- | :-- |
| Headless Execution | `-p "..."` / `--prompt "..."` | `--print "..."` / `-p "..."` | `agy -p` strictly takes the prompt as a string arg; cannot consume primary prompt from stdin pipes. |
| Output Formatting | `--output-format <text\|json\|stream-json>` | `--output-format <json>` (limited) | `stream-json` (NDJSON) completely missing in agy v1.0.0 — major regression. |
| Read-Only Safety | `--approval-mode plan` | **No direct equivalent** | Severe regression; `agy -p` auto-approves write commands. |
| Full Automation (YOLO) | `--yolo` / `--approval-mode yolo` | `--dangerously-skip-permissions` | Bypasses all tool-call confirmation prompts. |
| Context Injection | `--include-directories <paths>` | `--add-dir <path>` | Syntax for mounting local filesystems updated. |
| Model Selection | `-m <model-id>` | `/model` (interactive TUI) or `-m` via adapter | Headless model selection flags inconsistent in base agy; often requires interactive slash commands. |

## Standard I/O, IPC Pipelines, and the Non-TTY Defect

### The Standard Output Suppression Anomaly (Issue #76)
When a Rust app uses `std::process::Command` to spawn a child subprocess, it creates anonymous unnamed pipes for stdin/stdout/stderr. Because these are data pipes and not genuine terminal emulator interfaces, the OS flags the environment as non-TTY.

The legacy gemini-cli detected this and flushed JSON output into the stream regardless. The antigravity-cli suffers a severe defect (community-documented as **Issue #76**): when `agy --print` / `-p` runs in a non-TTY context (piped, redirected, or spawned as a headless subprocess), the binary silently drops all stdout emissions. The Go app completes the full inference round trip, executes local tools, and exits 0 — but emits an entirely empty buffer to both stdout and stderr. For Triumvirate, awaiting the output of `Command::new("agy").stdout(Stdio::piped())` yields nothing, breaking the orchestration loop.

### Engineering Mitigations for the Rust Daemon
- **PTY allocation:** spawn `agy` inside a virtual PTY (e.g., `ptyprocess` or `nix::pty`) so the binary's `isatty` checks pass and it flushes output normally; read raw bytes from the master side, then manually strip ANSI escape sequences / color codes / carriage returns.
- **ACP adapter:** integrate the `agy-acp` (Agent Client Protocol) bridge — a lightweight open-source intermediary that wraps `agy` and exposes a stable streaming JSON-RPC stdio interface, bypassing internal PTY management.

### Standard Input and Headless Ingestion
Legacy permitted direct piping of massive context files via stdin (`cat massive_log.txt | gemini`). Current `agy` strictly requires the prompt as an explicit flag argument and does not consume the stdin buffer for the primary prompt. All context must be serialized into the argument string, potentially necessitating file-writing workarounds (write prompt to temp file, pass file path) to avoid ARG_MAX limits.

## Output Formatting, Data Schemas, and the Loss of Streaming

### The Eradication of NDJSON Streaming
Legacy `--output-format stream-json` emitted NDJSON lifecycle events (`init`, `message`, `tool_use`, `tool_result`, `result`), enabling async stream parsers (e.g., `tokio_util::codec::LinesCodec`) to process chunks in real time. agy v1.0.0 completely eradicates native NDJSON streaming: `agy -p` blocks to completion and returns the entire finalized payload at once (averaging ~5 s per prompt, longer for multi-tool runs). All async stream parsers must be replaced with synchronous bulk JSON deserializers (`serde_json::from_str` over the full buffer).

### Schema Discrepancies in Bulk JSON Mode
Legacy bulk JSON: `{ "response": "...", "stats": { "input_tokens": 150, "output_tokens": 300 }, "error": null }`. The exact schema emitted by `agy` in JSON mode is sparsely documented (the non-TTY bug suppresses stdout for most automated testers). Daemons forced to use intermediate adapters like `agy-acp` interact with the ACP JSON-RPC format instead, requiring a different deserialization struct.

### Conversation Tracking and Multi-Turn Contamination
Legacy emitted a unique session ID in the `stream-json` `init` event, captured and re-injected via `--resume <session-id>`, allowing dozens of parallel independent threads without cross-contamination. `agy --print` does not surface a conversation ID to stdout/stderr under any circumstances. `--continue` / `-c` resumes the *most recent* conversation across the entire host machine — for a multi-tenant daemon this causes catastrophic cross-contamination (Agent B resumes Agent A's session). Workaround: force each execution into isolated ephemeral directories (`--add-dir`) to leverage workspace-scoped state tracking.

## Headless Authentication and OS Keyring Dependencies

### The D-Bus and Keyring Conundrum
Legacy stored auth in plain-text JSON or used env vars (`GEMINI_API_KEY`) to bypass interactive auth. antigravity-cli depends on OS security mechanisms: it authenticates silently by reading/writing OAuth tokens to the host keyring via `go-keyring` (Keychain/macOS, Credential Manager/Windows, libsecret over D-Bus/Linux).

In a headless Linux container or WSL2 instance lacking an active D-Bus session or `gnome-keyring-daemon`, `agy` fails to persist the OAuth token; every invocation stalls, prompting an interactive browser OAuth flow and breaking automation. Logs at `~/.gemini/antigravity-cli/cli.log` state: `consumerOAuth: failed to persist token to keyring: failed to unlock correct collection`.

### Engineering Mitigations for Headless Environments
1. **Daemonized D-Bus init:** container entrypoint installs/spawns a virtual keyring (`dbus-x11`, `libsecret-tools`), generates a dummy secret to force creation of `~/.local/share/keyrings`, and exports `DBUS_SESSION_BUS_ADDRESS` before spawning `agy`.
2. **API key env injection (recommended):** inject a dedicated API key directly into the subprocess environment. `agy` respects the `ANTIGRAVITY_API_KEY` env var (distinct from legacy `GEMINI_API_KEY`), allowing stateless authentication without the host keyring.

## Configuration Paths, Plugins, and State Management
Both systems still use `~/.gemini/` but sub-paths shifted:
- Legacy: `~/.gemini/settings.json`; extensions in a flat local structure.
- New: settings at `~/.gemini/antigravity-cli/settings.json`. Legacy Gemini extensions are obsolete; migrate to the new Antigravity Plugin architecture via `agy plugin import gemini`.
- MCP config relocated to `~/.gemini/antigravity/mcp_config.json`. Daemons that dynamically generate MCP configs must target this new path.

### System Hooks and Interception
The new `agy` hook system still receives JSON input via stdin and returns JSON via stdout, but uses strict camelCase and an expanded payload schema — all hooks now receive `conversationId` and `workspacePaths`. `agy` natively supports async subagent delegation (`invoke_subagent`, `define_subagent`); the daemon can offload heavy analytical workloads to isolated background tasks and poll for completion rather than expecting synchronous returns.

## Migration Verdict for the Triumvirate Daemon
**agy is NOT a 1:1 drop-in replacement for the gemini-cli at the subprocess layer.** It is a fundamentally new API design mandating substantial structural rewrites of the Triumvirate Rust bridge. Primary blockers: (1) non-TTY stdout suppression bug, (2) total eradication of NDJSON streaming, (3) inability to address specific multi-turn conversations reliably in headless mode, (4) hard dependency on OS keyrings in headless Linux.

**Strategic recommendation:** avoid wrapping the bare `agy` binary with standard `std::process::Command` pipes. Two viable patterns:
1. **Protocol Adapter Pattern (highly recommended):** integrate the open-source `agy-acp` (Agent Client Protocol) Rust adapter — bypasses the TTY bug, normalizes the schema, reinstates streaming via standardized ACP.
2. **PTY and Polling Pattern (fallback):** spawn `agy` inside a PTY to force stdout flushing; downgrade async NDJSON parsers to synchronous bulk JSON decoders; segregate multi-turn state via physically isolated directories (`--add-dir`).

### Side-by-Side Subprocess Execution Examples

**Legacy (gemini-cli):**
```rust
use std::process::{Command, Stdio};
use std::io::{BufReader, BufRead};

fn execute_gemini_prompt(prompt: &str) {
    let mut child = Command::new("gemini")
        .arg("-p").arg(prompt)
        .arg("--output-format").arg("stream-json")
        .stdout(Stdio::piped()) // Standard pipe operates flawlessly
        .spawn()
        .expect("Failed to execute gemini-cli");

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let json_event = line.unwrap();
        // Parse event type (init, message, tool_result, ...) in real time
        println!("Received stream chunk: {}", json_event);
    }
}
```

**New (antigravity-cli) — PTY wrapper + API key env injection + synchronous payload:**
```rust
use std::process::Command;
use ptyprocess::PtyProcess; // external crate for PTY management

fn execute_agy_prompt(prompt: &str, api_key: &str) {
    // Standard Stdio::piped() fails silently and drops all stdout.
    // The daemon MUST allocate a PTY to deceive 'agy' into outputting data.
    let mut command = Command::new("agy");
    command.arg("--print").arg(prompt)
        .arg("--dangerously-skip-permissions") // force auto-approve tools
        .arg("--output-format").arg("json")
        .env("ANTIGRAVITY_API_KEY", api_key); // bypass fragile keyring auth

    let mut pty_process = PtyProcess::spawn(command)
        .expect("Failed to execute agy within PTY boundary");

    // Execution strictly blocks until complete (no streaming support)
    let _exit_status = pty_process.wait().unwrap();

    let mut output = String::new();
    pty_process.get_raw_handle().read_to_string(&mut output).unwrap();
    // Must manually strip ANSI escape codes if the TUI injects them.
    println!("Received bulk response: {}", output);
}
```

## Works Cited (as provided by the source)
1. Google Developers Blog — *An important update: Transitioning Gemini CLI to Antigravity CLI* — https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/
2. VirtualizationReview — *Google Moves Gemini CLI Into Antigravity CLI* — https://virtualizationreview.com/articles/2026/05/19/google-moves-gemini-cli-into-antigravity-cli-as-agent-platform-expands.aspx
3. Antigravity CLI Overview — https://antigravity.google/docs/cli-overview
4. **Issue #76** — *agy --print / -p silently drops stdout when run with a non-TTY* — https://github.com/google-antigravity/antigravity-cli/issues/76
5. google-gemini/gemini-cli — https://github.com/google-gemini/gemini-cli
6. Antigravity blog — *Subagents, Hooks, Scheduled Tasks…* — https://antigravity.google/blog/google-io-2026-feature-deep-dive
7. Gemini CLI cheatsheet — https://geminicli.com/docs/cli/cli-reference/
8. Gemini CLI configuration — https://geminicli.com/docs/reference/configuration/
9. Reddit r/google_antigravity — *How to Capture Antigravity-CLI Output?* — https://www.reddit.com/r/google_antigravity/comments/1tk6hx5/
10. **Issue #7** — *feat(--print): emit per-conversation ID* — https://github.com/google-antigravity/antigravity-cli/issues/7
11. gsd-build/get-shit-done **Issue #3782** — https://github.com/gsd-build/get-shit-done/issues/3782
12. Reddit r/google_antigravity — *Built a quick agent skill for agy* — https://www.reddit.com/r/google_antigravity/comments/1ti2zq6/
13. Reddit r/GeminiAI — *antigravity cli doesn't remember auth* — https://www.reddit.com/r/GeminiAI/comments/1ti1xiq/
14. DEV Community (Arindam Majumder) — *Antigravity CLI Hands-On Guide* — https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7
15. Zeabur — *OpenAB Antigravity Deploy Guide* — https://zeabur.com/templates/SSJNXB
16. **Issue #45** — *read-only / plan-mode equivalent for non-interactive -p runs* — https://github.com/google-antigravity/antigravity-cli/issues/45
17. Antigravity docs — *Using AGY CLI* — https://antigravity.google/docs/cli-using
18. Antigravity docs — *CLI features* — https://antigravity.google/docs/cli-features
19. Lib.rs — atomr-agents-coding-cli-vendor-antigravity — https://lib.rs/crates/atomr-agents-coding-cli-vendor-antigravity
20. GitHub openabdev/openab — ACP harness — https://github.com/openabdev/openab
21. Gemini CLI headless mode reference — https://geminicli.com/docs/cli/headless/
22. google-gemini/gemini-cli Issue #24058 — https://github.com/google-gemini/gemini-cli/issues/24058
25. Codelabs — Getting Started with Google Antigravity — https://codelabs.developers.google.com/getting-started-google-antigravity
26. Antigravity docs — Getting Started with Antigravity CLI — https://antigravity.google/docs/cli-getting-started
27. Google AI Dev forum — *agy fails to persist authentication state in WSL 2* — https://discuss.ai.google.dev/t/bug-antigravity-cli-agy-fails-to-persist-authentication-state-in-wsl-2-environment/146059
28. Antigravity docs — Migrating from Gemini CLI — https://antigravity.google/docs/gcli-migration
29. Antigravity docs — MCP Integration — https://antigravity.google/docs/mcp
30. Antigravity docs — Hooks — https://antigravity.google/docs/hooks
