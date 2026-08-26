# /goal: Pantheon corpus remediation

Paste this to start or resume the work. It is written so a session with no prior context can execute it.

---

## READ THESE TWO FILES FIRST (compaction survival)

**If you have just lost context, this file alone is not enough. Live state lives in two other places:**

1. **`docs/pantheon/gcp-test-plan/REVIEW-PROGRESS.md`** is the authoritative state file. It carries the RESUME HERE
   pointer (which queue item, which section, what to do next), the queue status table, the per-document section lists,
   and every synthesized finding. **Read it before doing anything. Trust its RESUME HERE pointer over anything below,
   because this file is not updated after every section and that one is.**
2. **`docs/pantheon/gcp-test-plan/review-raw/`** holds every peer's verbatim output, one file per document-section.
   Synthesis loses detail; that directory does not. Consult it before re-asking a peer anything.

**Never re-review a section already marked DONE in `REVIEW-PROGRESS.md`. Never start over.**

Everything is committed and pushed after each section, so `git log --oneline docs/pantheon/` is a third independent
record of what actually happened.

### STATUS: THE QUEUE IS COMPLETE (2026-08-26)

**All nine items done.** 54 commits, 281 findings, 26 verbatim peer records. Every document reviewed section by
section by three peers, then rewritten, split, or archived with its purpose recorded.

| Outcome | Documents |
|---|---|
| Rewritten | `00-MASTER-PLAN.md`, `10-PREFLIGHT.md`, `20-EVIDENCE-BUNDLE-SPEC.md`, `30-DECISION-RULES.md`, `runbooks/gate-0-plumbing.md`, `runbooks/gate-6-airgap-sanity.md`, `twin-review-synthesis.md` |
| Split | `local-inference-buy-vs-rent.md` became `POLICY-rent-first.md` (the policy) plus a demoted analysis |
| Archived with extraction | hardware decision + provenance, model selection, graduated plan, six gate runbooks |
| Created | `SIZING-SWEEP-METHOD.md`, `EXTRACTED-from-archive.md`, `fast-vm-startup-strategies.md`, `POLICY-rent-first.md` |

**Do not re-run this queue.** It is finished. Two things remain, and neither is more reviewing.

### WHAT IS ACTUALLY NEXT

**1. Execute something.** The corpus is correct on paper and **nothing has ever been run.** April 2026 produced a
correct critique nobody acted on; this produced a correct corpus nobody has acted on. Both are paper, and the second
is not better than the first until something executes. **Run Track A on the local Lenovo. It costs nothing.**

Blockers named in `10-PREFLIGHT.md`: the fixtures do not exist, the container images are not built, and the harness
scripts are called at `/opt/pantheon-harness/`, a path from a GCP image that was never built. **That last one is the
predicted first-run failure.**

**2. Five findings are open that this review missed.** They were caught by the April 2026 twin review, never fixed,
and **not re-found here either.** They are recorded in `twin-review-synthesis.md` and are covered nowhere in the
rewritten corpus:

1. Human-in-the-loop operating protocol: what is the operator's control surface during an autonomous run?
2. Continuous model evaluation and quality regression detection.
3. Source-of-truth reconciliation under concurrent edits.
4. **Customer discovery as a gate before building.** Nothing anywhere gates building on demand existing.
5. Security and compliance for client codebases across their whole lifecycle.

**Also unfinished:** `POLICY-rent-first.md` carries reopening thresholds marked **NOT SET** on purpose. Fill them from
measured rental spend and utilization once the sizing sweep produces real numbers. Inventing a plausible figure
instead would repeat the defect that document exists to correct.

### Operating constraints learned the expensive way, obey them

- **DeepSeek cannot read files** through `ask_agent` and will correctly refuse rather than fabricate citations. Paste
  the section text to it, and give it ONE focused question at `reasoning_effort: low`.
- **The MCP bridge times out at 180 seconds.** Two long DeepSeek prompts were lost to this. The env var is read
  client-side, so it cannot be raised without restarting the session. Keep peer calls to roughly one section.
- **There are about 3 twin worker slots.** Do not fan out wider. A 10-agent Workflow was tried, stalled at 0 of 10
  complete, and wasted roughly 500k tokens producing nothing. Three direct `ask_agent` calls per section is correct.
- **Do not use the Workflow tool for this.** It is a loop over a list.

### STANDING RULE: never delete and hand-wave. It was there for a reason.

**Owner instruction, 2026-08-26, and it is not optional.** When a peer recommends deleting something, the review is
not finished until you have answered, in writing, in `REVIEW-PROGRESS.md`:

