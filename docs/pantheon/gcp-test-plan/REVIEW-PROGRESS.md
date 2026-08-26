# Review Progress — corpus remediation state

**THIS FILE EXISTS TO SURVIVE COMPACTION.** If you are a session that just lost context, read this file and
`/Users/michaelboscia/projects/triumvirate/docs/pantheon/GOAL-corpus-remediation.md`, then resume at the position marked
RESUME HERE. Do not re-review anything already logged below. Do not start over.

**Update this file after every section review, before moving to the next section.** Findings recorded only in
conversation context are lost at compaction. If it is not written here, it did not happen.

---

## RESUME HERE

**Current queue item:** 1 of 9 (`10-PREFLIGHT.md`)
**Current section:** Phase 4 (lines 372-476) — COMPLETE, all three peers logged
**Next action:** review Phase 5 (PD snapshots, lines 477-544) with all three peers.

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
| 1 | `gcp-test-plan/10-PREFLIGHT.md` | **IN PROGRESS** — 4 of 11 sections reviewed | |
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
| Phase 2 — Network + storage | 186-272 | **DONE** (Codex, Gemini, DeepSeek) |
| Phase 3 — Docker image pre-bake | 273-371 | **DONE** (Codex, Gemini, DeepSeek) |
| Phase 4 — Model weights cached to GCS | 372-476 | **DONE** (Codex, Gemini, DeepSeek) |
| Phase 5 — PD snapshots | 477-544 | no |
| Phase 6 — Custom VM images | 545-642 | no |
| Phase 7 — Fixtures + Pythia seed | 643-676 | no |
| Phase 8 — Tooling validation | 677-726 | no |
| Preflight completion checklist | 727-755 | no |
| Cost accounting | 756-773 | no |
| What comes next | 774-end | no |

---

## FINDINGS LOG

### `10-PREFLIGHT.md` Phase 4 (lines 372-476)

Raw output: `review-raw/10-PREFLIGHT-phase-4.md`. All three peers.

#### CRITICAL

**P4-C1. Caching weights in GCS is an anti-pattern under a multi-provider rent-first policy.**
Line 444. Every pull from `gs://pantheon-models` to a non-GCP node (RunPod, Lambda) incurs GCP internet egress at
roughly $0.08-0.12/GB. The 405B model alone would cost **over $20 in egress every time a node boots**. Downloading
directly from HuggingFace to the rented node is free. Gemini's conclusion: cut the `gs://pantheon-models` bucket
entirely. This reinforces the Phase 2 cut list. *(Gemini)*

**P4-C2. `pip install` fails on Debian 12 before a single byte downloads.**
Lines 403-404. PEP 668 externally-managed-environment blocks system-Python installs. The whole phase halts at the first
command. Fix is a venv (`python3-full python3-venv`, then `/opt/hf/bin/pip install -U "huggingface_hub[cli]"`). *(Codex)*

**P4-C3. The 12h cap can permanently destroy partial work.**
Line 391, `--max-run-duration=12h` with `--instance-termination-action=DELETE`, against a claimed 6-10 hour download of
361GB. Anything not yet copied to GCS is lost with the boot disk when the VM is deleted. There is no checkpointing and
no incremental upload: the copy loop (line 442) runs only after all eight downloads finish. *(Codex)*

#### HIGH

**P4-H1. Checksums establish integrity, never provenance. TWO-PEER CONVERGENCE.**
Lines 410-439 download by repo name with no `--revision` pin, and lines 448-453 then hash whatever arrived. DeepSeek's
framing: the manifest records *what arrived*, not *what was supposed to arrive*. A repo can be updated, retagged,
force-pushed, or hijacked between runs and the manifest would faithfully record the substitute. So "we know exactly
what weights are in the box" is false as stated: you know the bytes, not that they are the authoritative release.

Minimum fix both peers named: pin `--revision <full-git-commit-sha>` and store `repo + commit SHA + file SHA256s`
together. Sign the manifest and attach the publisher's attestation for stronger provenance. Same shape as P3-H1.

