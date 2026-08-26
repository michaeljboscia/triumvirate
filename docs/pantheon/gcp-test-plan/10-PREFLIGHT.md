# Pantheon Preflight

**Status:** rewritten 2026-08-26 against a full three-peer review of the 2026-04-18 original
**Review record:** `REVIEW-PROGRESS.md` (113 findings across 11 sections) and `review-raw/10-PREFLIGHT-*.md`
**Purpose:** Get to a first real executed run at the lowest possible cost and risk, and make every claim in this document falsifiable.

> **This is a rewrite, not an edit.** The original was reviewed section by section by Codex, Gemini, and DeepSeek. It
> was found to be unexecutable: the hard-kill Cloud Function had no source, its test could not fail, four checklist
> items referenced artifacts that had never existed, and two entire phases optimized a problem that no longer exists.
> The original is in git at `5c7e89c:docs/pantheon/gcp-test-plan/10-PREFLIGHT.md`.

---

## 0. The two rules this document now obeys

Both came out of the review, and both are corpus-wide.

**Rule 1: inputs before infrastructure.** The original was built as a forward dependency graph that nothing ever
walked backwards. Later phases assumed fixtures that had never been authored, and because nothing was ever executed,
the missing inputs stayed invisible for four months. **Before any gate is committed, its canonical fixtures must
already exist and pass a validation check.**