1. **What was this for?** What problem did the original author put it there to solve?
2. **Does that problem still exist?** If yes, deleting it creates a gap.
3. **What replaces the capability?** Name the replacement, or say plainly that the capability is being dropped and
   why that is acceptable.

Peers are good at spotting that something is wrong and bad at noticing what it was load-bearing for. Two things have
already been wrongly deleted this way: the PD snapshot (which was the only answer to a 30-minute cold start) and,
nearly, the pre-registered hypothesis structure (which is the only thing in the corpus preventing post-hoc
rationalization of results). **Separate the content from the structure. Usually the content is dead and the structure
is the good part.**

Record your own dissent from a peer when you have one. Agreement is not the goal, correctness is.

### Two corrections already made, do not re-introduce them

- **"Air-gap" does not have to mean literal isolation.** The working definition is not connected to the public
  internet, and Private Google Access rides Google's private backbone, so **PGA does not need to be removed**. The
  problem is terminology: never call the GCP test an air-gap proof in anything a client reads.
- **A "delete this phase" verdict is incomplete until you ask what replaces the capability.** The PD-snapshot delete
  verdict was accepted on a bad comparison and had to be corrected. See the Phase 5 entry in `REVIEW-PROGRESS.md`.

---

## GOAL

Every document in the Pantheon corpus has been reviewed by three peers section by section, rewritten against their findings, verified once, and the preflight has actually been executed against real GCP.

## DONE WHEN

All of these are observably true:

1. Every file in THE QUEUE below has a peer-review pass recorded and a rewrite committed.
2. `gcloud projects describe pantheon-validation-v1` succeeds. The project exists.
3. `10-PREFLIGHT.md` Phase 1 has been run start to finish and every step either succeeded or failed loudly with the failure recorded.
4. No document in the corpus still asserts a capability that has no implementation. Specifically: the hard-kill Cloud Function either has deployable source at a stated path, or every claim about it is deleted.
5. `HARDWARE_DECISION.md` no longer carries `Status: ACCEPTED`.
6. `REVIEW-HIT-LIST.md` has every CRITICAL item marked resolved with the commit that resolved it, or explicitly deferred with a reason.

## THE PROCESS

Per document, in this exact loop. Do not improvise a different one.

1. **Section by section.** Split the document by its `##` headings. One section at a time. Never send a whole large document to a peer.
2. **Three peers per section.** Codex and Gemini read from disk (give absolute path plus line range). DeepSeek cannot read files, so paste the section text into its message. Fire all three in one message so they run concurrently.
3. **Report findings after each section** before moving to the next. Short. What each peer flagged, where they agreed, what is actually wrong.
4. **After the last section, rewrite the document.** Keep its existing structure and coverage. Do not invent a new shape. Fix what the peers found. Commit.
5. **One narrow verification pass.** Scope it to: "here are the N findings from round one, did the rewrite fix each one, and did it introduce anything new." This is a check against a fixed list, NOT another open hunt. Open-ended rediscovery never terminates.
6. **Then stop reviewing that document and move to the next.** If verification surfaces something CRITICAL, fix that specific thing. If it surfaces medium or stylistic opinions, log them in `REVIEW-HIT-LIST.md` and move on. Do not rewrite twice.

**Termination rule:** the document is not the deliverable. Running it is. Preflight is $0 GPU. When `10-PREFLIGHT.md` is rewritten and verified, execute Phase 1 for real and let gcloud falsify what three models could not. The root cause of this entire mess is that nothing was ever executed, so every wrong claim survived four months unchallenged. More review is the same disease.

## THE QUEUE

In priority order. Priority reflects the rebuilt plan, where Track A (gate-0 then gate-6) is the product claim and gates 1 through 5 and 7 are demoted to rows in a pricing sweep.