**P4-H2. The `find` in the checksum block is misparsed and the manifest is partial.**
Lines 448-453. Without parentheses it evaluates as `( -type f AND -name '*.safetensors' ) OR -name '*.bin' OR -name
'*.gguf'`, so `-type f` guards only the first pattern and directories could reach `sha256sum`. It also excludes
tokenizer, config, and model-index files, so it is not a repo integrity manifest. Paths are relative to each model
root, so verification must run from the same directory. Corrected expression is in the raw file. *(Codex)*

**P4-H3. The cost-verification one-liner reports a falsely reassuring number as the bill grows.**
Line 470. `gsutil du -sh | awk '{print $1*0.020}'` coerces the human-readable size, so `350G` becomes `350` and yields
a correct-looking `$7`. But at `1.2T` awk computes `1.2 * 0.020` and prints **`$0.024/month`**. The check gets more
wrong precisely as the cost gets larger. *(Gemini and Codex, converging)*

#### MEDIUM

**P4-M1. `huggingface-cli` is renamed to `hf`.** Lines 405, 410-438. Current form is `hf download` and `hf auth login`.
*(Codex)*

**P4-M2. Interactive login is wrong for an unattended run.** Line 405 requires pasting a token on a disposable VM, and
the 6-10 hour job then depends on an SSH session staying alive. Use `HF_TOKEN` from Secret Manager plus `tmux`,
`systemd-run`, or a startup script. *(Codex)*

**P4-M3. `gsutil` is legacy.** Lines 444, 452, 456, 470 should use `gcloud storage`. *(Codex)*

**P4-M4. Disk margin is thin and the failure mode is misdiagnosed.** Line 386's 500GB against Codex's measured 361.3GB
of actual repo content leaves little room for cache metadata, partial files, and retries. `/tmp` on Debian 12 is on the
root filesystem, not tmpfs. A separate attached disk mounted at `/models` is the safer design. *(Codex)*

**P4-M5. Line 421's comment is wrong.** It says AWQ; `deepseek-ai/DeepSeek-Coder-V2-Lite-Instruct` is the unquantized
variant. *(Codex)*

