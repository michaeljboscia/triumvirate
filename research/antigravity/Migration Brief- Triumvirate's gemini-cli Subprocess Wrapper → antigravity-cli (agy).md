# Migration Brief: Triumvirate's `gemini-cli` Subprocess Wrapper → `antigravity-cli` (agy)

## TL;DR
- **Antigravity CLI (`agy`) is NOT a 1:1 drop-in for `gemini-cli` at the subprocess layer.** Triumvirate's Rust bridge will need a meaningful rewrite: at minimum, the structured-output parser must be torn out (because `agy --print` emits plain text only — no `--output-format json` / `stream-json` exists in v1.0.x), and the multi-thread "N independent concurrent conversations" architecture cannot be preserved with stock flags because `--print` never surfaces a conversation ID and `-c/--continue` resumes the most recent conversation **globally** rather than per-caller ([Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7)).
- **Plan for a hard cutover by mid-June 2026.** With Consumer AI Ultra (`oauth-personal`), your access to the legacy CLI stops on 2026-06-18 per the Google Developers Blog post by Dmitry Lyalin and Taylor Mullen ([*An important update: Transitioning Gemini CLI to Antigravity CLI*](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/)): *"On June 18, 2026, Gemini CLI and Gemini Code Assist IDE extensions will stop serving requests for Google AI Pro and Ultra, as well as those using it free of charge using Gemini Code Assist for individuals."* Core Gemini models (Gemini 3.5 Flash, Gemini 3.1 Pro) stay on the flat-rate subscription via `agy`, but Claude Sonnet/Opus and GPT-OSS in `agy` are routed through Vertex AI Model Garden, which means a paid GCP project, the `https://www.googleapis.com/auth/cloud-platform` OAuth scope, and per-token billing once you exceed your AI Ultra baseline credits.
- **Concrete rewrite checklist:** (1) swap binary name `gemini` → `agy`; (2) delete `--output-format json`/`stream-json` code paths and replace with a transcript-file reader against `~/.antigravitycli/<uuid>.json`; (3) replace `--session-id <uuid>` with a workaround (write our own sidecar mapping, or fork to add support for caller-supplied IDs once [Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7) is resolved); (4) remove `cat file | gemini -p -` stdin-piping (agy's `-p` only accepts a flag value, not stdin); (5) move config probes from `~/.gemini/` to a mix of system keyring + `~/.antigravitycli/` + `~/.antigravity/mcp_config.json`. Budget ~2 engineer-weeks if you keep `oauth-personal` only and several extra weeks if you also need Vertex routing for Claude.

---

## Key Findings

### 1. Binary & CLI Surface
- The new tool's binary is `agy` (not `antigravity` and not `gemini`). Installed by `curl -fsSL https://antigravity.google/cli/install.sh | bash` on Linux/macOS into `~/.local/bin/agy`, or `irm https://antigravity.google/cli/install.ps1 | iex` on Windows PowerShell into `%LOCALAPPDATA%\Antigravity\` ([dev.to hands-on install guide](https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7); [Agentpedia deep dive](https://agentpedia.codes/blog/antigravity-cli-deep-dive)).
- Written in Go (gemini-cli was Node/TypeScript), per the [Google Developers Blog](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/): *"Built in Go, Antigravity CLI is snappier and more responsive."* Version 1.0.0 was the I/O 2026 launch on May 19, 2026; 1.0.1 was already shipped within days for early bug fixes ([Romin Irani's Medium tutorial](https://medium.com/google-cloud/antigravity-cli-tutorial-series-12b46cfe3bf2)).

### 2. Headless flag inventory differs sharply
- Captured from a hands-on Linux dump in the [LinuxCapable install walkthrough](https://linuxcapable.com/how-to-install-google-antigravity-on-ubuntu-linux/), the **only** headless flags in `agy --help` (v1.0.0) are: `--add-dir`, `-c/--continue`, `--conversation <id>`, `--dangerously-skip-permissions`, `-i/--prompt-interactive`, `--log-file`, `-p/--print` (with `--prompt` alias), `--print-timeout` (default 5m), and `--sandbox`. Subcommands: `changelog`, `help`, `install`, `plugin`/`plugins`, `update`.
- **Missing vs gemini-cli** ([gemini-cli configuration reference](https://geminicli.com/docs/reference/configuration/)): `--output-format`, `--model/-m`, `--include-directories` (replaced by `--add-dir`), `--session-id`, `--resume`, `--all_files`, `--yolo` (the rough equivalent is the much-broader `--dangerously-skip-permissions`), `--prompt-interactive` exists but only as a session bootstrap, all `--telemetry-*` flags.
- **Renamed/repurposed:** `-c/--continue` still exists but its semantics changed — in agy it resumes the **most recent conversation globally on the machine**, with no per-caller scoping (per [Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7)). `--conversation <id>` exists to resume a specific known conversation, but the ID is never emitted to the caller in the first place.
- **Verified no `-m/--model`:** [gsd-build Issue #3782](https://github.com/gsd-build/get-shit-done/issues/3782) (hands-on against agy 1.0.0 on macOS arm64): *"No -m / --model flag on agy today (the CLI selects the model internally), so review.models.agy config key is unused for now — leave it null, document as 'reserved for future model-pinning support'."* Model selection is via the in-TUI `/model` slash command only.

### 3. STDIN / piping has regressed for headless callers
- gemini-cli supported `cat file | gemini -p -` and even `echo X | gemini -p "..."` (the prompt could be appended to piped input — see the geminicli.com configuration reference: *"Used to pass a prompt directly to the command... Appended to input on stdin (if any)."*).
- `agy -p` takes the prompt **only** as a flag value, never from stdin. Verified by the [gsd-build hands-on test](https://github.com/gsd-build/get-shit-done/issues/3782): *"agy -p takes the prompt as a flag value, NOT from stdin (unlike gemini -p -). Verified: `echo X | agy -p` errors with `flag needs an argument: -p`. agy -p "PROMPT" returns the response on stdout (verified: agy -p "What's 2+2? Reply with just the digit." → 4)."* This means any Triumvirate code that piped a file into the CLI must switch to reading the file in Rust and passing the contents as an arg string — and mind that the modern Linux x86_64 `getconf ARG_MAX` returns up to 2,097,152 bytes (2 MiB) but the hardcoded kernel lower bound is exactly 131,072 bytes (128 KiB), per `linux/limits.h`'s `#define ARG_MAX 131072`.

### 4. Output formatting is the biggest break
- Gemini CLI's documented [`--output-format` headless reference](https://geminicli.com/docs/cli/headless/): `--output-format json` returns a single JSON object with `{response, stats, error}` fields, and `--output-format stream-json` produces JSONL events of types `init` (with `session_id`, `model`), `message` (with `role`, `content`, `delta`), `tool_use` (with `tool_name`, `tool_id`, `parameters`), `tool_result` (with `tool_id`, `status`, `output`), `error`, and a final `result` event with aggregated per-model stats.
- `agy --print` in v1.0.x has **no equivalent**. It emits the assistant's final text on stdout as plain text. There is no `--output-format` flag, no session ID emitted, no per-token streaming format. A widely-cited [dev.to tutorial by Arindam Majumder](https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7) shows `agy -p "..." --output-format json` as an example, but multiple hands-on testers ([Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7) and [gsd-build #3782](https://github.com/gsd-build/get-shit-done/issues/3782)) confirm this flag does not exist in v1.0.x — treat that tutorial as speculative.
- Worse, the headless `-p` mode has a reported "stdout bug" in subprocess/non-TTY contexts that motivated at least one MCP bridge — [SinanTufekci's Claude-Code-Antigravity-CLI-MCP-Server](https://github.com/SinanTufekci/Claude-Code-Antigravity-CLI-MCP-Server/blob/main/test_smoke.py), described in its repo header as: *"MCP bridge that exposes Google's Antigravity CLI (agy) to Claude Code as a sub-agent. Works around the headless `agy -p` stdout bug by reading the response from agy's own transcript files."* Behavior may be intermittent / response-length-dependent: a short prompt like `agy -p "What's 2+2?"` returns `4` cleanly (gsd-build #3782), while longer outputs appear truncated or empty in some wrapper contexts.

### 5. Conversation-ID model is broken for multi-threaded wrappers
- The structural blocker for Triumvirate is documented in [Issue #7 on the official repo](https://github.com/google-antigravity/antigravity-cli/issues/7) (filed by `steve-krisjanovs` on launch day): *"`agy --print "..."` (and the `-p` / `--prompt` aliases) run a single prompt non-interactively and emit the response as plain text — but the conversation's identifier is never surfaced in stdout, stderr, or any documented file."* `--conversation <id>` exists for **resuming** known IDs but cannot capture an ID from a `--print` run, and `--conversation` on a not-yet-existent UUID is reported to error rather than create. `-c/--continue` only resumes the most-recent conversation globally: *"That's fine for a single human at a terminal; it falls apart for any wrapper or scripting context that runs more than one independent conversation thread."*
- For a Rust daemon that fans out N concurrent multi-turn threads, this means agy cannot reproduce the gemini-cli pattern of `gemini -p ... --session-id <uuid>` followed by `gemini -p ... --resume <uuid>`. Workarounds are all unpleasant: parse the undocumented `~/.antigravitycli/<uuid>.json` transcript files, re-feed transcripts on every turn (lossy + expensive), or run a per-thread `agy` subprocess in pseudo-interactive mode behind a PTY.

### 6. Authentication & on-disk layout
- **Old layout (gemini-cli):** config in `~/.gemini/settings.json` with `"security.auth.selectedType": "oauth-personal"`, OAuth tokens at `~/.gemini/oauth_creds.json` (plaintext), active account at `~/.gemini/google_accounts.json`, MCP OAuth tokens at `~/.gemini/mcp-oauth-tokens.json` ([Arm Learning Paths Gemini CLI guide](https://learn.arm.com/install-guides/gemini/); [gemini-cli MCP server docs](https://google-gemini.github.io/gemini-cli/docs/tools/mcp-server.html)).
- **New layout (agy):**
  - **OAuth credentials → OS keyring** (Keychain on macOS, Credential Manager on Windows, libsecret/Secret Service on Linux) per the [DeepWiki index of the README](https://deepwiki.com/google-antigravity/antigravity-cli) and the [dev.to walkthrough](https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7). No plaintext oauth_creds.json file. Sign-out is via the in-TUI `/logout` slash command.
  - **Conversation transcripts → `~/.antigravitycli/<uuid>.json`** (per-workspace symlinks). Schema is **undocumented** in the README, binary strings, or help output, per [Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7): *"Parsing `~/.antigravitycli/` — the per-workspace symlink `<uuid>.json` files referenced in agy's project-tracking state look promising, but the schema isn't documented and the binary doesn't surface those IDs to --print callers anyway. Fragile to rely on."*
  - **MCP server config → `~/.antigravity/mcp_config.json`** (new dedicated file, no longer inline in settings.json), with `url` field renamed to `serverUrl` for remote servers ([Agentpedia deep dive](https://agentpedia.codes/blog/antigravity-cli-deep-dive)).
  - **Global skills → `~/.gemini/antigravity-cli/skills/`** (note: still under `~/.gemini/`, oddly), workspace skills → `.agents/skills/`.
  - **Context files → still `GEMINI.md` and `AGENTS.md`**, no rename.
- **Headless / SSH auth:** First-run launch on a TTY opens the system browser. Over SSH or in non-graphical sessions, agy prints an authorization URL plus a one-time code to copy from a local-machine browser. There is currently no first-class `ANTIGRAVITY_API_KEY` or `GEMINI_API_KEY` environment variable supported by `agy` — this is open feature request [Issue #78 on the official repo](https://github.com/google-antigravity/antigravity-cli/issues/78). For Triumvirate running as a daemon, this means initial auth must be done interactively once on the host, and the keyring takes over afterward. WSL2 has a known persistence bug where the keyring fails to retain tokens between invocations.

### 7. Consumer AI Ultra (oauth-personal) — what works and what costs extra in agy
- **Core Gemini models** (Gemini 3.5 Flash High/Medium, Gemini 3.1 Pro High/Low) on Consumer AI Ultra ($100/mo tier introduced at I/O 2026): covered by the flat-rate subscription. Per [Google's official I/O 2026 blog post](https://blog.google/innovation-and-ai/technology/developers-tools/google-io-2026-developer-highlights/), the Ultra plan is positioned with *"a 5X higher usage limit in Google Antigravity than our Google AI Pro plan."* Per [Google One Help](https://support.google.com/googleone/answer/16286513?hl=en), *"Google AI Ultra members receive 25,000 AI credits every month. To manage your model usage and extend your sessions beyond your baseline quota, use Google AI credits."*
- **Third-party models via Vertex AI Model Garden** (Claude Sonnet 4.6 Thinking, Claude Opus 4.6 Thinking, GPT-OSS 120B Medium): the Antigravity TUI lets you `/model` to switch to these ([Rich Rose, Medium](https://medium.com/google-cloud/getting-started-with-antigravity-cli-3565d5db1e92)), **but** they are billed against your Vertex AI Model Garden quotas. The [opencode-antigravity-auth API spec](https://github.com/NoeFabris/opencode-antigravity-auth/blob/main/docs/ANTIGRAVITY_API_SPEC.md) confirms the Antigravity auth flow requests these scopes: `https://www.googleapis.com/auth/cloud-platform`, `userinfo.email`, `userinfo.profile`, `cclog`, `experimentsandconfigs` — so a user who has only ever done the consumer "Google OAuth" path may discover at runtime that selecting Claude either consumes Ultra AI credits at an unfavorable rate or errors out asking them to connect a GCP project. Multiple users on the [Google AI Developers Forum thread "Ultra Subscription: Claude Model Quota Even Worse Than Pro"](https://discuss.ai.google.dev/t/ultra-subscription-claude-model-quota-even-worse-than-pro/135870) have complained about Claude quota under AI Ultra being effectively unusable. The cleanest setup for Triumvirate is "Gemini-only via Ultra; Claude/GPT-OSS only if a GCP project is explicitly configured."

### 8. TUI-first design has subprocess implications
- The [README, indexed by DeepWiki](https://deepwiki.com/google-antigravity/antigravity-cli), explicitly frames agy as a "Terminal User Interface (TUI)" — the interactive mode is the primary surface, and `-p/--print` is the secondary, less-tested mode. Community testers note that the CLI does detect non-TTY environments for the auth flow (it prints a URL+code on SSH instead of opening a browser), but the headless stdout still occasionally leaks TUI chrome or returns empty output for longer responses, which is why community wrappers like SinanTufekci's MCP bridge fall back to reading the transcript file rather than trusting stdout.

---

## Details

### Side-by-side: one-shot prompt invocation

**Gemini CLI (legacy) — one-shot with JSON parsing**

```bash
gemini -p "Summarize this codebase" --output-format json
```

Stdout shape (single JSON object, per the [headless mode reference](https://geminicli.com/docs/cli/headless/)):

```json
{
  "response": "This codebase implements …",
  "stats": {"input_tokens": 1234, "output_tokens": 567, "duration_ms": 4321, "tool_calls": 2},
  "error": null
}
```

Or with `--output-format stream-json`, JSONL events (schema from [PR #10883 in google-gemini/gemini-cli](https://github.com/google-gemini/gemini-cli/pull/10883)):

```jsonl
{"type":"init","timestamp":"...","session_id":"abc123","model":"gemini-3-pro"}
{"type":"message","role":"user","content":"Summarize this codebase","timestamp":"..."}
{"type":"tool_use","tool_name":"Bash","tool_id":"bash-123","parameters":{"command":"ls"},"timestamp":"..."}
{"type":"tool_result","tool_id":"bash-123","status":"success","output":"file1.txt\nfile2.txt","timestamp":"..."}
{"type":"message","role":"assistant","content":"This codebase…","delta":true,"timestamp":"..."}
{"type":"result","status":"success","stats":{"total_tokens":1801,"input_tokens":1234,"output_tokens":567,"duration_ms":4321,"tool_calls":1},"timestamp":"..."}
```

Rust subprocess invocation (gemini-cli):

```rust
use std::process::{Command, Stdio};

let mut child = Command::new("gemini")
    .args(["-p", "Summarize this codebase",
           "--output-format", "stream-json",
           "--session-id", &session_uuid])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

// Read line-delimited JSON events
let stdout = child.stdout.take().unwrap();
let reader = std::io::BufReader::new(stdout);
for line in std::io::BufRead::lines(reader) {
    let event: serde_json::Value = serde_json::from_str(&line?)?;
    match event["type"].as_str() {
        Some("init")    => { /* capture session_id */ }
        Some("message") => { /* stream delta to consumer */ }
        Some("result")  => { /* finalize, capture stats */ }
        _ => {}
    }
}
```

**Antigravity CLI (`agy`) — equivalent one-shot**

```bash
agy -p "Summarize this codebase"
```

Stdout shape: plain text only. No JSON wrapper. No session ID. No stats. Exit code 0 on success.

Rust subprocess invocation (agy, naive port):

```rust
use std::process::{Command, Stdio};

let output = Command::new("agy")
    .args(["-p", "Summarize this codebase"])  // no --output-format, no -m, no --session-id
    .stdin(Stdio::null())                     // -p does NOT accept stdin in agy
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .output()?;                               // blocking; --print-timeout default is 5m

let response_text = String::from_utf8_lossy(&output.stdout);
// No session_id available. No token usage. No structured tool-call events.
// If we need the conversation ID for multi-turn, we must peek at
//   ~/.antigravitycli/<uuid>.json
// after the call, find the newest file, and parse the (undocumented) schema.
```

Rust subprocess invocation (agy, multi-turn workaround):

```rust
// Step 1: run the prompt and capture stdout
let output = Command::new("agy")
    .args(["-p", &prompt])
    .output()?;

// Step 2: find the conversation file agy just wrote
let home = std::env::var("HOME")?;
let transcripts_dir = format!("{}/.antigravitycli", home);
let newest_transcript = std::fs::read_dir(&transcripts_dir)?
    .filter_map(Result::ok)
    .filter(|e| e.path().extension().map_or(false, |x| x == "json"))
    .max_by_key(|e| e.metadata().unwrap().modified().unwrap())
    .ok_or("no transcripts")?;

let conversation_id = newest_transcript.path()
    .file_stem().unwrap().to_string_lossy().to_string();

// Step 3: for a follow-up, resume by ID — but be aware schema is undocumented and may break.
let _ = Command::new("agy")
    .args(["-p", &follow_up, "--conversation", &conversation_id])
    .output()?;
```

The race condition is real: if two Triumvirate workers each fire `agy -p` at the same moment, both will write a new transcript file and "newest by mtime" is not deterministic — there is no way today to scope `agy` to "this caller's workspace only." This is the precise concern Issue #7 was filed about.

### Side-by-side: multi-turn `--continue`
- gemini-cli: `gemini --resume <uuid>` resumes a specific session.
- agy: `agy -c` resumes the **most recent** conversation globally on the machine. If any other process or user invoked `agy -p` more recently, you resume their thread. There is no per-workspace scoping flag (the Issue #7 proposal floats `--workspace <id>` but it is not implemented).

### Migration verdict — answers to your specific questions

**(a) Can Triumvirate keep its existing subprocess-spawn + stdin/stdout + JSON-parse architecture with only flag/path changes?** No. The JSON parser must be deleted entirely and replaced with either (i) a plain-text reader that surfaces raw assistant text to the consumer with no tool-call visibility / no token-usage metering / no streaming, or (ii) a transcript-file reader that races every other `agy` invocation on the host.

**(b) Does it need a full parser rewrite due to schema changes?** Yes for the structured-output path. The `OutputFormat::Json` and `OutputFormat::StreamJson` enum variants in the Rust bridge are obsolete. Code that depended on the `session_id` field of the `init` event, the `tool_use`/`tool_result` events, or the final aggregated `stats` of the `result` event has no successor in `agy --print`. If those features are load-bearing for Triumvirate, the realistic alternatives are: keep gemini-cli alive on a paid Gemini API key (the only consumer-tier path that survives June 18), or bypass the CLI entirely and call the Gemini API or the new Antigravity SDK directly from Rust.

**(c) Gaps for a multi-threaded N-concurrent-conversations wrapper:**
- No caller-supplied conversation ID.
- No emitted conversation ID on `--print`.
- `-c/--continue` is host-global, not caller-scoped.
- No `--workspace` flag to scope `-c`.
- No `--output-format` to capture structured per-turn metadata.
- No stdin support for `-p`.
- No `-m/--model` to pin a model per call.
- Transcript files race because there is no atomic "tell me which file you just wrote."

**Risk assessment for a production migration in 4 weeks:**
- **High risk:** Multi-threaded concurrent-conversation correctness. Without Issue #7 being resolved upstream, any naive port can corrupt session state across workers. Severity: data-corruption-class for state-bearing agents.
- **Medium risk:** Token/cost accounting. Without `stats` events, Triumvirate cannot meter per-call token usage; it must approximate from prompt+response character counts or scrape it from a side-channel.
- **Medium risk:** Tool-call observability. Without `tool_use`/`tool_result` events on stdout, Triumvirate cannot show downstream consumers what the agent did.
- **Lower risk:** Auth. The keyring + URL+code flow works for SSH'd daemons after a one-time interactive bootstrap. Set up auth once on the prod host before June 18.
- **Lower risk:** Plain-text capture. Single-shot text-only flows (e.g., summarization, classification) port cleanly with just `gemini` → `agy` and dropping `--output-format` flags.

**Concrete checklist of wrapper code that must change:**
1. Replace binary name string literal `gemini` with `agy` everywhere.
2. Delete the `--output-format json` / `stream-json` argument-builder branches.
3. Delete the `--session-id` and `--resume` argument-builder branches; replace with `--conversation <id>` *only after* you have an ID, knowing the first call cannot create one.
4. Replace `--include-directories` with repeated `--add-dir` flags.
5. Replace `--yolo` with `--dangerously-skip-permissions` (semantics are broader; audit which tools were previously gated).
6. Replace any stdin piping (`Stdio::piped()` writing prompt bytes) with an in-process file read + passing contents as a `-p` flag argument; mind that the modern Linux x86_64 `getconf ARG_MAX` returns up to 2,097,152 bytes (2 MiB) but the hardcoded kernel lower bound is exactly 131,072 bytes (128 KiB).
7. Delete the JSONL stream-event parser (`serde_json::from_str` on each `BufRead::lines`).
8. Replace the JSON-shape data model (`response`, `stats.total_tokens`, etc.) with either a plain-string return type or a new struct sourced from the transcript file.
9. Move auth-state probes from `~/.gemini/settings.json` and `~/.gemini/oauth_creds.json` to the OS keyring (use a Rust keyring crate like `keyring` 2.x; namespace is `antigravity` / account `default`).
10. Move MCP server config probes from `~/.gemini/settings.json` to `~/.antigravity/mcp_config.json` and rename `url` → `serverUrl` for remote servers.
11. Set `--print-timeout` explicitly (default 5m may be too long for short prompts; too short for long refactors).
12. Add transcript-file discovery logic against `~/.antigravitycli/` with a file-lock or per-invocation working directory to avoid races (or accept that for now, multi-turn is single-threaded).
13. Decide model-routing policy: stay on Gemini-only (free under Ultra) by never invoking `/model`; or, if Claude/GPT-OSS are required, add a `gcloud auth application-default login` bootstrap step and configure a GCP project for Vertex Model Garden routing.
14. Re-test exit codes — gemini-cli used `0/1/42/53` (success, generic, input, turn-limit, per the [headless mode reference](https://geminicli.com/docs/cli/headless/)). agy's exit-code taxonomy is not documented; treat any non-zero as generic failure until empirical mapping is established.

---

## Recommendations

1. **This week:** Install `agy` on a non-prod machine, run `agy -p "ping"` and `agy -p "ping" --conversation $(uuidgen)` and snapshot the actual `agy --help` output for your records (the `--help` output is the only authoritative source; community tutorials are inconsistent). Confirm whether `--output-format` exists in your installed version — Google ships fast, and a v1.1 with JSON output could appear before your June 18 deadline.
2. **Next week:** Prototype a single-call Rust wrapper that drops all JSON parsing and treats `agy -p` as a plain-text RPC. Test it against your single-turn use cases first. Migrate single-turn workloads first; defer multi-turn until step 4.
3. **Two weeks out:** Decide your stance on the multi-thread concurrency gap. Three viable paths:
   - **Path A (safest, recommended for production daemon):** Keep one in-flight `agy` subprocess per conversation thread, spawned in long-lived interactive mode behind a PTY (use `portable-pty` or `expectrl` in Rust), and feed prompts through stdin to the TUI's `>` prompt. You give up structured output entirely but you preserve N independent threads.
   - **Path B (compromise):** Use one-shot `agy -p` per call, capture the freshest `~/.antigravitycli/<uuid>.json` after each call with a global Triumvirate-level mutex to serialize captures, persist the ID, and resume with `--conversation`. Limits Triumvirate to one concurrent agent dispatch at a time.
   - **Path C (escape hatch):** Migrate off the CLI entirely. Call the Gemini API directly from Rust using a paid Gemini API key (Triumvirate becomes responsible for the agent harness itself), or call the new Antigravity SDK (not yet well-documented for non-Go consumers). This is the cleanest long-term answer but the largest scope rewrite.
4. **Three weeks out:** Run a 48-hour soak test on a staging instance under realistic concurrency. The two specific failure modes to instrument for are (a) empty-stdout responses on long prompts (the "stdout bug" pattern) and (b) conversation cross-contamination when two threads invoke `agy -c` near-simultaneously.
5. **Cutover plan:** Flip prod to `agy` no later than 2026-06-15 (3 days of buffer). If the soak test reveals showstoppers, the fallback is to provision a paid Gemini API key and keep gemini-cli running for enterprise-tier service continuity — this is the only consumer-tier escape valve Google has left intact past June 18.

**Thresholds that should change these recommendations:**
- If Google merges a fix for [Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7) (caller-supplied conversation IDs) before June 18, Path B becomes vastly safer and is the right choice.
- If Google ships `--output-format json` in agy 1.x ([Issue #78](https://github.com/google-antigravity/antigravity-cli/issues/78) and adjacent feature requests suggest API-key/JSON output are on the roadmap), Path B becomes a near-drop-in for the existing Triumvirate JSON parser with only schema-name renames.
- If your Consumer AI Ultra Claude/GPT-OSS quota proves usable in practice (it has been reported as broken by some users — see [the Google AI Developers Forum thread on Ultra Claude quota](https://discuss.ai.google.dev/t/ultra-subscription-claude-model-quota-even-worse-than-pro/135870)), you can skip the Vertex setup. If it is not, plan a GCP project provisioning sprint alongside the CLI migration.

---

## Caveats

- **Official docs (`antigravity.google/docs/cli-overview`, `antigravity.google/docs/gcli-migration`) are a client-rendered SPA** and returned only metadata when fetched server-side (only the page title "Google Antigravity" and Open Graph image come through). The findings above come from: the [Google Developers Blog announcement by Dmitry Lyalin and Taylor Mullen](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/), the [public GitHub repo (`google-antigravity/antigravity-cli`)](https://github.com/google-antigravity/antigravity-cli/issues/7), official-tied secondary sources ([DeepWiki indexing of the README and CHANGELOG](https://deepwiki.com/google-antigravity/antigravity-cli)), and a small set of hands-on community write-ups. Where a fact rests on secondary sources, this report names them so you can weight reliability.
- **The `agy --help` output captured here is community-sourced** (a [LinuxCapable install guide](https://linuxcapable.com/how-to-install-google-antigravity-on-ubuntu-linux/) and the [gsd-build hands-on test in Issue #3782](https://github.com/gsd-build/get-shit-done/issues/3782)). Google's official CLI reference page is on the SPA. Always re-verify against your actual `agy --help` after install; flags may have changed in 1.0.1+ point releases.
- **The transcript file path `~/.antigravitycli/<uuid>.json`** is described in [Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7) as "the per-workspace symlink `<uuid>.json` files referenced in agy's project-tracking state" — the schema is explicitly undocumented and the filer warns it is "fragile to rely on." [SinanTufekci's MCP bridge](https://github.com/SinanTufekci/Claude-Code-Antigravity-CLI-MCP-Server/blob/main/test_smoke.py) does rely on it, but the exact path/schema is not in his public README either. Production code that depends on this path is at risk of silent breakage on agy upgrades.
- **The "stdout bug" in headless `agy -p`** is described colloquially in SinanTufekci's repo description; there is no upstream issue with that exact title. Some testers report it (especially for long outputs); others get clean stdout (`agy -p "2+2"` returns `4` per gsd-build #3782). It may be intermittent, length-dependent, or wrapper-context-specific.
- **No `ANTIGRAVITY_API_KEY` env var is supported in v1.0.x.** A feature request for API-key auth in headless environments ([Issue #78](https://github.com/google-antigravity/antigravity-cli/issues/78)) is open but unimplemented. If your CI/daemon truly cannot do an interactive first-time OAuth, you currently cannot use agy headlessly; you would need to either fall back to a paid Gemini API key (which keeps gemini-cli alive past June 18) or wait for Issue #78 to ship.
- **Closed-source concern:** Per [The Register's Brandon Vigliarolo (May 20, 2026)](https://www.theregister.com/ai-ml/2026/05/20/bye-bye-gemini-cli-google-nudges-devs-toward-antigravity/), *"Antigravity CLI isn't open source — at least not from what Google has published so far."* The repo `google-antigravity/antigravity-cli` is public for issues but is reportedly not a full source mirror. For a production migration this means you cannot fork-and-patch the CLI itself if you hit a blocker.
- **Pricing/quota for Claude under Ultra is contested.** Google's marketing positions AI Ultra as offering Claude Sonnet/Opus access; users on the [Google AI Developers Forum](https://discuss.ai.google.dev/t/ultra-subscription-claude-model-quota-even-worse-than-pro/135870) report that the Claude quota under Ultra is in practice often *worse* than under Pro, with frequent lockouts. Do not architect Triumvirate around heavy Claude routing through agy without first stress-testing your actual Ultra quota.
- **The May 19, 2026 announcement is recent.** All flag/path findings here are accurate as of late May 2026. Google has explicitly said in the [transition announcement](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/) that *"there won't be 1:1 feature parity right out of the gate"* and feature parity will improve over the migration window, and the 1.0.x line is changing fast. Re-verify any high-stakes claim against the actual installed binary before code freeze.

---

## Bibliography

- Google Developers Blog (Dmitry Lyalin, Taylor Mullen), *An important update: Transitioning Gemini CLI to Antigravity CLI*, May 19, 2026 — https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/
- Google Antigravity blog landing — https://antigravity.google/blog/introducing-google-antigravity-cli (SPA, contents not server-rendered)
- Google Antigravity migration docs — https://antigravity.google/docs/gcli-migration (SPA, contents not server-rendered)
- google-antigravity/antigravity-cli GitHub repo — https://github.com/google-antigravity/antigravity-cli
- Issue #7, *feat(--print): emit per-conversation ID so headless callers can resume specific sessions* — https://github.com/google-antigravity/antigravity-cli/issues/7
- Issue #78, *Support Gemini API Key (Google AI Studio) Authentication for Headless Environments* — https://github.com/google-antigravity/antigravity-cli/issues/78
- DeepWiki index of `google-antigravity/antigravity-cli` (README + CHANGELOG) — https://deepwiki.com/google-antigravity/antigravity-cli
- gsd-build/get-shit-done Issue #3782 (hands-on `agy --help` capture, macOS arm64) — https://github.com/gsd-build/get-shit-done/issues/3782
- LinuxCapable, *How to Install Google Antigravity on Ubuntu* (full `agy --help` dump) — https://linuxcapable.com/how-to-install-google-antigravity-on-ubuntu-linux/
- Arindam Majumder (DEV Community), *Antigravity CLI: A Hands-On Guide to Google's Terminal Coding Agent* — https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7
- Rich Rose (Medium / Google Cloud Community), *Getting started with Antigravity CLI* — https://medium.com/google-cloud/getting-started-with-antigravity-cli-3565d5db1e92
- Romin Irani (Medium / Google Cloud Community), *Antigravity CLI Tutorial Series* — https://medium.com/google-cloud/antigravity-cli-tutorial-series-12b46cfe3bf2
- Agentpedia Codes, *Antigravity CLI Deep Dive* — https://agentpedia.codes/blog/antigravity-cli-deep-dive
- SinanTufekci, *Claude-Code-Antigravity-CLI-MCP-Server* (transcript-file workaround) — https://github.com/SinanTufekci/Claude-Code-Antigravity-CLI-MCP-Server/blob/main/test_smoke.py
- NoeFabris, *opencode-antigravity-auth API spec* (OAuth scopes) — https://github.com/NoeFabris/opencode-antigravity-auth/blob/main/docs/ANTIGRAVITY_API_SPEC.md
- The Register (Brandon Vigliarolo), *Bye-bye, Gemini CLI; Google nudges devs toward Antigravity* — https://www.theregister.com/ai-ml/2026/05/20/bye-bye-gemini-cli-google-nudges-devs-toward-antigravity/
- TechCrunch (Ivan Mehta), *Google launches Antigravity 2.0 with an updated desktop app and CLI tool at IO 2026* — https://techcrunch.com/2026/05/19/google-launches-antigravity-2-0-with-an-updated-desktop-app-and-cli-tool-at-io-2026/
- Google official blog, *I/O 2026 developer highlights* (Ultra "5X higher usage limit" framing) — https://blog.google/innovation-and-ai/technology/developers-tools/google-io-2026-developer-highlights/
- Google One Help, *Get Google AI Ultra benefits* (25,000 monthly AI credits) — https://support.google.com/googleone/answer/16286513?hl=en
- Google AI Developers Forum, *Ultra Subscription: Claude Model Quota Even Worse Than Pro* — https://discuss.ai.google.dev/t/ultra-subscription-claude-model-quota-even-worse-than-pro/135870
- Gemini CLI headless mode reference (legacy baseline) — https://geminicli.com/docs/cli/headless/
- Gemini CLI configuration reference (legacy baseline) — https://geminicli.com/docs/reference/configuration/
- google-gemini/gemini-cli Pull Request #10883 (stream-json schema) — https://github.com/google-gemini/gemini-cli/pull/10883
- google-gemini/gemini-cli MCP server docs (legacy ~/.gemini paths) — https://google-gemini.github.io/gemini-cli/docs/tools/mcp-server.html
- Arm Learning Paths, *Gemini CLI* (legacy `~/.gemini/settings.json` example) — https://learn.arm.com/install-guides/gemini/
- google-gemini/gemini-cli Discussion #27274 (Gemini CLI lead Dmitry Lyalin's transition Q&A) — https://github.com/google-gemini/gemini-cli/discussions/27274
- Linux Command Library, *agy man* (VS-Code-style launcher conventions for the IDE binary) — https://linuxcommandlibrary.com/man/agy