| # | Document | Status |
|---|---|---|
| 0 | `HARDWARE_DECISION.md` + `_provenance.md` | **COMPLETE 2026-08-26** (commit `401fdde`). Archived, ACCEPTED stripped, TPS floor extracted. |
| 1 | `gcp-test-plan/10-PREFLIGHT.md` | **COMPLETE 2026-08-26.** 11 sections, 113 findings, rewritten 778 to 405 lines, verified 35/36. |
| 2 | `gcp-test-plan/20-EVIDENCE-BUNDLE-SPEC.md` | **IN PROGRESS**, unit 1 of 4 done. Client-facing artifact spec. 4 units: 1-47, 48-321, 322-390, 391-end. |
| 3 | `gcp-test-plan/30-DECISION-RULES.md` | Delete rules 1, 2, 3, 6, 10 (pure CapEx triggers). Keep 4, 5, 7, 8 (validate software, substrate agnostic). Rule 10 is the auto OPEX-to-CAPEX trigger and is the most dangerous line in the corpus. |
| 4 | `runbooks/gate-0-plumbing.md` | Track A. Known broken: delete command sits after `exit` (line 284) so self-destruct never runs; compose step is a literal placeholder. |
| 5 | `runbooks/gate-6-airgap-sanity.md` | Track A, and the product claim. Known broken: says "ZERO outbound" in purpose, passes at "<= 5 packets" in decision rule; permits Private Google Access so it is restricted-egress not air-gap; no IPv6; `g4-standard-32` is not a real machine type. |
| 6 | `local-inference-buy-vs-rent.md` | Standing policy lives here (section 6). Needs the TPS floor folded in. |
| 7 | `model-selection.md`, `graduated-gcp-validation-plan.md` | April vintage, stale hardware and model landscape. |
| 8 | `runbooks/gate-1` through `gate-5`, `gate-7` | Lowest priority. Demoted to pricing-sweep rows. Consider collapsing rather than rewriting each. |
| 9 | `twin-review-synthesis.md` | Historical. Probably archive. |

`00-MASTER-PLAN.md` is already rewritten. It gets the narrow verification pass only, at the end, once the documents it points at are true.

## HARD CONSTRAINTS

Learned the expensive way in this session. Violating these repeats a known failure.

- **DeepSeek has no filesystem access** through `ask_agent`. It will correctly refuse to cite files rather than fabricate. Paste content to it or use it only for reasoning.
- **The daemon times out at 180 seconds.** Codex died on 102KB of runbooks. Keep each peer call to roughly one section, well under 25KB.
- **There are about 3 twin worker slots.** Do not fan out wider than that. Ten parallel agents each wanting three twin calls produces a queue, not throughput, and burns the full setup cost of every agent for nothing.
- **Do not use the Workflow tool for this.** It was tried, stalled at 0 of 10 complete, and wasted roughly 500k tokens. Three direct `ask_agent` calls per section is the correct width.
- **Verify peer claims before writing them into a document as fact.** Codex gives line numbers; check them. They have all been accurate so far, which is a reason to keep checking, not to stop.

## WHAT NOT TO DO

- Do not build an orchestration layer for this. It is a loop over a list.
- Do not send a whole document to a peer to save calls.
- Do not run a second open-ended review pass on a rewritten document.
- Do not report status that was not asked for. Answer the question, do the work.
- Do not annotate a dead premise with a banner. If a document's spine is dead, cut the spine or archive the file. The 2026-08-23 banner on `00-MASTER-PLAN.md` was an attempt to reinterpret a document instead of fixing it, and it did not work.

## FACTS A FRESH SESSION NEEDS

- The local GPU purchase this corpus was written to de-risk is permanently cancelled. Policy is rent first, always, and renting is the destination, not a waypoint. Owned metal may never happen and that is acceptable.
- Nothing in this plan was ever executed. No GCP project exists. `fixtures/` never existed (confirmed against a 2026-05-14 backup and full git history across all branches, not merely absent from the repo).
- Of six advertised spend-control layers, three are confirmed non-functional: layer 4 (no `runner.py`), layer 6 (no Cloud Function source), and layer 3 for gate-0 specifically (delete command unreachable).
- Harness defects found by Codex are FIXED and merged: the `local` outside a function, the silent `e2-standard-4` / 1.0-hour cost fallbacks, the wrong default project, and `g4-standard-32`.
- A local NVIDIA RTX 4000 Ada (12GB, compute 8.9, AD104, same die family as the GCP L4) is available at host alias `lenovo` (`newlenovo`, 24 cores, 31GB RAM). Codex and this session independently concluded Track A belongs there: gate-0 gains nothing from GCP, and a box that can be physically unplugged is categorically stronger air-gap evidence than a cloud VM that always has a hypervisor, a metadata server, and Private Google Access exceptions.
- Cloudflare was never evaluated anywhere in this corpus. AI Gateway (identity-aware audit logs per prompt), R2 (US-only jurisdiction, no egress fees), Vectorize, and Containers are a fourth deployment door that `docs/advisory/claude-deployment-options.md` does not map.
- Salvage before deleting: the TPS floor of 15 tokens per second per stream under 4-way batched load, from `HARDWARE_DECISION.md`. It is substrate agnostic and applies to rented configs.

## START HERE

Queue item 0 (archive `HARDWARE_DECISION.md`), then queue item 1 Phase 1 with three peers.
