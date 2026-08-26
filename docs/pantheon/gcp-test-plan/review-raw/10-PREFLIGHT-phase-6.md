# Raw peer output: 10-PREFLIGHT.md Phase 6 (lines 545-642)

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. Line 606-607: `common-cu126` is stale and likely invalid.**
Google's current DLVM docs list active base GPU families as `common-cu129-ubuntu-2404-nvidia-580` and `common-cu129-ubuntu-2204-nvidia-580`. CUDA 12.8 is already deprecated for support as of April 13, 2026. No current `common-cu126` family appears in Google's current DLVM image table. The naming scheme also changed to include CUDA, Ubuntu version, and NVIDIA driver version. Source: Google DLVM images docs, last updated August 11, 2026.

**2. Line 613: `--metadata=install-nvidia-driver=True` conflicts with the comment at line 600.**
Current docs still describe the flag as the mechanism for Google to install the latest stable driver on first boot, with a reboot. But current DLVM families explicitly include pre-installed NVIDIA driver 580 in the family name. The flag can trigger first-boot driver installation and reboot, defeating the "drivers baked" assumption. For `common-cu129-ubuntu-*-nvidia-580`, omit it unless testing proves otherwise.

**3. Lines 605, 611-612: Spot is wrong for a baker VM with `DELETE`.**
If preempted mid-bake, Compute Engine may stop or delete it. With `--instance-termination-action=DELETE` the VM is deleted, the partially prepared boot disk is gone, and there is no artifact to image. Spot only suits idempotent restartable bakes. This is an interactive SSH bake followed by manual image creation: use a standard VM, or at minimum `STOP` rather than `DELETE`.

**4. Lines 568-569: `newgrp docker` is brittle in this SSH/script context.**
`usermod -aG` changes future login group membership. `newgrp docker` starts a new shell; in a non-interactive scripted SSH block it commonly blocks, replaces the shell context, or prevents subsequent commands running as expected. The `docker pull` at line 578 is not reliable. Safer: `sudo docker pull ...`, or reconnect after group membership applies.

**5. Lines 573-579 and 619-621: `${REGISTRY}` is unset on the baker VMs.**
`REGISTRY` was set in a different shell on a different machine. `docker pull ${REGISTRY}/${img}` expands to `docker pull /pantheon-triumvirate:main`. Docker image references cannot start with `/`; this fails as an invalid reference before any pull. Same for lines 620-621.

**6. Lines 586-592 and 629-635: imaging the stopped boot disk is structurally correct, but cleanup is missing.**
`gcloud compute images create --source-disk --source-disk-zone` is the right shape after stopping. `--force` is not needed (that is for imaging a running instance). Boot disk name usually matches the instance name. **Missing: guest cleanup before imaging, especially SSH host keys and machine identity.** Linux needs no sysprep, but cloning a booted machine without identity cleanup duplicates host keys and persisted machine state.

**7. Line 625: the CUDA tag exists; GPU Docker runtime depends on DLVM contents.**
`nvidia/cuda:12.6.0-base-ubuntu22.04` exists with active amd64/arm64 images. `--gpus all` requires NVIDIA Container Toolkit. DLVM images historically include Docker and NVIDIA Docker, but verify `docker info | grep -i nvidia` before relying on it.

**8. Line 545: "2-3 hours, ~$1-2" is not defensible.**
`g2-standard-4` with one L4 is roughly the `$0.70/hr` on-demand class in US regions, Spot is variable. Add `e2-standard-2`, a 100GB boot disk, image storage, and failed/retried bakes. The lower bound fits only if everything succeeds cleanly. The plan also cannot complete because the referenced Pantheon images are known-unbuildable or missing upstream.

**Net: Phase 6 is not executable as written.** Hard blockers: stale DLVM family, unset `REGISTRY`, nonexistent Pantheon images, unsafe Spot `DELETE` semantics for artifact creation.

---

## GEMINI (strategic angle)

**1. Same lock-in as Phase 5.** GCE custom images bias infrastructure toward GCP. `gcloud compute images create` artifacts (lines 588, 632) cannot deploy to RunPod or AWS. And the "baking" here literally consists of installing Docker and running `docker pull` (lines 566-579, 619-621). **The portable equivalent is to not use custom VMs at all: do everything in containers.** RunPod and other cheap providers natively consume Docker containers.

**2. What is the actual saving?** Installing Docker and pulling three images takes roughly 2-3 minutes on a datacenter connection. Spending "2-3 hours" (line 545) plus ongoing image storage to save 3 minutes of startup on an hour-long gate run is aggressive premature optimization. Not worth doing before a single gate has ever executed.

**3. Image lifecycle: there is no update story.** Lines 588 and 632 hardcode `pantheon-orchestrator-v1` and `pantheon-gpu-v1`. Lines 574-577 and 620-621 hardcode `vllm-gpu:v0.6.5` and `triumvirate:main`. **If a developer pushes a new commit to `main`, the baked image is immediately stale**, and maintaining the fast-boot advantage means re-running the 3-hour baker on every code change.

**4. Does the GPU baker test the right hardware? The line 597 claim is suspicious if not false.**
Line 604 bakes on an L4 (`type=nvidia-l4`). Line 613's `install-nvidia-driver=True` triggers an on-boot script installing drivers specific to the attached hardware. Snapshotting the disk *after* that script runs for an Ada L4 bakes the L4 driver into the image. Booting it on an Ampere A100 or Blackwell RTX Pro 6000 will likely produce driver mismatches, CUDA failures, or degraded performance.

**5. Should any of it happen now?** No. With Track A on a local RTX 4000 Ada, building GCE VM images is actively useless. Defer entirely, or better, discard permanently in favor of runtime container pulls.

**6. Isolation and client pilots.** Handing a client a GCP-specific VM image works against the isolation proof. If a pilot must run on their own infrastructure or AWS, a GCE image proves nothing. The proof relies on portable orchestration (Docker Compose or Kubernetes).

**Verdict: DELETE Phase 6.** Premature, GCP lock-in, severe maintenance burden on code updates, likely breaks across GPU architectures, zero purpose for a local Track A. Pull containers at runtime.

---

## DEEPSEEK (adversarial logic angle)

Asked: what is lost by capturing a machine state instead of describing how to build it?

> A snapshot records **state**, not **cause**. You lose:
>
> - Which package versions apt resolved at bake time, and whether later updates silently changed them.
> - Which container image digests were pulled (tags are mutable pointers).
> - The base image, apt sources, kernel, and any non-determinism in the install steps.
> - Any audit trail that lets you recreate, patch, or verify the environment.
>
> The reproducibility claim fails "more than it first appears" because the test environment is an opaque, mutable blob.
> Two runs from the same image name can use different underlying content if the image is ever rebuilt or drifted;
> results become unrepeatable in a way that's invisible until something breaks. Reproducibility means *given the same
> inputs, get the same output*. With only an artifact, there are no recorded inputs, so the claim is unfalsifiable.
>
> **Minimum fix:** a version-controlled, declarative build manifest (Dockerfile/Packer config) with **pinned package
> versions and image digests**, and rebuild the image from that manifest rather than snapshotting an ad hoc state.

**Third instance of the same root pattern.** P3-H1 (unpinned container tags), P4-H1 (unpinned model revisions), and now
P6: in every case the artifact records what happened rather than what was specified, and the resulting claim is
unfalsifiable rather than merely unverified. This is a corpus-wide theme, not three separate bugs.
