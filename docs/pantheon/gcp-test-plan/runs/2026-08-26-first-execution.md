# Run record: first actual execution in this corpus

**Date:** 2026-08-26
**Operator:** Claude (session), commands run directly, output captured verbatim below
**Why this file exists:** every prior document in this corpus described work that had never been run. This is the
first record of commands that actually executed. It is written to be falsifiable: each claim below names the command
that produced it, so anyone can re-run it and disagree.

---

## 1. Preflight 1.1, local machine (PASS, 3 of 3)

Ran against host alias `lenovo`.

```
$ ssh lenovo 'nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv'
name, memory.total [MiB], driver_version, compute_cap
NVIDIA RTX 4000 Ada Generation Laptop GPU, 12282 MiB, 595.84, 8.9

$ ssh lenovo 'docker info >/dev/null 2>&1 && echo "docker OK" || echo "docker MISSING"'
docker OK

$ ssh lenovo 'docker run --rm --gpus all nvidia/cuda:12.6.0-base-ubuntu22.04 nvidia-smi'
NVIDIA-SMI 595.84   Driver Version: 595.84   CUDA Version: 13.2
NVIDIA RTX 4000 Ada Gene...  2598MiB / 12282MiB   0%   35C   P8   3W / 115W
No running processes found
```

The third check is the one the preflight document called most likely to fail, because it needs NVIDIA Container
Toolkit rather than just a driver. It passed. **The container runtime GPU path on `lenovo` works today.**

### Finding R-1: RETRACTED AND CORRECTED. It was a live service, not the desktop, and the card is now clear.

**My first diagnosis was wrong.** I claimed the 2598 MiB was a desktop/display session, reasoning from the fact that
the container could not see host PIDs. That inference was lazy: I never checked the host, and one command would have
settled it.

Checked from the host on 2026-08-26:

```
$ nvidia-smi -q | grep -i "Display Active"
    Display Active                                     : Disabled          <-- headless, so NOT a desktop

$ nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv
2767213, /app/.venv/bin/python, 2588 MiB

$ ps -o lstart,etime,args -p 2767213
Thu Jul 30 02:57:42 2026   27-08:45:30   /app/.venv/bin/infinity_emb v2 \
    --model-id BAAI/bge-reranker-v2-m3 --dtype auto --batch-size 32 --port 7997 --host 0.0.0.0
```

It was the **`infinity` container**, an Infinity embeddings/rerank server holding `BAAI/bge-reranker-v2-m3` resident,
deliberately deployed, up 27 days, compose file at `/home/mikeboscia/infinity/docker-compose.yml`.

**It was also not stuck.** `GET localhost:7997/models` returned `queue_absolute: 0`, `results_pending: 0`. Healthy and
idle. A reranker holds its weights resident by design; that is the service working, not failing.

Before stopping it I checked for consumers: **no live code reference anywhere in `~/projects` on either machine**
(searched `.py .ts .js .json .yaml .yml .env* .toml .sh` for `:7997`, `bge-reranker`, `RERANK`). The only hits were
Claude session transcripts from ContentFactory, meaning it was used interactively at some point, not wired into
anything running.

Stopped it:

```
$ docker stop infinity
$ nvidia-smi --query-gpu=memory.used,memory.total,memory.free --format=csv
2 MiB, 12282 MiB, 11876 MiB
```

**Corrected number: ~11.6GB usable (11876 MiB free), not 9.7GB and not a flat 12GB.** The ~400 MiB gap between total
and free is driver and context reserve, which is normal and unavoidable. Section 1.3's model sizing stands.

**Restore when the reranker is wanted again:**

```bash
ssh lenovo 'docker start infinity'      # or: cd ~/infinity && docker compose up -d
```

Its restart policy is `unless-stopped`, so an explicit `docker stop` keeps it down across reboots. It will not come
back on its own and quietly re-take the 2.6GB mid-sweep.

**The lesson, which is the actual finding.** I reported a hardware limit when the truth was a running service, and I
did it by reasoning from a container's PID namespace instead of running one command on the host. Had that gone
uncorrected, every future sizing decision would have been made against a 9.7GB budget that was never real, and the
2.6GB would have been written off permanently as a property of the machine. **A number reported without checking its
cause is not evidence, it is a guess with a unit attached.** This is the same defect class the corpus review kept
finding, produced by me, one day after documenting it.

**Standing rule for GPU work on `lenovo`: check `nvidia-smi --query-compute-apps` on the host before every run, and
treat any resident process as a service to be identified rather than a limit to be accepted.**

---

## 2. Preflight 3.1, GCP project (PASS)

```
$ gcloud projects create pantheon-validation-v1 --name="Pantheon Validation v1"
Create in progress ... done.

$ gcloud billing projects link pantheon-validation-v1 --billing-account=01F713-7EFFD2-83E164
billingEnabled: true

$ gcloud services enable --project=pantheon-validation-v1 compute artifactregistry storage \
    cloudbuild logging monitoring pubsub billingbudgets cloudfunctions run eventarc iam
Operation ... finished successfully.

$ gcloud projects describe pantheon-validation-v1 --format="value(projectId,lifecycleState)"
pantheon-validation-v1	ACTIVE
```