**Rule 2: verification, not attestation.** A human-ticked checkbox converts a question about the world ("does this
work?") into a question about a person's confidence. Before irreversible spending, that is the one thing you cannot
trust. **Every gate in this document is a command whose failure is observable, and every check is first run against a
broken world to prove it can fail.** A test that cannot fail is not a gate, it is an ornament.

A corollary that applies to every destructive step below: **treat it as untrusted until verified, and never chain a
destroy to an unverified create.**

---

## Part 1: Local preflight (do this first, costs nothing)

Track A (plumbing, then isolation proof) runs on local hardware. This is not a compromise, it is the better test: a
box you can physically unplug is stronger isolation evidence than a cloud VM that always has a hypervisor, a metadata
server, and Private Google Access routes.

**Target machine:** host alias `lenovo` (`newlenovo`), NVIDIA RTX 4000 Ada Generation Laptop GPU, 12GB VRAM, compute
capability 8.9, 24 cores, 31GB RAM. Ada Lovelace means native bf16, FP8, working FlashAttention 2, and current vLLM
support. It is the same AD104 die family as the GCP L4, so it is a faithful rehearsal rig for that class.

### 1.1 Verify the machine

```bash
ssh lenovo 'nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv'
ssh lenovo 'docker info >/dev/null 2>&1 && echo "docker OK" || echo "docker MISSING"'
ssh lenovo 'docker run --rm --gpus all nvidia/cuda:12.6.0-base-ubuntu22.04 nvidia-smi >/dev/null && echo "gpu-in-container OK"'
```

All three must succeed before proceeding. The third is the one that actually matters and the one most likely to fail,
because it needs NVIDIA Container Toolkit rather than just a driver.

### 1.2 What runs locally

Everything Track A needs: Docker Compose, NATS, the Triumvirate daemon, a small model for smoke tests, and the
isolation proof. **Nothing here needs GCP.** No Artifact Registry, no custom images, no PD snapshots.

Images are built locally and kept locally. The isolation test loads images from disk (`docker load` from an OCI
tarball) rather than pulling at runtime, so the test has no pull path to explain away.

### 1.3 Models that fit 12GB

Only three of the original eight are usable here, and only these should be downloaded now:

| Model | Purpose | Fits 12GB |
|---|---|---|
| TinyLlama 1.1B Chat (or a current small equivalent) | plumbing smoke test | yes, trivially |
| BGE-large-en-v1.5 (or BGE-M3 / Nomic-Embed v2, verify current) | embeddings | yes, ~1.5GB |
| Whisper large v3 | audio | yes |

Cut for now: Qwen2.5-Coder-32B-AWQ (~18GB), Qwen2.5-72B-AWQ (~40GB), Phi-4 14B and DeepSeek-Coder-V2-Lite-16B
(unquantized, 25-30GB), Llama-3.1-405B-AWQ-INT4 (~209GB, multi-GPU). Download those only when a rented sizing sweep
actually needs them, on the rented node itself.

**Model selection above is April 2026 vintage and needs a current check before use.** The landscape has moved; verify
what the right small, coding, embedding, and reasoning models are today rather than trusting these names.

### 1.4 Pin everything by digest and revision

Non-negotiable, and it is the fix for three separate findings.

```bash
# Container images: pin by digest, never by mutable tag
docker pull vllm/vllm-openai@sha256:<digest>
docker pull nats@sha256:<digest>

# Model weights: pin by commit, and record repo + commit + file hashes together
hf download <repo-id> --revision <full-git-commit-sha> --local-dir <dir>
```

A checksum of what you downloaded proves **integrity** (the bytes have not changed since). It does not prove
**provenance** (that they are the authoritative release). Only the revision pin does that. Record `repo + commit SHA +
per-file SHA256` in one manifest, or the sovereignty claim is false as stated.

Note `vllm/vllm-openai:v0.6.5-cpu` **does not exist** (Docker Hub returns 404; CPU images live in a separate repo).
And `v0.6.5` is a late-2024 pin that predates roughly two years of FlashAttention, batching, and FP8 work, and lacks
proper support for Ada and Blackwell. **Pick a current vLLM release and pin its digest.**

---

## Part 2: Fixtures (author these before any gate is written)

This is Rule 1 in practice. The original uploaded a `fixtures/` directory that never existed, and every gate silently
depended on it.

**Fixtures live in git, not in a bucket.** They are text. Putting canonical test inputs in object storage creates
opaque detached state and breaks reproducibility.

Author under `docs/pantheon/gcp-test-plan/fixtures/`:

| Fixture | Contents | Status |
|---|---|---|
| `agent-tasks/` | canonical code-generation tasks with expected properties, per language | NOT WRITTEN |
| `eval-scorers/` | scoring rubric per task type, plus the scorer implementation | NOT WRITTEN |
| `embed-corpus/` | the corpus used for embedding-throughput tests, or a pinned pointer to it | NOT WRITTEN |

Cut: the LoRA training corpus. It existed to serve fine-tuning gates for hardware that is not being bought.

**Naming was inconsistent across three documents in the original** (`test-tasks-*` in the master plan,
`agent-tasks-*` in preflight, other names in the runbooks). The names above are canonical; reconcile the other
documents to them during their rewrites.

**This is real authoring work, not an upload.** The original budgeted "1-2 hours, $0" for what is actually the design
of an evaluation methodology. Treat the estimate as unknown until the rubrics exist.

### Validation gate for fixtures

```bash
# Must exit non-zero if any fixture is missing or malformed.
test -d fixtures/agent-tasks && test -d fixtures/eval-scorers || { echo "FIXTURES MISSING"; exit 1; }
```

No gate may be committed until this passes.

---

## Part 3: GCP preflight (deferred until GCP actually runs something)

**Do not do this yet.** Track A is local. This part exists for when the sizing sweep needs rented hardware, and it is
deliberately much smaller than the original.

Only two GCP resources have any near-term justification, and only if the local box proves insufficient:

1. An **evidence bucket** with real immutability, if evidence must live off the local machine.
2. **Artifact Registry**, only if something in GCP needs to pull images.

Everything else waits.

### 3.1 Project, billing, APIs

```bash
set -euo pipefail
export PROJECT_ID="pantheon-validation-v1"
export BILLING_ACCOUNT="01F713-7EFFD2-83E164"   # verify with: gcloud billing accounts list
export DEFAULT_REGION="us-central1"
export DEFAULT_ZONE="us-central1-a"

gcloud projects create "$PROJECT_ID" --name="Pantheon Validation v1"
gcloud config set project "$PROJECT_ID"
gcloud billing projects link "$PROJECT_ID" --billing-account="$BILLING_ACCOUNT"   # 'beta' no longer needed
gcloud config set compute/region "$DEFAULT_REGION"
gcloud config set compute/zone "$DEFAULT_ZONE"
```

A dedicated project is what makes a project-wide kill switch safe. Do not share it with anything else, and never with
a client pilot.

APIs, including the ones Gen2 Cloud Functions actually require (the original omitted `run` and `eventarc`, so the
function deploy would have failed):

```bash
gcloud services enable \
  compute.googleapis.com artifactregistry.googleapis.com storage.googleapis.com \
  cloudbuild.googleapis.com logging.googleapis.com monitoring.googleapis.com \
  pubsub.googleapis.com billingbudgets.googleapis.com cloudfunctions.googleapis.com \
  run.googleapis.com eventarc.googleapis.com iam.googleapis.com
```

### 3.2 Quota

New projects start at **zero** GPU quota and need **both** a per-model regional quota **and** the global
`GPUs (all regions)` quota. The original omitted the global one, so VM creation would still have failed after regional
approval.

**Ask small.** The original requested 8x A100 while stating a $100/month budget, which reads to a reviewer as either a
compromised account or someone who does not understand the pricing, and invites denial. It also contradicts policy:
RunPod runs A100 80GB at $1.19-1.60/hr against GCP's roughly $5.03 on-demand, so heavy GPU work does not belong here
at all.

Request only what GCP-specific validation needs: **1 to 2 L4**, plus the matching `GPUs (all regions)`. Route
everything larger to a cheaper provider.

Quota display names in the console are not stable CLI metric identifiers. Verify the metric name against Cloud Quotas
before scripting against it.

### 3.3 Service account and access

The original granted a set labelled "minimum required" that was both too broad and incomplete. `compute.instanceAdmin.v1`
grants no network permissions (so it cannot create the VPC or firewall rules), `storage.objectAdmin` is object-level
(so it cannot create buckets), `artifactregistry.reader` is read-only (so it cannot create the repo), and
`iam.serviceAccountUser` at project scope lets the account act as any service account in the project.

**Decide and document who runs what.** Setup commands run as you. The runtime service account gets only what the
runtime needs, and network permissions are granted temporarily with a stated removal point.

**Do not export a JSON service account key.** Google's current guidance treats user-managed keys as a risk because the
private key is exposed in clear text on creation. Use local ADC with `--impersonate-service-account`, an attached VM
service account, or Workload Identity Federation.

### 3.4 Network, only if needed

If a GCP VM is required, note that **Private Google Access changes what you can claim.** See Part 5.

Do not create public SSH ingress from a shell-captured IP. `curl -s ifconfig.me` can return empty (making the rule
`--source-ranges=/32`), HTML, or an IPv6 address that is invalid with `/32`, and it breaks on CGNAT, VPN, or any
address change. Use **IAP TCP forwarding**, allowing `35.235.240.0/20` to TCP 22, or OS Login with IAP-only access.

If you write an egress restriction, write the rule. The original had a comment claiming egress was limited to Google
APIs while leaving VPC default allow-all in place, and then admitted it in the next line.

### 3.5 Buckets, if any

Bucket names are globally unique and `pantheon-evidence` is a name someone else may hold. Suffix with the project.

**An evidence bucket needs immutability, which uniform access and public-access-prevention do not provide.** Those are
access controls. Add a retention policy and bucket lock. And note the independence problem: evidence assembled by the
system under test, stored in a bucket that same system can overwrite, is worth little to an auditor. Ideally the
bucket lives in a separate project where the test system has append-only rights.

Cut from the original: `pantheon-models` (cross-provider egress makes a GCS weight cache a bill paid twice; pull from
HuggingFace to the rented node instead), `pantheon-fixtures` (fixtures belong in git), `pantheon-runners` (no purpose
once Track A is local), and defer `pantheon-pythia-corpus`.

---

## Part 4: The kill switch, done properly

The original's spend controls were advertised as six layers. Three were not real: the Pub/Sub kill function had no
deployable source, the `timeout` wrapper referenced a `runner.py` that does not exist, and gate-0's self-delete was
placed after `exit` so it never ran. **Claiming six layers while operating four is the dangerous kind of wrong.**

Build this only when GCP is actually being used, and build it correctly.

### 4.1 The function must be Gen2-shaped

The original was Gen1 background-function code (`def hard_kill(event, context)`) deployed with `--gen2`. Gen2 uses
CloudEvents, and the payload sits at `cloud_event.data["message"]["data"]`. The deploy would have succeeded while the
handler never fired correctly.

It also needs real files on disk at a real path, including `requirements.txt` declaring `google-cloud-compute`. Put
them in `harness/functions/hard-kill/`.

> **NOT BUILT.** As of 2026-08-26 that directory does not exist. This section is a specification for work not yet
> done, and it is labelled so deliberately: the original document's defining failure was describing an intended end
> state in the present tense until a reader believed it was real. **No GCP spending happens until this function exists,
> deploys, and passes the test in 4.3.** Until then the honest count of working spend-control layers is the ones you
> can name and demonstrate, which right now is `--max-run-duration` with `--instance-termination-action=DELETE`, and
> nothing else.

### 4.2 It must fail loudly

The original swallowed every exception with a bare `continue`, hiding missing permissions, disabled APIs, auth
failures, and throttling. Then it printed `"Hard-kill completed"` unconditionally at the end, so it could report
success having deleted nothing. It also referenced `cost_amount` outside the block that bound it, raising `NameError`
on any delivery without `data`.

Requirements for the rewrite:

- Enumerate real zones from the API rather than guessing suffixes `a` through `d`.
- Catch only the specific exceptions you intend to tolerate, and log every other one as a failure.
- Wait for the delete operations, since `client.delete()` returns a long-running operation.
- Report counts: instances found, deletes issued, deletes confirmed, errors. **Never print a success string that is
  not conditional on those numbers.**
- Handle disks, snapshots, and images too, or state plainly that it only kills instances so nobody believes otherwise.

Budget notifications arrive multiple times per day with current status, not only at threshold crossings, so the
`cost/budget` guard is load-bearing rather than belt-and-braces. Guard the zero denominator.

### 4.3 It must be tested against a world where it can fail

This is the finding all three peers converged on. The original test published a synthetic message and asserted that no
VMs remained, while its own comment explained the list was empty because the smoke test had already deleted its VM
itself. It passed whether the function worked, was broken, or was absent.

The correct test:

1. Create a disposable VM that **does not** self-delete, and confirm it exists.
2. Publish an **under**-threshold message. Assert the VM still exists. (Proves no false positives.)
3. Publish an **over**-threshold message. Poll until the VM is actually gone, or fail on timeout.
4. Assert the function's reported delete count matches what you created.

**If you break the kill switch, this test must fail.** Verify that by breaking it deliberately once and watching it go
red. An untested kill switch is not a control, it is a plan for a control.

---

## Part 5: What you can honestly claim about isolation

Terminology matters here because it ends up in front of clients.

**For our own testing,** air-gap means not connected to the public internet. Private Google Access rides Google's
private backbone, so a PGA-enabled subnet with default-deny egress satisfies that, and PGA is what makes evidence
upload possible without a public IP. Keep it.

**For a client-facing claim, that is not an air gap,** and a security team will say so: data written to GCS over PGA
is retrievable from the public internet by anyone with credentials. The strongest claim a firewalled GCP VM supports:

> "The workload has no public IP and no general internet egress. Outbound traffic is blocked except to Google API
> ranges via Private Google Access, and during the test window after applying deny-all-egress, fewer than N outbound
> packets were observed."

Call the GCP test **cloud restricted-egress validation**. Reserve "air-gap proof" for the local box, which can be
physically disconnected.

Two things no runtime egress test can establish, both of which need separate evidence:

- **Build-time behavior.** Anything fetched or phoning home during image build is outside the measurement window
  entirely. This is why Part 1.4 pins digests and why images are loaded from disk rather than pulled.
- **Dormant callbacks.** A payload that beacons on a schedule longer than the test simply waits it out. Coverage,
  window adequacy, instrumentation trust, and non-perturbation all have to be argued, not assumed.

---

## Part 6: The gate before spending (commands, not checkboxes)

The original ended in twenty human-ticked boxes, four of which referenced artifacts that could not exist, and one of
which claimed a "nuclear backstop verified" by a test that could not fail. Note its own wording: *"triggered deletion
behavior"* rather than *"deleted a VM."* The first asserts a function ran. The second asserts the world changed.

Replace it with a script that **fails closed** and writes its output as evidence. Each check is a command with an
expected result, and each check has itself been run against a broken world to confirm it goes red.

| Check | Command shape | Passes when |
|---|---|---|
| Local GPU usable in a container | `ssh lenovo docker run --gpus all ... nvidia-smi` | exit 0 |
| Fixtures exist and validate | fixture validation script | exit 0 |
| Images pinned by digest | grep the compose/manifest for `@sha256:` on every image | no unpinned tags |
| Model provenance recorded | manifest contains repo + commit SHA + file hashes | all three present |
| No unexpected billable state | `gcloud compute instances list`, disks, snapshots, images | matches expected inventory |
| Kill switch actually kills | the four-step test in Part 4.3 | VM confirmed deleted, and confirmed surviving under threshold |
| Budget alerts configured | `gcloud billing budgets list` | thresholds present |

**No green script, no spending.** Human sign-off is for judgement, not for facts a command can settle.

---

## Part 7: Cost accounting (honest version)

The original claimed $6-13 one-time and $15-20/month ongoing, then concluded "effective cost to Mike: $0." Every part
of that needs correcting.

**Local Track A: $0.** No cloud resources. This is the entire argument for doing it first.

**Deferred GCP costs, when incurred,** must count what the original omitted entirely: Artifact Registry storage for
multi-GB images, Cloud Storage across whatever buckets survive, custom image storage, **per-gate persistent disks**
(the original provisioned a 500GB pd-ssd per gate VM at roughly $0.116/hour and never put it in any table), snapshot
storage at corrected pricing, failed and retried builds, logging and monitoring, Pub/Sub and Function invocations, and
network egress. The Phase 5 snapshot alone was closer to $18/month than the $15 claimed for everything.

**On the Gemini Ultra credit.** Google AI Ultra can include monthly Google Cloud credits through the Google Developer
Program, so the original claim was not fiction. But it is a **bounded benefit, not a blank cheque**, and it is
falsified by credit exhaustion, an ineligible billing account or project, expired or unclaimed credits, or SKUs and
regions outside the promo terms.

**Verify the live credit balance on the billing account before spending. Do not infer it from the subscription.**
Designing a budget around an entitlement rather than a balance, while the kill switch is also unproven, is how a
capped experiment becomes an uncapped liability.

---

## Part 8: What comes next

Run something small, locally, and let reality falsify what review cannot.

1. Author the fixtures (Part 2). Nothing downstream is real until they exist.
2. Verify the local box (Part 1.1).
3. Run the plumbing test locally: Docker Compose, NATS, Triumvirate daemon, small model. Cost: $0.
4. Run the isolation test locally, with the network physically disconnected and images pre-loaded from disk.
5. Only then decide whether anything needs GCP, and only then work Part 3.

The root cause of everything in the review record was that nothing was ever executed, so every wrong claim survived
four months unchallenged. **More review is the same disease. Run it.**

---

## Appendix: What was deleted from the original, and why

Kept here so nobody restores it without reading the reasoning.

**Phase 5, PD snapshots for fast model mount. The MECHANISM is replaced, the CAPABILITY is kept.**

The original had real defects: `mkfs.ext4` ran against an unverified device path that could have formatted the boot
disk; the snapshot could capture a partial copy after which both VM and source disk were deleted with nothing
verified; the cost was understated (roughly $18/month for the snapshot, plus an unbudgeted 500GB pd-ssd per gate VM at
about $0.116/hour); `pd-ssd` cannot attach to G4 at all; and there was no lifecycle or update story.

**But do not delete this and replace it with nothing.** The peer review priced the snapshot against `gsutil cp` from a
same-region bucket, which is the document's own 3-5 minute figure. **That is the wrong comparison once the GCS model
cache is also cut. The real alternative is a cold HuggingFace download, which for a large model is closer to 30
minutes.** For a client pilot or a live demo, a 30-minute cold start is disqualifying, not a rounding error.

The answer is conditional on where the node runs:

| Node location | Weight source | Cold start |
|---|---|---|
| Local Lenovo | download once to local disk | zero after the first time, so no problem to solve |
| GCP | **Hyperdisk ML in read-only-many mode**, the current 2026 primitive for shared read-only model mounts (up to 2,500 instances at <=512 GiB) | seconds |
| RunPod or other third party | HuggingFace direct, since GCS would charge egress | still the slow case, and it needs a decided answer |

Hyperdisk ML replaces the PD-snapshot pattern. It is not a footnote to its deletion.

**Phase 4, the `gs://pantheon-models` cache, follows the same conditional logic.** Cutting it is right for
cross-provider work, where egress means paying to pull weights HuggingFace serves free. It is wrong as a blanket rule
for GCP-resident nodes, where a same-region cache costs no egress and turns a 30-minute cold start into minutes.

**Phase 6, custom VM images.** Deleted. The image family `common-cu126` no longer exists (current is
`common-cu129-ubuntu-2204-nvidia-580`); `${REGISTRY}` was unset on the baker VMs so every pull was an invalid
reference; Spot plus `termination-action=DELETE` would destroy the very disk being prepared; baking on an L4 bakes Ada
drivers into an image claimed to be A100 and RTX PRO 6000 compatible; and the whole artifact encodes "Docker is
installed and three images are pulled," which is a couple of minutes of runtime work. Pull containers at runtime.

**`gs://pantheon-models`.** Cut. Cross-provider egress at roughly $0.08-0.12/GB means a rented non-GCP node pays to
pull weights that HuggingFace serves free. The 405B model alone would have cost over $20 in egress per node boot.

**The 405B download.** Cut. Roughly 209GB for a gate demoted to a pricing-sweep row that may never run.

**The LoRA training corpus.** Cut. It served fine-tuning gates for hardware that is not being purchased.

**"Your first Pantheon run is immortalized."** Cut, along with the rest of the hype register. Engineering documents
describe deterministic behavior. Watch for the same voice elsewhere in the corpus ("knowledge moat", "nuclear
backstop") and cut it there too.
