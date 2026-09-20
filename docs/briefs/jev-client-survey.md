# Jev clients: what actually exists, from their own docs

**Compiled:** 2026-09-19. Every row below comes from the package's own README or registry
metadata, fetched directly, not from secondhand description. Raw docs saved during the survey.

**Correction to earlier notes in this repo:** the handoff's reference sheet was wrong in three
places. `choice` takes `criteria`, not `options` (a request with `options` returns 422). There is
an official Python SDK, which no earlier note mentioned. And `jtsang4/jev-cli` is documented as
defaulting to TypeSafe's own API, not as rejecting it.

---

## Official, from TypeSafe AI

| Thing | Where | Notes |
|---|---|---|
| HTTP API | `POST https://api.typesafe.ai/v1/systemone` | One endpoint. Everything else wraps this. |
| Python SDK | `typesafe-sdk` 0.7.0 on PyPI | Python 3.10+. `TypeSafeClient().system_one(state=, questions=)`. Repo `github.com/typesafe-ai/typesafe-sdk-python`. |
| JavaScript SDK | `@typesafe-ai/sdk` 0.6.0 on npm | Node 20+. **Zero dependencies.** ESM, CJS and TS declarations. Answer types inferred from questions. |

**There is no official CLI, no official Rust SDK, and no official MCP server.** Everything in the
sections below is community-built and unaffiliated.

Two details the SDK quickstarts settle that the API docs did not: `state` accepts an **object**,
not only a string, and `criteria` values may be `null`, so a choice can be bare option names with
no rubric text.

**Name hazard:** `typesafe-ai` on PyPI is **not** TypeSafe's. It is a redirect shim published by a
third party that depends on the real `typesafe-sdk`. It sits on the obvious guess.

---

## Unofficial CLIs

| CLI | Language | Install | Dependency weight | Command surface | Key storage |
|---|---|---|---|---|---|
| **`jevctl`** (Nasrallah-AL) | TypeScript | `npm i -g jevctl`, or `npx jevctl` | **3**: zod, commander, and the official SDK | verify, screen, classify, extract, find, rerank, match, route, ask, compact, batch, auth, config, models, update | OS keychain or env |
| **`jev-cli`** (tumf) | Python 3.13+ | `uv tool install jev-cli` | **64 packages** | noul, choice, score, run, auth, install-skills | env wins, else 0600 file |
| **`jev-studio`** (utk2103) | Python 3.10+ | `pip install jev-studio` | not audited | same surface as jevctl | keychain, env, OpenRouter, Cloudflare |
| **`jev-cli`** (jtsang4) | TS, Node 22+/Bun | `npm i -g @jtsang/jev-cli` | not audited | `eval` only, plus config and doctor | config file |
| **`jev-cli`** (MrHodlX) | **Rust** | `cargo install --git` | the `jev-sdk` crate | predict, choose, score | `~/.jevcli/jev.conf` 0600, or `.env` |

### What distinguishes them

**`jevctl`** is the most developed and the only one built **on the official SDK**, so its request
construction is the vendor's. Worth noting for this repo specifically:
- `--dry-run` prints the exact request without calling the API.
- Exit code 2 when `--fail-on` matches, so it composes as a CI gate.
- Output as table, json, jsonl, md, csv, tsv, plus `--pluck` for one value. JSON field names are
  documented as a stable contract.
- "A response that is not one answer per question is `Malformed response`, never a silent pass."
  That is the discipline this repo already enforces elsewhere.
- Ships a Claude Code plugin adding `/jev:*` commands, **and a hook that replaces Claude Code's
  compaction summary with `jev compact`**. That hook is invasive and should be considered
  separately from the CLI itself.

**`jev-cli` (tumf)** is the one currently installed on this machine. Its distinguishing features
are the bundled `jev-mcp` stdio server and four selectable providers, including a `custom` one
that points at any Jev-compatible proxy endpoint. Its cost is 64 packages, almost all dragged in
by the MCP server SDK, for a CLI whose own source is two files.

**`jev-cli` (MrHodlX)** is agent-facing by design: stdout carries only the answer, diagnostics go
to stderr, thresholds and pass/fail are built into the output shape. It is also the only Rust one.

