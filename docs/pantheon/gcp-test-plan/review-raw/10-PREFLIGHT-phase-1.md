# Raw peer output — 10-PREFLIGHT.md Phase 1 (lines 11-185)

Verbatim agent responses. Preserved because synthesis loses detail and context does not survive compaction.
Synthesized findings live in `../REVIEW-PROGRESS.md`.

**Date:** 2026-08-25
**Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic — TIMED OUT, see bottom)

---

## CODEX (engineering angle)

**Findings**

High severity: `10-PREFLIGHT.md:159` deploy will fail as written because `cd ~/projects/.../harness/functions/hard-kill` points at a path previously established not to exist, and this section only shows inline code at lines 113-154. Even if the directory is created, deploy needs `main.py` plus `requirements.txt`; `google-cloud-compute` is not in the standard runtime by implication and must be declared.

High severity: `10-PREFLIGHT.md:129` the function signature/code is wrong for Gen2 Pub/Sub. Current Google samples for Cloud Functions Gen2 / Cloud Run functions use CloudEvents and `@functions_framework.cloud_event`; Pub/Sub data sits under `cloud_event.data["message"]["data"]`, not a Gen1 `(event, context)` background function shape. The deploy may succeed, but invocation is likely to fail or never call the handler correctly. Source: Google's Gen2 Pub/Sub sample uses CloudEvent handling, not `(event, context)`. https://docs.cloud.google.com/functions/docs/samples/functions-cloudevent-pubsub

High severity: `10-PREFLIGHT.md:31` API enablement is incomplete for `gcloud functions deploy --gen2`. `cloudfunctions.googleapis.com` is not sufficient. Gen2 is backed by Cloud Run, Cloud Build, Artifact Registry, Eventarc, Logging, and Pub/Sub. This list already includes Artifact Registry, Cloud Build, Logging, Pub/Sub, and Cloud Functions, but is missing `run.googleapis.com` and `eventarc.googleapis.com`. Google's Pub/Sub trigger docs explicitly call out enabling Artifact Registry, Cloud Build, Cloud Run Admin API, Eventarc, Cloud Logging, and Pub/Sub.

High severity: `10-PREFLIGHT.md:153` `cost_amount` and `budget_amount` can raise `NameError` if the event has no `data`. The synthetic test has data, but malformed Pub/Sub/Eventarc delivery or a direct test without data reaches the final print and crashes.

High severity: `10-PREFLIGHT.md:142` zone iteration is unsafe and opaque. It guesses only `a/b/c/d`; not every listed region has those zones, and some regions can have different zone sets. Worse, line 150 catches every `Exception` and line 151 silently continues. That hides missing permissions, disabled APIs, auth failures, API shape errors, quota/API throttling, and delete failures. For tonight, this can print "Hard-kill completed" while having deleted nothing.

High severity: `10-PREFLIGHT.md:149` `client.delete()` returns a long-running operation and this code does not wait. The function can finish before VMs are actually gone. It also deletes only VM instances. It does not delete persistent disks, snapshots, images, reservations, IPs, buckets, Artifact Registry artifacts, or anything else that can keep costing money.

Medium severity: `10-PREFLIGHT.md:99` budget Pub/Sub routing description is conceptually off. Programmatic budget notifications are not only sent when an email threshold fires; Google says they are sent multiple times per day with current budget status, and can arrive even with no usage. The function's `costAmount / budgetAmount < 0.5` guard is therefore necessary. Field names `costAmount` and `budgetAmount` are correct per current docs. https://docs.cloud.google.com/billing/docs/how-to/budgets-programmatic-notifications

Medium severity: `10-PREFLIGHT.md:161` `gcloud functions deploy --gen2` still exists and the flags shown are broadly valid, including `--trigger-topic`, `--service-account`, memory, timeout, source, region, runtime, and entry point. But for Gen2 I would add `--region=$DEFAULT_REGION` to logs too, and deploy will need the missing APIs above. Current deploy reference confirms the command surface exists. https://docs.cloud.google.com/sdk/gcloud/reference/functions/deploy