Project number: `598523033903`. No VM, no GPU, no disk. Current spend on this project: zero.

### Finding R-2: the gcloud install has a quota project pinned to an unrelated project

The first `gcloud billing budgets create` failed with `USER_PROJECT_DENIED` naming
`projects/e5bravo-workspace-cli`, a Workspace CLI project unrelated to Pantheon. The local gcloud configuration
routes billing/quota API calls through it.

Workaround used: `--billing-project=pantheon-validation-v1` explicitly on the command.

**This will bite every future billing, quota, and service-usage command in this corpus, and no runbook mentions it.**
Any runbook issuing those calls must pass `--billing-project` explicitly or the operator will hit an error whose text
points at an unrelated project and is actively misleading.

### Finding R-3: a GCP budget is an ALERT, not a cap (documentation defect)

Budget `effb4df0-52cb-47e2-98b4-f8a0875a7fc9` created on the billing account, scoped to project `598523033903`,
amount **$100 USD**, thresholds at 50% / 90% / 100% of current spend.

**It does not stop spending.** It sends notifications. Nothing in it terminates a resource or disables billing.

I initially named it "hard cap", which is false, and renamed it to `pantheon-validation-v1 spend ALERT (not a cap)`.
Recording the mistake rather than quietly fixing it, because it is the same defect the review found six times over:
naming a control after the outcome someone wants rather than the behavior it has. The corpus's original spend
kill-switch failed in exactly this way.

A budget becomes an actual cap only when its Pub/Sub notification drives a function that disables billing or deletes
resources. **That function does not exist.** Until it does, the only real spend controls on this project are
`--max-run-duration` on instances and the teardown trap in gate-0.

---

## 3. GPU quota (Part 3.2)

```
$ gcloud compute regions describe us-central1 --project=pantheon-validation-v1
```

| Metric (us-central1) | Limit |
|---|---|
| `NVIDIA_L4_GPUS` | **16** |
| `PREEMPTIBLE_NVIDIA_L4_GPUS` | **16** |
| `NVIDIA_A100_GPUS` | 16 |
| `PREEMPTIBLE_NVIDIA_A100_GPUS` | 64 |
| `NVIDIA_T4_GPUS` | 8 |
| `NVIDIA_V100_GPUS` | 8 |
| `NVIDIA_A100_80GB_GPUS` | **0** |
| `PREEMPTIBLE_NVIDIA_A100_80GB_GPUS` | **0** |

### Finding R-4: "new projects start at zero GPU quota" is FALSE on this account (CORRECTS THE REWRITE)

`10-PREFLIGHT.md` Part 3.2 states that new projects start at **zero** GPU quota and that a regional quota request is
required before any GPU VM can be created. That is the widely repeated general case. **It is not true here.**

This project was created minutes ago and already carries 16 L4, 16 A100 40GB, 8 T4, and 8 V100 in `us-central1`.
The account's billing history evidently confers a non-zero default. Only the A100 **80GB** SKU is at zero.

Two consequences, and the second is the important one:

1. **The multi-day quota-approval lead time I described as the reason to create this project early does not apply to
   L4.** The sizing sweep's most likely GCP target can be launched today. My stated justification was wrong, even
   though creating the project was still the right call.
2. **The guardrail I assumed existed does not exist.** I had been reasoning as though zero quota was a structural
   backstop against accidental spend. It is not. Sixteen L4s are launchable right now against a budget that only
   sends email. That is a materially worse risk posture than the document describes, and it is an argument for
   building the real kill-switch **before** the first GPU VM, not after.

### Not verified: `GPUS_ALL_REGIONS`

The global quota is no longer exposed through `gcloud compute project-info describe` (46 metrics returned, none
GPU-related), and the Cloud Quotas API path requires the `alpha` component, which is not installed. **The document's
claim that a global quota is also required remains unverified in either direction.** Do not treat R-4 as clearing it.

---

## What this changes

| Document | Change required |
|---|---|
| `10-PREFLIGHT.md` §1.3 | No change needed. ~11.6GB usable once the card is clear (R-1, corrected) |
| `10-PREFLIGHT.md` §3.2 | The zero-quota claim is false on this account; correct it and mark the global quota unverified (R-4) |
| `10-PREFLIGHT.md` §3.1 | Add the `--billing-project` requirement (R-2) |
| `30-DECISION-RULES.md` | The budget is an alert; no automated stop exists (R-3) |
| `POLICY-rent-first.md` | Utilization thresholds still NOT SET, but L4 quota exists now, so a sweep can start |

**Card state as of this run:** clear. 2 MiB used, 11876 MiB free, no compute processes.

**Highest-priority follow-on:** build a real spend stop before launching a GPU VM in this project. R-3 and R-4
compound: launchable GPUs plus notification-only budget equals no enforced ceiling.