**`jev-cli` (jtsang4)** is minimal, a single `eval` command. It uses `"type": "boolean"` rather
than `"noul"` and returns `probability`, so its wire shape differs from the others. Its `doctor`
command sends one real evaluation to prove the key works rather than only checking that one is set.

---

## MCP servers

| Server | Language | Notes |
|---|---|---|
| **`typesafe-mcp`** (itsmostafa) | **Go, single static binary** | One read-only `evaluate` tool. No Node, no Python, no runtime. Retries 429 and 529 with backoff. 60s timeout, rejects responses over 16 MiB rather than truncating. Ships usage guidance to the client so the agent writes better questions. `evaluate setup mcp` auto-registers with Claude Code, Claude Desktop and Codex. |
| **`jev-mcp`** (tumf) | Python | Bundled with `jev-cli`, already on this machine. |
| **`jev-studio`** (utk2103) | Python | Bundled with `jev-studio`. Repo badge says "Under Development". |

**Caution on `typesafe-mcp`:** its one-command setup "carries over every `TYPESAFE_*` variable in
your shell," which means writing the key into the MCP config file. That is a second copy of a
secret this repo has decided lives in `.env` alone. Its install is also `curl | sh`, though
`go install` is offered as an alternative.

---

## Rust crates

All four are unofficial, all are version 0.1.x, all were published within days of each other, and
all have double-digit download counts.

| Crate | Version | Downloads | Repo | docs.rs |
|---|---|---|---|---|
| `jev-sdk` | 0.1.0 | 19 | portlandhodl/jev-sdk | no |
| `typesafe-sdk` | 0.1.2 | 44 | codeitlikemiley/typesafe-sdk-rust | no |
| `typesafe-ai` | 0.1.0 | 18 | Twister915/typesafe-ai | **yes** |
| `jev` | 0.1.0 | 29 | gitlab.com/porky11/jev | no |

This is the section that decides the Triumvirate question, and it decides it against adopting.
Four brand-new crates by four unrelated authors, none official, none with meaningful adoption,
wrapping a single HTTP POST that `reqwest` already in this workspace can make directly.

---

## One genuinely novel approach

`jev` 0.3.0 on PyPI (a different thing from the `jev` crate) is not a CLI or a client. It is a
decorator that compiles a Python function signature into a Jev request:

```python
class Triage(BaseModel):
    department: Literal["billing", "technical", "sales"]
    is_urgent: bool
    frustration: int = Field(ge=0, le=2)

@jev.fn
def triage(ticket: str) -> Triage:
    """A customer support ticket:

    {{ ticket }}
    """
    return triage.state()
```

The return annotation's field types compile to question types: `bool` to noul, `Literal` and
`Enum` to choice, and `int` or `float` with `Field(ge=, le=)` to score. The docstring is rendered
as a Jinja2 template and sent as `state`. Answers are validated back through pydantic.

Requires Python 3.14 and its install instructions are `uv sync`, which suggests it is not really
packaged for outside consumption yet. Listed because the idea is worth stealing even if the
package is not worth depending on.

---

## What to use, per job

| Job | Use | Why |
|---|---|---|
| Triumvirate's own code (Rust) | **Write it.** `reqwest` is already a workspace dependency | One POST. Four unofficial 0.1 crates with about 20 downloads each is a worse dependency than a hundred lines we own and can pin. |
| Python: the wiki report, scripts, Prefect | **`typesafe-sdk`**, the official one | Vendor-maintained, Python 3.10+, no CLI subprocess in the middle. |
| Shell exploration | **`jevctl`** | 3 dependencies against 64, built on the official SDK, `--dry-run`, best documentation, and `npx jevctl` needs no install at all. |
| Jev callable in a Claude Code session | **`typesafe-mcp`** | A single Go binary with no runtime beats a 64 package Python tree. Decide the key question first. |
| Currently installed | `jev-cli` (tumf) | Dominated by `jevctl` on every axis that matters here except the bundled MCP server, and `typesafe-mcp` beats that too. |
