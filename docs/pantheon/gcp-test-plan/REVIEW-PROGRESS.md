# Review Progress — corpus remediation state

**THIS FILE EXISTS TO SURVIVE COMPACTION.** If you are a session that just lost context, read this file and
`/Users/michaelboscia/projects/triumvirate/docs/pantheon/GOAL-corpus-remediation.md`, then resume at the position marked
RESUME HERE. Do not re-review anything already logged below. Do not start over.

**Update this file after every section review, before moving to the next section.** Findings recorded only in
conversation context are lost at compaction. If it is not written here, it did not happen.

---

## RESUME HERE

**Current queue item:** 1 of 9 (`10-PREFLIGHT.md`)
**Current section:** Phase 1 (lines 11-185) — COMPLETE, all three peers logged
**Next action:** review Phase 2 (Network + storage, lines 186-272) with all three peers.

### OPERATING CONSTRAINT discovered 2026-08-25, obey it

**DeepSeek times out at the bridge's 180s ceiling on long prompts.** `TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS` is read
client-side in `daemon-http` (lib.rs:437) by the MCP bridge process, so it cannot be raised without restarting the
session. Two long DeepSeek prompts failed; a short single-question prompt at `reasoning_effort: low` returned
immediately.

**Working pattern per section:**
1. Codex and Gemini get the full section by absolute path plus line range. They read from disk. Fire both in one message.
2. DeepSeek gets ONE focused question, `reasoning_effort: low` or `medium`, minimal pasted context. It cannot read files.
3. Append every peer's verbatim output to `review-raw/<doc>-<section>.md` immediately.
4. Update this file.
5. Commit.

Do not send DeepSeek a six-part question with a large pasted body. It will time out and the work is lost.

---

## QUEUE STATUS

| # | Document | Status | Commit |
|---|---|---|---|
| 0 | `HARDWARE_DECISION.md` + provenance | **DONE** — archived, TPS floor extracted into buy-vs-rent section 6 | `401fdde` |
| 1 | `gcp-test-plan/10-PREFLIGHT.md` | **IN PROGRESS** — Phase 1 of 11 sections reviewed | |
| 2 | `gcp-test-plan/20-EVIDENCE-BUNDLE-SPEC.md` | pending | |
| 3 | `gcp-test-plan/30-DECISION-RULES.md` | pending | |
| 4 | `runbooks/gate-0-plumbing.md` | pending | |
| 5 | `runbooks/gate-6-airgap-sanity.md` | pending | |
| 6 | `local-inference-buy-vs-rent.md` | partially touched (TPS floor added) | `401fdde` |
| 7 | `model-selection.md`, `graduated-gcp-validation-plan.md` | pending | |
| 8 | `runbooks/gate-1` through `gate-5`, `gate-7` | pending | |
| 9 | `twin-review-synthesis.md` | pending | |

**Section list for queue item 1 (`10-PREFLIGHT.md`), 11 sections:**

| Section | Lines | Reviewed |
|---|---|---|
| Phase 1 — Project + billing + quota | 11-185 | **DONE** (Codex, Gemini, DeepSeek) |
| Phase 2 — Network + storage | 186-272 | no |
| Phase 3 — Docker image pre-bake | 273-371 | no |
| Phase 4 — Model weights cached to GCS | 372-476 | no |
| Phase 5 — PD snapshots | 477-544 | no |
| Phase 6 — Custom VM images | 545-642 | no |
| Phase 7 — Fixtures + Pythia seed | 643-676 | no |
| Phase 8 — Tooling validation | 677-726 | no |
| Preflight completion checklist | 727-755 | no |
| Cost accounting | 756-773 | no |
| What comes next | 774-end | no |

---

## FINDINGS LOG

### `10-PREFLIGHT.md` Phase 1 (lines 11-185)

Reviewed by Codex (engineering) and Gemini (strategic). DeepSeek (adversarial logic) pending as background task.

#### CRITICAL

**P1-C1. The hard-kill function is Gen1 code deployed as Gen2. It will not work.**
Line 129. The handler signature `hard_kill(event, context)` is the Gen1 background-function shape. Gen2 Pub/Sub
functions use CloudEvents via `@functions_framework.cloud_event`, and the payload sits at
`cloud_event.data["message"]["data"]`, not `event['data']`. The deploy may succeed while invocation never calls the
handler correctly. This is the nuclear backstop for spend control. *(Codex)*