**P4-M6. No license or acceptable-use logging.** Nothing records model licenses. If a client pilot serves these
weights, terms compliance (for example Meta's MAU limits) is unmanaged. *(Gemini)*

#### STRATEGIC

**P4-S1. Five of eight models cannot run on the 12GB local box and should be cut from this phase.**
Keep TinyLlama (plumbing), BGE-large (embeddings, ~1.5GB), Whisper-large-v3. Cut Qwen2.5-Coder-32B-AWQ (~18GB),
Qwen2.5-72B-AWQ (~40GB), Phi-4 14B and DeepSeek-Coder-V2-Lite-16B (unquantized, 25-30GB), and Llama-3.1-405B
(multi-GPU). Defer until a rented sizing sweep actually needs them. *(Gemini)*

**P4-S2. The 405B download is indefensible right now.** ~200GB for a Gate 5 that was demoted to a pricing-sweep row
that may never run, burning download hours against the 12h cap and paying storage forever. *(Gemini)*

**P4-S3. Model selection is four months stale.** Gemini's take: TinyLlama is workable but dated for a smoke test;
Qwen2.5-Coder and DeepSeek-Coder-V2-Lite are stale for coding; BGE-large-en-v1.5 should be BGE-M3 or Nomic-Embed-Text
v2; and Llama-3.1-405B is a poor reasoning-to-weight choice now. **Verify current model landscape independently before
acting on specific replacement names.** *(Gemini)*

**P4-S4. All eight repo IDs currently resolve and none are gated.** Codex checked HF metadata for all eight. Good news
worth recording: no license-acceptance blocker stands in the way. *(Codex)*

---

### `10-PREFLIGHT.md` Phase 3 (lines 273-371)

Raw output: `review-raw/10-PREFLIGHT-phase-3.md`. All three peers.

**Verdict: Phase 3 does not execute as written.** Step 3.1 is partly valid, step 3.2 is blocked by missing files, and
step 3.3 is entirely fictional.

#### CRITICAL

**P3-C1. `vllm/vllm-openai:v0.6.5-cpu` does not exist upstream.**
Line 288. Codex checked the Docker Hub tag API: `404`. vLLM publishes CPU images under a separate repo
(`vllm/vllm-openai-cpu`), not as a `-cpu` tag, and `vllm/vllm-openai-cpu:v0.6.5` is also `404`. The GPU image
(line 283) and `nats:2.10-alpine` (line 293) both resolve `200`. *(Codex)*

**P3-C2. Step 3.3 has five independent build failures.**
- Line 366: `-f harness/Dockerfile` — file does not exist.
- Line 340: `COPY requirements.txt .` resolves to `gcp-test-plan/requirements.txt`, but line 349 documents it at
  `harness/requirements.txt`. Neither exists.
- Line 343: `COPY harness/ ./harness/` copies a directory that is not a Python package.
- Line 344: `COPY fixtures/ ./fixtures/` — never existed.
- Line 346: `ENTRYPOINT ["python3", "-m", "harness.runner"]` — module does not exist.
*(Codex)*

**P3-C3. Step 3.2 Cloud Build references three missing files and a broken substitution.**
`cloudbuild.yaml` does not exist at the repo root or in `gcp-test-plan/`. `daemon/Dockerfile` does not exist
(line 308). And line 318 uses `${DEFAULT_REGION}` as if it were a Cloud Build substitution; it is a shell variable and
Cloud Build will not expand it. `SHORT_SHA` (lines 310, 315) is not reliably populated for `gcloud builds submit` from
a local source upload, and unavailable substitutions are replaced with empty strings, producing invalid tags. *(Codex)*

**P3-C4. vLLM v0.6.5 is a late-2024 pin being used to test 2026 hardware.**
Lines 283-284. It predates roughly two years of FlashAttention, continuous batching, and FP8/AWQ quantization work, and
lacks optimization (possibly support) for the local Ada Lovelace sm_89 card and for GCP's Blackwell G4. Testing current
silicon on that runtime yields broken builds or throughput that says nothing about the hardware. *(Gemini, with Codex
concurring that the pin is indefensible unless deliberately targeting a historical runtime)*

#### HIGH

**P3-H1. Build-time supply chain defeats the isolation claim. TWO-PEER CONVERGENCE.**
Lines 283, 288, 293 pull mutable public tags with no digest pinning, no signature verification, no SBOM, and no
provenance. DeepSeek's point is the sharp one: the runtime egress test is *structurally incapable* of detecting what is
inside those images. Dormant or time-triggered callbacks, pre-positioned exfiltration code, vulnerable dependencies,
and internal pivot tooling all survive a clean packet capture, because the measurement window never covers build and a
payload can simply wait out the test.

Minimum remedy both peers named: pin by `@sha256:` digest, verify signatures at pull (cosign/Notary), generate and scan
an SBOM, scan for known vulnerabilities, and record build provenance (command, source commit, dependency tree per
layer). The egress test then remains one control among several rather than the whole proof.

**This survives the owner's terminology correction.** Even defining air-gap as "not on the public internet," a dormant
callback baked in at build time is a real hole.

**P3-H2. Cost claim is materially incomplete.**
Header says "~$2-5 in Cloud Build." Cloud Build itself is plausibly cheap on default pools with a free tier, but the
line ignores Artifact Registry storage entirely, and vLLM GPU images run roughly 8-10 GB compressed. *(Codex)*

#### MEDIUM

**P3-M1. The Python runner duplicates the shell runner that already exists.**
Line 346's `harness.runner` would need `__init__.py`, `runner.py`, and a CLI matching the runbooks' gate/config
semantics. But `harness/runner-wrapper.sh` already owns provision, run, capture, destroy, evidence upload, and gate
config loading. Codex's recommendation: containerize the wrapper rather than invent a parallel Python runner without a
deliberate migration plan. His minimal working Dockerfile is in the raw file. *(Codex)*

**P3-M2. Environment variables assumed to persist across steps.**
Line 280 assumes `DEFAULT_REGION` and `PROJECT_ID` are exported; line 366 depends on `${REGISTRY}` still being set from
step 3.1. A fresh shell produces malformed tags rather than a clear error. *(Codex)*

**P3-M3. `options: logging: CLOUD_LOGGING_ONLY` (line 320) is defensible but unexplained.**
Required only when using a user-specified service account, which must set `logsBucket`, `CLOUD_LOGGING_ONLY`, or
`NONE`. Not inherently required otherwise. *(Codex)*

#### STRATEGIC (whole-phase)

**P3-S1. Registry topology is backwards now that Track A is local.**
Pushing images to GCP Artifact Registry only to pull them back down to the Lenovo adds latency and egress cost for no
benefit. Build and retain locally; defer Artifact Registry until GCP actually runs something. *(Gemini)*

**P3-S2. Images must be loaded from disk, not pulled, for any isolation test.**
If the stack pulls from a registry at runtime, that is a network path, and it is precisely the PGA path the Phase 2
finding describes. Load from a local OCI tarball (`docker load`) or a strictly local registry before the network is
severed, so the test has no pull path at all. *(Gemini)*

**P3-S3. Gemini's cut list.** Cut the vLLM CPU image (lines 287-290; pointless with a local Ada GPU). Cut the Cloud
Build step (lines 300-326; build locally). Cut the containerized test harness (lines 328-369; run via `uv` or a venv).
Defer all `docker push` (lines 285, 290, 295, 367). Only **vLLM GPU** and **NATS** earn their place for Track A, both
pinned by digest.

---

### `10-PREFLIGHT.md` Phase 2 (lines 186-272)

Raw output: `review-raw/10-PREFLIGHT-phase-2.md`. All three peers.

#### CRITICAL

**P2-C1. Private Google Access in the baseline subnet pre-invalidates the air-gap claim. THREE-PEER CONVERGENCE.**
Line 201 enables `--enable-private-ip-google-access`. That gives every instance without an external IP a live route to
Google's public API endpoints. Gate 6 later claims to prove air-gap while explicitly permitting the Google API ranges,
so the "deny-all" rule is really default-deny with an allowlist, and GCS is a writable destination reachable by anyone
with the right access. Codex reached this from firewall mechanics, Gemini from architecture, DeepSeek from claim logic.

DeepSeek's wording of the strongest honest claim, reusable verbatim in the gate-6 rewrite:

> "The workload has no public IP and no general internet egress. Outbound traffic is blocked except to Google API
> ranges via Private Google Access, and during the test window after applying deny-all-egress, fewer than 5 outbound
> packets were observed."

**OWNER CORRECTION (Mike, 2026-08-25), read this before acting on the above.** Air-gap here does not have to mean
literal physical isolation, especially for testing and experimentation. The working definition is **not connected to
the outside world, meaning the public internet**. Private Google Access reaches Google APIs over Google's private
backbone, not the public internet, so **PGA does not automatically fail that definition and does not need to be
ripped out.**

**So the finding is a terminology problem, not an architecture problem.** Both readings are correct in their own
context and the rewrite must hold them apart:

- **For our own testing:** PGA-enabled with default-deny egress genuinely satisfies "not on the public internet." Keep
  it. It is what makes evidence upload possible without a public IP.
- **For a client-facing sovereignty claim:** a security team WILL make the distinction, because data written to GCS
  over PGA is retrievable from the public internet by anyone holding credentials. That is an exfiltration path in the
  strict sense, and calling it "air-gapped" in a proposal is the kind of claim that ends an engagement.

**The fix is precise language, not network surgery.** Do not label the GCP test "air-gap proof." Use DeepSeek's wording
or something close to it, which is accurate under both readings:

> "The workload has no public IP and no general internet egress. Outbound traffic is blocked except to Google API
> ranges via Private Google Access, and during the test window after applying deny-all-egress, fewer than 5 outbound
> packets were observed."

The local box remains the stronger demonstrator for a client who demands literal isolation, because it can be
physically unplugged. That is an option to offer, not a requirement to impose on our own test rig.

**P2-C2. The Phase 1 service account cannot run most of Phase 2.**
Phase 2 never says who executes these commands. If it is `pantheon-validator`, nearly everything fails:
`compute.instanceAdmin.v1` grants no `compute.networks.create`, `compute.subnetworks.create`, or
`compute.firewalls.create` (lines 192-194, 197-201, 204-209, 213-218). `storage.objectAdmin` is object-level and grants
no `storage.buckets.create`, so all five bucket creates fail (lines 229-256). `artifactregistry.reader` is read-only, so
the repo create fails (lines 262-265). `gcloud auth configure-docker` (line 268) configures the local user's Docker
helper, not the runtime SA. *(Codex)*

#### HIGH

**P2-H1. `curl -s ifconfig.me` is a fragile way to build a firewall rule.**
Line 212. Can return empty, HTML error text, or IPv6. Empty expands line 218 to `--source-ranges=/32`. IPv6 with `/32`
is invalid CIDR. CGNAT, VPN, hotspot, or a changing ISP address makes the rule useless or misleading. Better baseline:
no public SSH ingress at all. Use IAP TCP forwarding allowing `35.235.240.0/20` to TCP 22, or OS Login with IAP-only
admin access. *(Codex)*

**P2-H2. The egress comment describes a rule that is never created.**
Lines 220-222 claim egress is "allowed to GCS + Artifact Registry + Google APIs only." No rule exists; VPC default
egress is allow-all, which the comment then admits. Anything built between Phase 2 and Gate 6 has unrestricted egress.
Same disease as the rest of the corpus: the document describes the intended end state in the present tense. *(Codex)*

**P2-H3. The evidence bucket has no immutability, only access control.**
Lines 235-238 set uniform bucket-level access and public access prevention. Neither is immutability. No
`--retention-period`, no bucket lock, no object versioning, no lifecycle policy, no explicit `--soft-delete-duration`
(default 7 days is recoverability, not WORM). The system generating the evidence holds the same IAM rights to overwrite
or delete it, which is worthless to an auditor. Needs a retention policy plus bucket lock, and ideally a separate
project where the test system has append-only rights. *(Codex and Gemini, converging)*

#### MEDIUM

**P2-M1. Generic globally-unique bucket names will collide.**
Lines 229, 235, 241, 247, 253. `pantheon-models`, `pantheon-evidence`, `pantheon-fixtures`, `pantheon-runners`,
`pantheon-pythia-corpus` are all plausible names someone else already took. Creation fails with a conflict. Add a
project or environment suffix. *(Codex)*

**P2-M2. The flat internal firewall is fine solo, dangerous shared.**
Lines 204-209 allow all tcp/udp/icmp across the entire `/20`. Acceptable for a disposable rig, a lateral-movement risk
the moment a Track C client pilot shares the subnet. *(Gemini)*

**P2-M3. MTU 1500 is a deliberate non-default and is unexplained.**
Lines 192-194. GCP's VPC default is 1460, valid range 1300-8896. 1500 is defensible but the doc should say why, since
GKE dataplane behavior can inherit VPC MTU. *(Codex)*

#### STRATEGIC (whole-phase)

**P2-S1. Four of five buckets are dead weight.**
Cut `pantheon-models` (250GB cache for demoted gates), `pantheon-fixtures` (corpora confirmed never to have existed),
and `pantheon-runners` (GCP VM startup scripts, useless if Track A is local). Defer `pantheon-pythia-corpus` unless the
local box pulls from it. Keep `pantheon-evidence`, rebuilt with immutability controls. *(Gemini)*

**P2-S2. The only GCP resources needed right now are Artifact Registry and a hardened evidence bucket.**
With Track A local, the VPC, subnet, firewall rules, and SSH ingress (lines 188-223) serve no immediate purpose.
Artifact Registry serves container images to the local hardware; the evidence bucket receives proofs. Everything else
waits. *(Gemini)*

**P2-S3. Track C cannot share this project.**
It was built as a disposable rig, and Phase 1's kill-switch deletes every VM in the project with no label filter. A
client pilot needs stable uptime, tenant isolation, IAP, and destruction logging. Gemini's position: a dedicated
project per client. At minimum the kill-switch must filter by label before any pilot shares the project. *(Gemini,
reinforcing P1-H2)*

---

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
