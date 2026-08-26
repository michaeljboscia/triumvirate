# Raw peer output: gate-0-plumbing.md unit 3 (lines 246-314)

Step 6 evidence emission, Step 7 capture logs and self-destruct, Step 8 verify and vault, Cost accounting, What comes after.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. Step 7 self-destruct is dead code. Confirmed in context.**
Line 285 runs `exit`; line 286 therefore never runs. **On the original GCP target, the VM would remain alive after logs/cost/note upload. Nothing in this block saves it.** Only an external TTL, manual cleanup, a preconfigured shutdown policy, or spot preemption would prevent leakage, and none is shown here.

**2. Step 6 does not follow sentinel-last semantics, and is not even the last writer.**
Lines 250-257 generate `manifest.json` and `summary.md`, then line 260 bulk-uploads the directory. **No content-addressing, no `COMPLETE` sentinel, no finalize step.** Worse: **Step 7 writes more objects into the same bundle after Step 6's apparent upload point**, adding logs, a cost report, and the Obsidian note at 269-282. Validation is `gsutil ls -r` (263), which lists objects but validates neither schema, checksums, required files, nor completeness.

**3. Step 8 "verify" is existence listing only.**
Line 292 lists the prefix. **It can pass against a partial bundle if any objects exist.** Lines 293-295 assume a local `~/Documents/pantheon-vault` path and therefore a specific operator machine. Under the updated rule, `obsidian-note.md` should be an internal sidecar, and vault mutation should not live in the evidence verification path.

**4. Cost accounting is now wrong.** Line 304 assumes `e2-standard-4` Spot cost; line 306 totals `~$0.10`. For the local target this should say no GCP VM charge, cloud storage charges only if still uploading, and `cost_status: pending_billing_export`. **Do not state a dollar total.**

**5. "What comes after" is unsupported.** Line 312 asserts Gate 1 is next and calls it "first real GPU burn." **The next step should be conditional on verified complete evidence and explicit promotion criteria, not an automatic jump.** Line 314's "cheapest debug available" is unsupported rhetoric.

**6. Local Step 7 teardown must replace VM deletion.** Explicitly: stop and remove Gate 0 containers; remove the compose-created network; remove named volumes **only if test-scoped**; free and check bound ports; delete temp compose/env/config files under `/tmp`; finalize or clear evidence staging after upload; revoke or delete temporary credentials and token files. **Avoid deleting shared Docker resources or long-lived local credentials.**

---

## GEMINI (strategic angle)

**1. What Step 7 should be**, ordered, since the machine no longer burns down:
1. Capture telemetry: `docker compose ... logs > ...`
2. **Eradicate state:** `docker compose ... down -v --remove-orphans` (this is the missing H-0.4 teardown).
3. Archive evidence: move `/tmp/evidence/$RUN_ID` to permanent local storage.
4. Scrub temps: `rm -rf /tmp/evidence/$RUN_ID /tmp/docker-compose.gate-0.yml`.

The `exit` (285) and `gcloud compute instances delete` (286) must both be deleted.

**2. The Obsidian vault note (289-296).** *What it was for:* integrating run metadata into a personal knowledge base for historical searching. **It does not belong in this runbook.** Extract it into a global post-run hook or a generalized reporting script. **Tying PKM git commits to a specific test stage creates brittle coupling.**

**3. Cost accounting (300-309).** *What it was for:* tracking GCP compute and storage spend to prevent budget drain. *Locally:* the financial problem is gone, **but the constraint problem remains.** Replace with **Resource Accounting**: local SSD consumption from artifacts, and execution time (operator waiting, process locking).

**4. "What comes after" (310-314) does not force advancement.** Lines 312-314 state a passive fact ("Gate 0 PASS goes to Gate 1"). **Under Rule A, passive documentation licenses stopping.** To force advancement, passing must output the literal execution command for the next stage, or trigger it in the harness.

**5. THE FIRST-RUN FAILURE PREDICTION.** Lines 250, 254, 273, 278 call Python scripts at **`/opt/pantheon-harness/`**. These are absolute paths native to a custom GCP image template. **Locally that directory almost certainly does not exist, and the scripts fail immediately with file-not-found.** Further, if the test hangs before Step 7, nothing enforces a timeout, leaving ports squatted and state contaminated.

---

## DEEPSEEK (adversarial logic angle)

Asked: a step named "evidence bundle emission" is not the last writer to the bundle. What does that reveal?

> The pattern suggests the runbook was **written incrementally**: Step 6 was authored as if it were the terminal step,
> then Step 7 was appended later without revisiting Step 6's name or checking the final data flow. **The name
> "emission" was chosen from intention, not from the actual end-to-end behavior.**
>
> The discipline is: **name steps by the invariant they guarantee, not by the role they play in a draft narrative.**
> If a step does not make the bundle consumable and complete, call it "upload bundle contents (phase 1)". Ensure the
> final writer is the step whose name implies finalization. **Equivalently, author from the consumer's viewpoint:
> only the step that leaves the bundle in its terminal state may be called "emission."**

**Same root cause as the corpus-wide tense finding, in a different grammatical category.** The tense rule said present
tense is reserved for what has been executed. This says a step's *name* is a claim about an invariant, and naming it
from intention rather than behavior is the same failure: **a label asserting something the code does not do.**

**Practical check for the rewrite:** for every step name that implies terminality (emit, finalize, complete, publish,
verify), confirm nothing after it touches the thing that step claims to finish.