**P1-C2. Gen2 deploy will fail on missing APIs.**
Line 31. `cloudfunctions.googleapis.com` alone is insufficient. Gen2 is backed by Cloud Run and Eventarc, so
`run.googleapis.com` and `eventarc.googleapis.com` must also be enabled. Google's Pub/Sub trigger docs name Artifact
Registry, Cloud Build, Cloud Run Admin, Eventarc, Logging, and Pub/Sub explicitly. *(Codex)*

**P1-C3. The deploy path does not exist.**
Line 159. `cd .../harness/functions/hard-kill` points at a directory that has never existed. The code at lines 113-154
is the only copy and it lives inside this markdown file. Needs `main.py` plus a `requirements.txt` declaring
`google-cloud-compute`. *(Codex, previously confirmed independently)*

**P1-C4. The kill loop hides every failure.**
Lines 142-151. It guesses zone suffixes a/b/c/d per region (not every region has those, and some have others), and
catches every `Exception` with a bare `continue`. That silently swallows missing permissions, disabled APIs, auth
failures, throttling, and delete failures. It can print "Hard-kill completed" having deleted nothing. *(Codex)*

**P1-C5. Deletes are fire-and-forget and cover only instances.**
Line 149. `client.delete()` returns a long-running operation and the code never waits, so the function can return
before any VM is actually gone. It also touches nothing else that bills: disks, snapshots, images, reservations,
static IPs, buckets, Artifact Registry. *(Codex)*

**P1-C6. `NameError` on the final print.**
Line 153. `cost_amount` and `budget_amount` are bound inside `if 'data' in event:` but referenced in the closing
`print` outside it. Any delivery without `data` crashes there. *(Codex)*

#### HIGH

**P1-H1. The budget math is incoherent with the gate costs.**
Lines 104-108. A $100/month budget with the nuclear kill at 50% ($50), against a plan whose most expensive single gate
was estimated at $20-40. One gate consumes up to 80% of the kill threshold. A delayed cleanup or a second concurrent
run trips the wire and destroys the environment mid-work. *(Gemini)*

**P1-H2. The kill function's blast radius is never stated, and it is project-wide.**
Lines 123-127, 146-149. It deletes *every* instance across eleven regions with no label, tag, or name filter. The
rebuilt plan's Track C hosts client pilots whose VMs must not self-destruct. If a pilot runs in this project when the
budget signal fires, it is vaporized. Either Track C needs its own project or the kill must filter by label. *(Gemini)*

**P1-H3. `GPUS_ALL_REGIONS` is missing from the quota ask.**
Lines 50-57. New projects start at zero GPU quota and require BOTH the per-model regional quota AND the global
`GPUs (all regions)` quota. Omitting the global one means VM creation still fails after regional approval is granted.
*(Codex)*

**P1-H4. The quota request will likely be denied on its own text.**
Line 59. It asks for 8x A100 (roughly $40/hr to run) and 192 CPUs while stating a $50/run cap and a $100/month total
budget. A Google reviewer reads that as either a compromised account or someone who does not understand the pricing.
The hardware requested vastly exceeds the stated budget. *(Gemini)*

**P1-H5. The A100 quota ask contradicts the rent-first policy.**
Line 52. GCP charges roughly $5.03/hr on-demand for A100 80GB; RunPod charges $1.19-1.60/hr. Requesting 8x A100 quota
on GCP is the wrong provider for that workload. The GCP ask should be small (1-2x L4 for GCP-specific validation) with
heavy GPU work routed elsewhere. *(Gemini)*

#### MEDIUM

**P1-M1. IAM is labelled "minimum required" and is not.**
Line 72. `roles/compute.instanceAdmin.v1` is broad across instances; `roles/storage.objectAdmin` is project-wide across
all bucket objects; `roles/iam.serviceAccountUser` granted project-wide allows acting as any service account in the
project. Separately, Phase 2 creates firewall rules and `compute.instanceAdmin.v1` does not cover that, so the set is
simultaneously too broad and incomplete. *(Codex)*

**P1-M2. Exporting a JSON service account key is the wrong default in 2026.**
Line 84. Google's IAM guidance treats user-managed keys as risky since the private key is exposed in clear text on
creation. Prefer local ADC with `--impersonate-service-account`, attached VM service accounts, or Workload Identity
Federation. *(Codex)*