Medium severity: `10-PREFLIGHT.md:181` `gcloud functions logs read` still exists, but for Gen2 the command has a `--gen2` flag and `--region` flag. The doc says if `--gen2` is omitted it falls back to config or lookup, so this may work, but it is brittle. Use `gcloud functions logs read pantheon-hard-kill --gen2 --region=$DEFAULT_REGION --limit=10`. https://docs.cloud.google.com/sdk/gcloud/reference/functions/logs/read

Medium severity: `10-PREFLIGHT.md:72` IAM is not actually "minimum required." `roles/compute.instanceAdmin.v1` is broad over Compute instances and includes delete, start, stop, set metadata, etc. `roles/storage.objectAdmin` is broad over all bucket objects project-wide. `roles/iam.serviceAccountUser` is dangerous if granted project-wide because it allows acting as service accounts in the project. If Phase 2 creates firewall rules, this service account likely also needs Compute network/firewall permissions; `compute.instanceAdmin.v1` does not cover all network admin operations. Either add a narrowly scoped custom role for `compute.firewalls.*` or temporarily grant `roles/compute.networkAdmin` with a clear removal point.

Medium severity: `10-PREFLIGHT.md:84` exporting a JSON key to disk is the wrong default in 2026. Google's IAM guidance treats user-managed service account keys as risky because the private key is exposed in clear text once created, and Google recommends more secure alternatives where possible. Prefer local ADC plus `--impersonate-service-account`, attached VM service accounts, or Workload Identity Federation for non-GCP runners. https://docs.cloud.google.com/iam/docs/best-practices-for-managing-service-account-keys

Medium severity: `10-PREFLIGHT.md:44` quota instructions contain forbidden timeline language per repo rules, and the line 11 title also does. Operationally: new projects often start with zero GPU quota. Current Compute quota docs explicitly say GPU requests require both the model quota in each target region and global `GPUs (all regions)`. This section omits `GPUs (all regions)` / `GPUS_ALL_REGIONS`, so VM creation can fail even after regional GPU quota is approved.

Medium severity: `10-PREFLIGHT.md:52` quota names are plausible display names, not stable CLI metric identifiers. Console search may find them, but CLI automation should use Cloud Quotas / Service Usage metric names. The doc should distinguish display names from metric names. `CPUs (G2)`, `CPUs (A2)`, and `CPUs (G4)` may not be the quotas that gate accelerator-optimized machine creation in current Compute docs; for G2/G4, GPU quota is the key requirement, and CPU family quotas may be absent or not separately requestable depending on current quota model. Needs validation against the exact VM types later phases create.

Low severity: `10-PREFLIGHT.md:21` `gcloud projects create $PROJECT_ID --name=...` is current and valid. Caveat: it can fail if the ID is already taken, the account lacks project creation permission, an org requires `--organization` or `--folder`, or project creation quota/org policy blocks it. https://docs.cloud.google.com/sdk/gcloud/reference/projects/create

Low severity: `10-PREFLIGHT.md:23` `gcloud beta billing projects link` still exists, but beta is no longer needed. GA command is `gcloud billing projects link $PROJECT_ID --billing-account=$BILLING_ACCOUNT`. https://docs.cloud.google.com/sdk/gcloud/reference/billing/projects/link

Low severity: `10-PREFLIGHT.md:177` `gcloud pubsub topics publish ... --message=...` is fine for publishing the synthetic payload. It will not prove the real budget payload shape reaches the Gen2 handler unless the handler is fixed to CloudEvent format.

**Ordering Bug**

Lines 95-96 say the function code is in Step 1.6 but deployed in Phase 4. Lines 156-170 deploy it immediately. Immediate deploy is the correct dependency order if the 50% Pub/Sub kill path is supposed to be active during real spend. A budget routed to Pub/Sub before the function exists can still publish messages, but nothing useful happens until a subscription/trigger exists. Do not run spend-bearing phases until topic, budget notification permissions, function deployment, and synthetic invocation are all proven.

---

## GEMINI (strategic angle)

**1. Necessity and Size**

Phase 1 is bloated and premature. Since Track A (plumbing and air-gap proof) has been moved to the local RTX 4000 Ada, testing cloud infrastructure at this stage is unnecessary.
- **Cut:** Step 1.3 (lines 44-59) in its entirety. You do not need massive GPU quotas to test local plumbing.
- **Defer:** Steps 1.4, 1.5, and 1.6 (lines 62-182) should be moved to Track B or C, when cloud execution is actually required.
- **Keep:** Steps 1.1 and 1.2 (lines 13-42) only if Track A requires cloud-based Artifact Registry or Storage. Otherwise, defer them too.

