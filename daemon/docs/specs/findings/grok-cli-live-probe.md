# Live probe: what the installed `grok` binary actually accepts

**Date:** 2026-08-30 · **Binary:** `grok 1.0.13 (5e9a58528b76)`, macos-aarch64, installed to `~/.grok/bin`, symlinked
`~/.local/bin/grok`
**Cost:** zero tokens, no auth required. Everything below is from `--help` and one deliberate invalid-value error.
**Settles:** the `-s` versus `--resume` ambiguity that both peer reviewers named as the guide's least validated
assumption, and which neither could resolve from documentation.

---

## 1. THE HEADLINE: `streaming-json` **is** ACP

```
$ grok --output-format definitely-not-valid -p x
error: invalid value 'definitely-not-valid' for '--output-format <OUTPUT_FORMAT>'
  [possible values: plain, json, streaming-json, streaming-messages-json]
```

```
Possible values:
- plain
- json
- streaming-json:          NDJSON: one ACP session update per line, the agent's native format
- streaming-messages-json: NDJSON in the Anthropic Messages API wire format

[default: plain]
```

**"one ACP session update per line, the agent's native format."**

The guide frames v1 as *headless NDJSON* and treats *ACP* as a deferred v2 alternative (constraint #4, §11 backlog).
**They are not alternatives.** `streaming-json` payloads *are* ACP session updates. The difference between the guide's
v1 and `grok agent stdio` is the **transport** (stdout lines from a spawned process versus JSON-RPC over a leader
socket), not the **vocabulary**.

The guide's own §3.3 event table is the tell, now that we know what to look for: `tool_call`, `tool_call_update`,
`plan`, `available_commands` are ACP session-update names, not Grok inventions.

**What this means for the decision:**

- A `GrokStreamParser` written for §3.3 is substantially **the same parser** a future `grok agent stdio` client would
  need. The event shapes carry over; the framing does not.
- Codex's "duplication is correct, build the parser" and Gemini's "go ACP" were much closer than the debate made them
  look. For Grok specifically, **building the parser IS building most of the ACP client.**
- This materially de-risks parking the orchestrator-inversion work. Slice C is not throwaway effort against a future
  protocol migration; it is a down payment on it.

> Caveat, stated plainly: this establishes that Grok *calls* its NDJSON "ACP session updates." It does **not**
> establish that the per-line JSON is byte-compatible with the ACP spec, nor that `codex app-server proxy` speaks the
> same dialect. Confirming that needs a captured stream, which needs auth.

## 2. `-s` versus `--resume`: RESOLVED, and the guide's rule is correct

Verbatim from `grok --help`:

```
-s, --session-id <SESSION_ID>
        Use a specific session UUID for a **new** conversation (must be a valid UUID and must not
        already exist under the target session directory). With `--resume`/`--continue`, only valid
        together with `--fork-session` (names the forked session). Does not resume existing sessions,
        use `--resume` / `--continue` instead

-r, --resume [<SESSION_ID_OR_TITLE>]
```

**The guide's §3.2 implementation rule is correct:** first turn `-s <uuid>`, later turns `-r <id>`, never `-c`. The
upstream doc conflict it flagged is resolved by the binary: `-s` never resumes.

**Codex's feared failure mode is milder than predicted.** He warned that a rejected `-s` mid-flight could leave the
daemon holding "a Grok id that never existed." The constraint is *must not already exist*, so passing `-s` for a known
id is a **hard argument error before any token spend**, not a silent divergence. His provisional-id rule is still
worth adopting, but the failure is loud, which is the good case.

**One flag ordering constraint the guide gets wrong by omission:** `-s` is *"only valid together with
`--fork-session`"* when combined with `--resume`/`--continue`. `build_grok_invocation` must therefore **never** emit
`-s` and `-r` together. The guide's arg order (step 5) already does an if/else, so it is correct by construction, but
the reason should be in the spec, and the test should assert it because the CLI will reject the combination.

## 3. NEW HAZARD the guide does not mention: bare `--resume` is `--continue`

Note the brackets: `-r, --resume [<SESSION_ID_OR_TITLE>]`. **The argument is optional.**

The guide bans `--continue` in strong terms (§3.2: *"Do not use `--continue`. That is 'most recent session in this
cwd' and will cross-talk between Triumvirate sessions."*). Correct. But it never says that **bare `-r` has the same
hazard**.

So a bug where the persisted session id is empty, `None`, or an unwrapped default does not fail. It silently becomes
"resume the most recent session in this cwd," which is precisely the cross-talk the guide is trying to prevent,
reached through the flag the guide recommends.

**Required, and not in the guide:** `build_grok_invocation` must refuse to emit `--resume` without a non-empty id.
That is a `bail`, not a fallback. Add an acceptance test: `resume=true` with `session_id=None` is an error, never a
bare `-r`.

Also note `--resume` accepts a **title**, not just a UUID. Anything passing a Triumvirate session *name* where an id
belongs will silently resolve to the wrong conversation rather than erroring.

## 4. Surface the guide never mentions

| Flag / value | Why it matters |
|---|---|
| `--output-format` default is **`plain`** | Must always be passed explicitly. The guide does this, but nothing in it says the default is unusable, so a dropped flag degrades to unparseable prose rather than failing. |
| `streaming-messages-json` | A fourth format, **"NDJSON in the Anthropic Messages API wire format."** Grok can emit Anthropic Messages shape. Not needed for v1, worth knowing before writing a bespoke parser. |
| `--permission-mode <MODE>` with `default, acceptEdits, auto, dontAsk, bypassPermissions, plan` | The guide's §3.1 only discusses `--always-approve` and `--sandbox`. There is a much richer permission surface, and it belongs in the forbidden-extra-flags list since Triumvirate owns approval policy. **`plan` mode is worth evaluating for the consult default**, which is exactly the read-leaning posture §3.1 wanted from `--sandbox` and could not confirm. |
| `--json-schema` (implies `--output-format json`) | Structured output constrained to a schema. Not v1, but a better fit for gate-style consults than parsing prose. |
| `--fork-session` | Already on the guide's forbidden list. Now we know why it exists: it is the only legal way to combine `-s` with a resume. |

## 5. Changes required to the guide before Slice B is written

1. **§3.2:** state that `-s` and `-r` are mutually exclusive without `--fork-session`, and that the CLI enforces it.
2. **§3.2:** add the bare-`-r` hazard. `--resume` with no id equals `--continue`. Builder must `bail` on an empty id.
3. **§3.2:** note `--resume` accepts a title, so a session *name* passed where an id belongs resolves silently wrong.
4. **§3.7:** add `--permission-mode` and `--json-schema` to `FORBIDDEN_EXTRA_FLAGS`.
5. **§3.1:** evaluate `--permission-mode plan` as the consult default. It may be the read-leaning posture the guide
   wanted and could not confirm existed.
6. **Constraint #4 and §11:** correct the framing. `streaming-json` is ACP-native, so v1 is not an alternative to ACP,
   it is ACP over a different transport. The v2 item is "switch transport to the leader socket," not "adopt ACP."

## 6. Still not established

- The actual per-line JSON shapes. §3.3's fixtures are **unverified** against this binary. Capturing one real stream
  needs `XAI_API_KEY` or `grok login`, neither of which is present (`~/.grok` has no auth file).
- Whether Grok's ACP dialect and Codex's app-server dialect are compatible.
- Exit-code behavior (§3.6), untested.

**When auth is available, the cheapest confirming run is a single trivial prompt captured to a fixture file:**

```bash
grok --no-auto-update --no-alt-screen --output-format streaming-json \
     --cwd /tmp -s "$(uuidgen)" -p 'reply with the single word pong' \
  | tee daemon/crates/agent-adapter/tests/fixtures/grok-streaming-$(date +%Y%m%d).jsonl
```

That one command validates §3.3, §3.4, §3.6, and the `end.sessionId` contract at once.
