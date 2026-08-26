# Raw peer output: 20-EVIDENCE-BUNDLE-SPEC.md unit 3 (lines 322-390)

Lifecycle, and Downstream consumers.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle): audit of the six claimed automations

**1. Supabase extraction: DOES NOT EXIST** (lines 356-361)
Looked for `pantheon_runs`, `run_hypotheses`, `run_metrics`, `run_costs`, `supabase`, Cloud Function markers, GCS object triggers, deploy configs. Found only spec/runbook prose. No function source, no Supabase schema, no deploy config.

**2. Pythia semantic embedding: PARTIAL** (363-367)
Looked for `BGE`, `bge-large`, `embedding`, `pythia`, `lcs_investigate`. There is `.pythia/` index state and docs/skills reference `mcp__pythia__lcs_investigate`, so the command concept exists. **No bundle-ingestion pipeline** that watches GCS, embeds `summary.md`, tags by manifest, or inserts the run into Pythia.

**3. Obsidian vault sync: PARTIAL / PROSE ONLY** (369-372)
Found an `obsidian-note.md.template` and runbook shell lines copying to `~/Documents/pantheon-vault/runs/$RUN_ID.md`. **No automation code** watching bundles or auto-committing landed bundles.

**4. Dashboard refresh: PARTIAL, CLAIMED INTEGRATION DOES NOT EXIST** (374-377)
There is a `dashboard/` directory, but its source fetches local app/daemon routes like `/metrics`. **No Grafana/Streamlit code and no Supabase-backed evidence trend refresh.**

**5. Hypothesis tracker update: DOES NOT EXIST** (379-382)
Looked for `open-hypotheses.md`, `lessons/candidates.md`, `candidates.md`. **Those files were not found.** The section is explicitly manual, but the named artifacts do not exist.

**6. Alert on failure via PubSub: DOES NOT EXIST** (384-387)
Looked for PubSub topics, subscriptions, Slack/SMS notification config tied to bundle verdicts. Found general docs and unrelated notification code. **No subscription, function, or notification config.**

### Latencies

`within 60 sec`, `within 5 min`, `on next hour` are **aspirations, not commitments.** Nothing in lines 354-387 defines deployed triggers, schedules, queues, retries, SLIs, logs, timestamps, or consumer health checks. **There is no observable contract that would prove those latencies true.**

### Lifecycle

Lines 326 and 338 mutate `manifest.json`: create `status="running"`, later update with verdicts and `ended_at`. Contradicts the immutable-bundle goal.

**Minimal correct lifecycle:**
- Write all runtime artifacts locally under a staging path.
- Write final immutable artifacts once, including final `manifest.json`.
- Upload non-sentinel files first.
- **Upload a final completion sentinel, or the final `manifest.json`, LAST.**
- Consumers trigger only on the sentinel and verify the required object set.

Line 342 uploads the bundle before line 345 deletes the VM. **If the VM is preempted or killed between those steps, the bundle may be partial, absent, or missing final files.** `trap` does not cover hard preemption reliably, and `--max-run-duration` deletion can interrupt finalization unless upload runs outside the VM or uses resumable/atomic completion semantics.

### Ordering hazard

Lines 336-342 are **not atomic.** GCS object uploads become visible object-by-object. A bucket watcher can see `manifest.json` before `summary.md`, `cost-report.json`, `obsidian-note.md`, or `metrics/*.json` exist. **The line 356 trigger on `manifest.json` write can fire against an incomplete bundle** and insert partial rows or fail nondeterministically. The trigger object must be last, or consumers must require a completion marker plus manifest-declared object validation.

---

## GEMINI (strategic angle)

**1. What the pipeline was FOR.** The author was building a compounding knowledge engine. **The problem is real:** compute cycles usually yield ephemeral shell output rather than durable intelligence. Capturing outputs simultaneously into relational SQL (358-361), vector storage (365), and local Markdown (371) was an attempt to make "test history IS the moat" into queryable data rather than a slogan.

**2. Which of the six survive** (each with what it was for, whether the problem persists, and the replacement):

- **Supabase extraction (354-362): DROP.** (a) Aggregate tracking of cloud runs and GCP costs. (b) Track A has no cloud costs, and client bundles are standalone artifacts rather than an aggregate pool. (c) Replaced by the static `manifest.json` inside the bundle.
- **Pythia embedding (363-368): KEEP / MOVE.** (a) Made historical runs semantically queryable. (b) Still critical. (c) Move from a Cloud Function to a local post-run script.
- **Obsidian sync (369-373): KEEP / MOVE.** (a) Readable reports into a local knowledge base. (b) Still necessary. (c) A local `cp`, since the test now originates locally.
- **Dashboard refresh (374-378): DROP.** (a) Vanity charts and GCP cost-per-insight tracking. (b) Problem does not exist on local hardware. (c) Replaced by the per-run `summary.md`.
- **Hypothesis tracker (379-383): KEEP.** (a) Forced human synthesis of raw test data. (b) Still required; evidence bundles are useless without human interpretation of why the data matters. (c) Survives as a mandatory manual step.
- **Alert on failure (385-388): DROP.** (a) Paging the author when an unattended remote VM failed. (b) Local execution means the author is at the terminal. (c) Replaced by a non-zero exit code.

**3. The dependency chain is fragile.** With Track A local, relying on a GCS upload (352) to trigger a remote Cloud Function cascade is architectural malpractice. **None of this needs to be cloud-triggered.** Drive the pipeline locally from the runner's finalization step (336-343) with synchronous hooks.

**4. The manual step (379-382).** Listing a human review under a heading claiming "six automations trigger" (352) is careless writing, **but as a design choice it is correct.** Automation cannot synthesize strategic lessons. The pipeline must dump formatted evidence and halt, forcing a human to review before altering the master belief state. **The step is mandatory; only its categorization as an automation is flawed.**

**5. The knowledge-capture idea.** The missing fixtures do not invalidate the moat idea, they prove the infrastructure was built in the wrong order. *"You build a moat by digging a hole (writing tests). The author instead built an elaborate, event-driven, six-stage water filtration plant for a hole they hadn't dug yet. The core thesis, that retained evidence compounds over time, remains structurally sound. It simply starved to death waiting for data."*

---

## DEEPSEEK (adversarial logic angle)

Asked: what does writing unbuilt consumers in the present tense with stated latencies accomplish, and what rule prevents it?

> Writing it in the present tense with concrete latencies lets the author present an aspirational design as operational
> fact, making the specification sound authoritative and complete **without confronting the uncomfortable truth that
> nothing is built.** The one editorial rule that would have prevented it: **never use the present tense for behavior
> that is not implemented and verified; use "will" or "should" for intended behavior, or mark it explicitly as
> "planned / not implemented."**

**This is the root-cause rule for the entire corpus.** Every major finding in this review reduces to the same thing:
six spend layers of which three were prose, a Cloud Function pasted into a runbook, a Dockerfile copying a directory
that never existed, golden images with no build manifest, a harness entrypoint pointing at a missing module, and now
six automations of which zero are deployed. **In every case the tense did the lying.** The specificity of the
latencies ("within 60 sec") is what makes it persuasive: precision reads as evidence of implementation.

**Adopt as a corpus-wide editorial rule in every rewrite:** present tense is reserved for what has been executed and
verified. Everything else is `will`, `should`, or an explicit `NOT BUILT` marker.