**2. Quota Requests**

Requesting 8x A100 80GB GPUs on GCP (line 52) violates the "rent first always" policy. GCP charges ~$5.03/hr per A100, whereas RunPod charges $1.19-$1.60/hr.
- This request is unjustified. The quota ask for A100s on GCP should be zero.
- If GCP quota is required strictly for GCP-specific validation, it should be limited to 1-2x L4 GPUs (line 53). Heavy GPU compute should be routed to RunPod.

**3. Budget and Credit Assumption**

The $100/month budget (line 104) and 50% ($50) nuclear kill threshold (line 108) are mathematically incoherent.
- If a single gate costs $20-$40, running one test consumes up to 80% of the kill threshold. A single delayed cleanup or a second concurrent run will trip the 50% wire and obliterate the environment.
- If the unverified Gemini Ultra credit does not exist or does not cover Compute Engine, you will burn real cash immediately. The budget parameters must be redesigned to accommodate the actual cost of the gates, independent of hypothetical credits.

**4. Hard-kill Function**

The blast radius is concealed by omission.
- Lines 146-149 unconditionally iterate through and delete *every* instance in 11 regions (lines 123-127). It checks no labels, tags, or names.
- If a Track C client pilot is running in `pantheon-validation-v1` when this function fires, it will be instantly vaporized. This is unacceptable for a shared project. Track C requires a dedicated project or the kill function must filter by labels.

**5. Missing Elements**

Given the rebuilt plan's objectives, Phase 1 lacks critical infrastructure:
- **Evidence Bundles:** There is no infrastructure (e.g., locked buckets, audit log sinks) established to capture and cryptographically sign the air-gap proofs for client security teams.
- **Track C IAP:** Track C requires stable uptime and Identity-Aware Proxy (IAP). The IAP API (`iap.googleapis.com`) is not enabled in Step 1.2 (lines 31-41), and there are no firewall or IAM configurations to support it.
- **Proof-of-destruction:** The kill script (lines 146-149) simply deletes VMs. It fails to generate or export verifiable logs of disk wipes or resource destruction to satisfy client compliance obligations.

**6. Quota Request Justification Text**

The justification text (line 59) actively hurts your chances of approval.
- It requests 8x A100s (costing ~$40/hr to run) and 192 CPUs, while simultaneously claiming a "Max concurrent spend capped at $50/run" and a "Budget $100/month total."
- A Google reviewer will immediately flag this as contradictory. The requested hardware vastly exceeds the stated budget, which reads as either a compromised account or a user who does not understand GCP pricing. It will be denied.

---

## DEEPSEEK (adversarial logic angle)

**STATUS: FAILED — daemon timeout at 180s, twice.**

Task IDs `khfs3uynt` (earlier, different section) and `k89n0o224` (this section) both exceeded the
`TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS=180` ceiling. DeepSeek is consistently slower than that on substantial prompts
even at `medium` reasoning effort.

The questions posed, preserved so this can be retried without rewriting the prompt:

1. Classify each step: does it produce a verifiable falsifiable result, or merely appear to succeed? Where would
   someone running this at 2am conclude "done" while nothing happened?
2. The kill-function test greps logs for "Hard-kill completed." That string prints unconditionally at the end
   regardless of whether any VM was found or deleted, and the delete loop swallows every exception. What does a
   passing test actually prove? Design the test that would prove the kill switch works.
3. Failure modes of `if cost_amount / budget_amount < 0.5: return` beyond the obvious: repeated delivery, zero
   denominator, and notifications arriving many times per day with current status rather than at threshold crossings.
4. Circular verification: where does the check share credentials, tooling, or assumptions with the thing checked?
5. Ignoring security: how does over-broad permission damage the TRUSTWORTHINESS of every later result?
6. The single change to Phase 1 that would most reduce the chance of a silent no-op.

Note that Codex independently reached point 2's conclusion from the code: the bare `except: continue` plus the
unconditional final `print` means the function "can print 'Hard-kill completed' while having deleted nothing."