**P1-M3. Quota display names are not metric names.**
Line 52. `NVIDIA A100 80GB GPUs`, `CPUs (G2)`, `CPUs (G4)` are plausible console display strings, not stable CLI metric
identifiers. For G2/G4 the GPU quota is the gating one and CPU-family quotas may not be separately requestable under
the current quota model. Needs validation against the exact machine types later phases create. *(Codex)*

**P1-M4. `gcloud functions logs read` is brittle for Gen2.**
Line 181. Works, but should be `--gen2 --region=$DEFAULT_REGION` explicitly rather than relying on config fallback.
*(Codex)*

**P1-M5. Budget notification semantics are misdescribed.**
Line 99. Programmatic budget notifications are sent multiple times per day with current status, not only when an email
threshold fires, and arrive even with no usage. This makes the function's `< 0.5` guard load-bearing rather than
belt-and-braces. Field names `costAmount` and `budgetAmount` are correct. *(Codex)*

**P1-M6. Ordering contradiction.**
Lines 95-96 say the function code is in step 1.6 but "deployed in Phase 4"; lines 156-170 deploy it immediately.
Immediate deploy is correct if the kill path is meant to be live during real spend. A budget can route to a topic before
a function exists, and nothing happens. Do not run spend-bearing phases until topic, budget notification permissions,
function deployment, and a synthetic invocation are all proven. *(Codex)*

#### LOW / CORRECT AS WRITTEN

- `gcloud projects create` syntax is current and valid (line 21). Can fail on taken ID, missing permission, or an org
  requiring `--organization`/`--folder`. *(Codex)*
- `gcloud beta billing projects link` works but `beta` is no longer needed; GA is
  `gcloud billing projects link` (line 23). *(Codex)*
- `gcloud functions deploy --gen2` flag surface shown is broadly valid (line 161). *(Codex)*
- `gcloud pubsub topics publish --message=` is fine for the synthetic payload (line 177), but proves nothing until the
  handler is converted to CloudEvent format. *(Codex)*

#### ADVERSARIAL LOGIC (DeepSeek)

**P1-A1. The kill-switch test proves logging, not killing. Independent confirmation of P1-C4.**
The test greps for `"Hard-kill completed"`, which prints unconditionally at the end. Because every Compute call sits
inside `try/except: continue`, that line runs even if every `list` and `delete` failed or no instances existed.

The minimum test that would actually prove the switch works:
1. Inject known state: create one test VM per zone, or mock `InstancesClient` so `list()` returns fixed fake instances
   across the 11 regions x 4 zones.
2. Send a synthetic event where `costAmount / budgetAmount >= 0.5`.
3. Run `hard_kill`.
4. Assert `client.delete` was called once per instance returned by `list`, with correct project, zone, and instance. On
   real GCP, poll until those instances are gone or the delete operations complete.
5. Send a second event under the threshold and assert zero `delete` calls and no state change.

Two peers reached "it can report success having done nothing" from different directions: Codex by reading the exception
handling, DeepSeek by reasoning about what the assertion tests. Treat that as confirmed, not suspected.

#### STRATEGIC (whole-phase)

**P1-S1. Phase 1 is premature at this size now that Track A moved local.**
Gemini's cut list: delete step 1.3 entirely (no large GPU quota is needed to test local plumbing); defer steps 1.4, 1.5,
1.6 to whichever track first needs cloud execution; keep 1.1 and 1.2 only if Track A needs cloud Artifact Registry or
Storage. Worth weighing against the counter-argument that quota approval is not instant, so filing a SMALL ask early
still has value. *(Gemini)*

**P1-S2. Missing entirely for the rebuilt plan's needs.**
No locked buckets or audit log sinks for client-facing evidence capture. No `iap.googleapis.com` enabled and no
firewall/IAM to support Track C's Identity-Aware Proxy requirement (Track C needs stable uptime and IAP, not SSH). No
proof-of-destruction logging: the kill path deletes VMs but exports nothing verifiable about disk wipes. *(Gemini)*

**P1-S3. The `$100/mo absorbed by Gemini Ultra credit` assumption is four months old and unverified.**
If the credit does not exist or does not apply to Compute Engine, this burns real cash immediately. Verify before
planning around it. *(Gemini, and flagged independently in the rebuilt master plan)*
