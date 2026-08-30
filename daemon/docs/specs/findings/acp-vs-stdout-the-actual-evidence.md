<!-- EMDASH_OK: this file quotes codex_capabilities.rs:161 verbatim, and that source line
     contains an em dash written by its author. The argument in section 3 turns on what that
     comment actually says, so the quote is reproduced exactly rather than recast. -->

# "app-server is the painful one": what the pain actually is

**Date:** 2026-08-30
**Why this exists:** the Grok guide's constraint #4 forbids starting with ACP, supported by one sentence: *"Codex
already has a separate `app-server` path and it is the painful one."* Gemini's review says that constraint is the
expensive mistake. Neither claim was evidence-backed, so this is the evidence.
**Status:** investigation only. No code changed.

---

## 1. The pain is real, and it is documented in the code

`agent_exec.rs:2252`, verbatim:

```rust
// If the configured protocol is `app-server` but the installed codex
// binary no longer implements the JSON-RPC stdio server there (0.121+
// turned that subcommand into a tooling namespace), fall back to `exec`
// with a loud warning. Without this, every call silently exits status 1.
```

**Upstream removed the stdio JSON-RPC server out from under this repo**, and until the capability probe and downgrade
were added, *every app-server call silently exited status 1*. That is a genuine, expensive failure and it fully
justifies wariness. The guide's instinct is not baseless.

## 2. The app-server path is dead code on this machine today

Three independent reasons, all verified:

1. **Default is `exec`.** `codex_protocol()` (`mcp-bridge/src/lib.rs:409`) returns `"exec"` unless
   `TRIUMVIRATE_CODEX_PROTOCOL=app-server` is set explicitly.
2. **Even when set, it downgrades.** `agent_exec.rs:2258` forces `exec` whenever
   `!caps.has_app_server_protocol_server`.
3. **That capability is false on the installed binary.** `codex-cli 0.145.0`.

So `codex_app_server.rs` (292 lines, including the only `ApprovalRequest` machinery in the tree) is currently
unreachable. Any belief that this repo "has an ACP-shaped path" is false in practice.

## 3. But the protocol did not vanish. It moved, and the probe cannot see the new shape

This is the finding that changes the argument.

```
$ codex --version
codex-cli 0.145.0

$ codex app-server --help
[experimental] Run the app server or related tooling
Commands:
  daemon                Manage the local app-server daemon
  proxy                 Proxy stdio bytes to the running app-server control socket
  generate-ts           [experimental] Generate TypeScript bindings for the app server protocol
  generate-json-schema  [experimental] Generate JSON Schema for the app server protocol
```

```
$ codex app-server proxy --help
Proxy stdio bytes to the running app-server control socket

$ codex app-server daemon --help
Commands:
  bootstrap               Install durable local app-server management for SSH-driven use
  start                   Start the local app server daemon if it is not already running
  restart / enable-remote-control / disable-remote-control
```

**There is still a stdio transport to a JSON-RPC app server.** It was restructured from *"`app-server` IS the inline
stdio server"* into *"a persistent `app-server daemon`, reached through a thin `app-server proxy` stdio pipe."*

Now read the probe that decides the capability is gone, `codex_capabilities.rs:159`:

```rust
fn looks_like_protocol_server(help: &Option<String>) -> bool {
    let Some(text) = help else { return false };
    // New-shape tooling namespace — explicitly not the server we want.
    if text.contains("generate-ts") || text.contains("generate-json-schema") {
        return false;
    }
    // Old-shape protocol server surfaced a listen/transport option.
    text.contains("--listen") || text.contains("--transport") || text.contains("stdio")
}
```

**The early return fires first and wins.** The 0.145.0 help text contains `generate-ts`, so the function returns
`false` before it can reach the `stdio` check, which that same help text *would* satisfy: the `proxy` line literally
reads "Proxy **stdio** bytes."

The probe is not detecting "the protocol server is gone." It is detecting "this is the new shape," and the code then
treats new-shape as absent. That was a defensible reading when 0.121 first broke things and the replacement was
unclear. It is a **stale conclusion** now that the replacement is visible and named.

## 4. What this does to the ACP argument

| Claim | Verdict |
|---|---|
| "app-server is painful" | **True.** Upstream broke it, silently, and it cost real debugging. |
| "so ACP is the wrong bet for v1" | **Not established.** The pain was an upstream restructure this repo has not tracked, not an inherent property of the protocol approach. |
| "this repo already has an ACP path to reuse" | **False in practice.** It is unreachable dead code on 0.145.0. |
| "the new shape is better for orchestrator-agnostic use" | **Plausible and untested.** A persistent daemon plus a thin stdio proxy is closer to what Gemini argues for than the old inline server was. |

**Gemini's position gets stronger, not weaker.** Its argument was that per-turn stdout scraping cannot express a
symmetric relationship, and that `ApprovalRequest` type in `codex_app_server.rs` is the proof: it is the only event in
the tree that expects an answer back. The reason that machinery is unused is not that the approach failed. It is that
the transport moved and nobody followed.

**The guide's caution also survives**, in a narrower form: the concrete lesson from 0.121 is that a protocol surface
can be restructured without warning, so any protocol path needs a capability probe and a graceful downgrade **from
day one**. This repo learned that the expensive way and the machinery exists.

## 5. What is NOT established, and must be tested before anyone acts

- **Whether `codex app-server proxy` speaks the same JSON-RPC that `CodexAppServerParser` already parses.** Not
  checked. The parser may need changes, or may work unmodified. This is a bounded experiment: start the daemon, pipe
  a handshake through the proxy, compare against the parser's expectations.
- **Whether `grok agent stdio` speaks a compatible ACP dialect**, or merely a similarly-shaped one. The Grok guide
  never characterizes it beyond "JSON-RPC."
- **What ACP's current spec version is.** Claude's own knowledge of it runs to roughly May 2026 and the guide is dated
  August. Do not treat that summary as current.

## 6. Recommended next step, cheapest first

**Run the proxy experiment before deciding the Grok scope.** It is a small, bounded test that answers the actual
question:

```bash
codex app-server daemon start
codex app-server proxy   # feed it an initialize request, capture the response
```

If the existing parser handles that stream, then "app-server is painful" is resolved, this repo regains a working
protocol path for free, and the Grok ACP-versus-NDJSON decision can be made on evidence. If it does not, the guide's
constraint #4 is vindicated with a real reason behind it instead of a one-line assertion.

Either outcome is worth more than the argument currently is. And it is worth doing **regardless of whether Grok is
built at all**, because it determines whether Codex approvals work.

## 7. Side effect worth fixing independently

`looks_like_protocol_server` should be revisited whether or not any of the above happens. As written it will report
`false` for every future Codex that keeps `generate-ts` in the namespace, even if a perfectly good stdio server sits
beside it. At minimum the early return should be narrowed to "tooling-only namespace" rather than "namespace contains
tooling."
