# Raw peer output — 10-PREFLIGHT.md Phase 5 (lines 477-544)

**Date:** 2026-08-25 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. Destructive disk target is wrong/unstable: lines 494, 503, 505.**
`--disk=name=pantheon-model-staging,mode=rw` omits `device-name`. Google documents `device-name` as the guest-OS symlink used to identify the disk; without setting it, this plan should not assume `/dev/disk/by-id/google-persistent-disk-1`. The intended stable path should be something like `/dev/disk/by-id/google-models` after attaching with `device-name=models`, or likely `/dev/disk/by-id/google-pantheon-model-staging` if relying on the disk resource name. `/dev/disk/by-id/google-persistent-disk-1` is attach-order-style naming and not a safe contract. **If it resolves to the boot disk or the wrong secondary disk, line 503 `mkfs.ext4` destroys that filesystem.** Use `ls -l /dev/disk/by-id/`, `readlink -f`, and `lsblk` before formatting.

**2. `sudo chown -R $USER /mnt/models` mostly works, but is sloppy: line 506.**
`$USER` is expanded by the user's shell before `sudo` runs, not by root's environment under sudo. So it becomes the SSH login user on the VM. With OS Login that username may be transformed, for example domain-qualified or `ext_...`. Safer: `sudo chown -R "$(id -u):$(id -g)" /mnt/models`.

**3. `pd-ssd` is OK for `g2-standard-4`, but not for G4 and some newer families: lines 488, 536-537.**
G2 still supports Persistent Disk. G4 is different: Google's GPU VM docs explicitly say G4 cannot use zonal or regional Persistent Disk and can only use Hyperdisk. A4/A4X/A3 Ultra also have Hyperdisk-only restrictions, as do N4/N4A/N4D in practice. This snapshot-to-`pd-ssd` create pattern breaks anywhere the target machine cannot attach Persistent Disk.

**4. Cost claim is undercounted/misframed: lines 477, 537.**
A standard disk snapshot bills on compressed incremental snapshot size, not 500GB provisioned size. At roughly 361GB of model data, a regional standard snapshot at about `$0.05/GiB-month` is about **$18.05/month before compression effects and network fees**, not `$15/mo`. Model weights are often already compressed enough that large compression savings should not be assumed.
Separately, line 537 creates a **500GB pd-ssd per gate VM**. US pricing implies `$0.17/GB-month`; 500GB is about **$85/month if left up**, prorated by seconds, about **$0.116/hour per running gate VM**. That is not represented in the line 477 ongoing-cost claim.

**5. 2026 recommendation: don't default to per-VM disks from snapshot for shared model weights.**
Prefer **Hyperdisk ML in read-only-many mode** where supported. Google documents Hyperdisk ML read-only sharing, up to 2,500 instances for <=512 GiB, positioned for model/data loading. G2 supports Hyperdisk ML per the matrix. If the gate fleet stays small and PD-capable, one shared read-only SSD PD may also be viable, but Google recommends at most 100 instances for SSD PD read-only sharing. Filestore is better for POSIX shared filesystem semantics, not raw model-load throughput. GCS FUSE with caching is simpler but usually inferior for deterministic low-latency model startup.

**6. `--snapshot-names` is still current: line 516.** `gcloud compute snapshots create` is a valid alternative, but line 516 is not using a removed flag.

**7. Ordering/runtime bug: lines 497, 500-516.**
The plan SSHes in, formats/mounts, then runs `gsutil -m cp -r` for ~361GB. The VM has `--max-run-duration=4h` with `--instance-termination-action=DELETE`. If copy, verification, logout, and snapshot coordination do not complete in time, the instance is deleted. The disk is not marked auto-delete in line 494, so the disk likely survives, but the copy can be partial. **Line 516 can then snapshot a partially populated ext4 filesystem unless the operator notices.** Add explicit `gsutil rsync`/manifest validation before snapshot, and do not use a hard VM deletion cap for the staging copy.

**8. Other first-run failures.**
Line 515 snapshots while the disk may still be mounted on a running VM. Google allows snapshots of attached disks, but the clean pattern is `sync`, unmount, detach or stop the VM, then snapshot. Line 522 deletes the staging disk immediately after snapshot creation; because `gcloud compute disks snapshot` waits for READY by default this is probably OK, but only with error checking. The snippet has no `set -euo pipefail`, no device verification, no mount verification, and no byte/count/hash validation before deleting the source disk.

**Sources:** Google disk attach docs, GPU VM docs (G4 Hyperdisk-only), Hyperdisk support matrix, disk sharing docs, Hyperdisk ML docs, disk pricing, `disks snapshot` and `snapshots create` references.

---

## GEMINI (strategic angle)

**1. Survival:** Phase 5 is entirely downstream dead weight. Line 509 explicitly copies weights from the deprecated bucket (`gsutil -m cp -r gs://pantheon-models/*`). If the GCS cache is deleted to avoid egress costs, this staging step fails immediately, rendering the subsequent snapshot (line 516) impossible. The phase has no independent value; it is a dependent artifact of a dead strategy.

**2. The optimization:** The economics are indefensible. The phase claims $15/mo to save 3-5 minutes of VM startup (line 479). The target VM at line 536 (`g2-standard-4`) costs roughly $0.70/hour, making 4 minutes of compute worth about $0.05. **To break even on $15 monthly overhead you must execute at least 300 gate runs per month.** For a pipeline that runs a handful of times, you are spending dollars to save pennies.

**3. GCP lock-in:** Lines 516 and 537 rely on proprietary GCP primitives (`gcloud compute disks snapshot`, `source-snapshot`). If RunPod is 2-4x cheaper for A100s, this PD snapshot mechanism cannot be brought along. Implementing it biases future execution toward GCP merely to retain the 20-second fast path over a 5-minute HuggingFace download elsewhere.

**4. Snapshot staleness:** No lifecycle management or upgrade path. Line 518 hardcodes `pantheon-models-v1` and line 519 stamps an arbitrary date. To update one model an operator must manually re-run lines 483-527 for a `v2`, manually update the hardcoded reference at line 537, and manually garbage-collect `v1`. Because downloads are not revision-pinned, the contents of `v1` are effectively opaque and irreproducible the moment it is created.

**5. What should replace it:** Absolutely nothing. Track A is a local RTX 4000 Ada with permanent local storage. Local model caching means downloading weights once to the host drive and bind-mounting them into containers. The ephemeral cold-start problem Phase 5 solves does not exist on a persistent local workstation. **Phase 5 should be deleted, not fixed.**

**6. Client pilot (Track C) and isolation:** A centralized PD snapshot violates strict tenant isolation. Sharing one `pantheon-models-v1` across client projects demands cross-project IAM, breaking the boundary. Respecting the boundary means re-running Step 5.1 and 5.2 inside every client project, forcing them to absorb the overhead for a cold-start delay they likely do not care about.

---

## DEEPSEEK (adversarial logic angle)

Asked: what is the general principle for writing irreversible operations into a runbook a tired human copy-pastes at 2am?

> **Principle:** Write irreversible runbooks for a tired, copy-pasting human by treating every destructive step as
> **untrusted until verified**. The runbook must make the wrong action impossible, not merely unlikely, by embedding
> verification and abort behavior into the commands themselves.
>
> **Patterns that enforce it:**
>
> 1. **Resolve before you wreck.** Use explicit stable identifiers (`/dev/disk/by-id/...`, UUIDs) and immediately
>    verify the target with `lsblk`/`readlink` plus a hard mismatch check before formatting. Example:
>    `[ "$(readlink -f /dev/disk/by-id/google-persistent-disk-1)" = "/dev/sdb" ] || exit 1`.
> 2. **Make the shell refuse to continue.** Start every script with `set -euo pipefail` and add preconditions that
>    abort: target must not be mounted, must not contain `/` or `/boot`, and required environment variables (like
>    `CONFIRM_DISK_ID`) must be set.
> 3. **Never chain destroy to untested create.** After creating a snapshot/copy, verify it independently (size
>    matches, mountable, checksum/file count OK) **before** deleting the source. Put the delete behind a separate
>    explicit command, not a straight-line `snapshot && delete`.

**Converges with Codex** on both halves: the unverified `mkfs` target and the delete-source-after-unverified-snapshot
sequence. Pattern 3 is the general form of Codex's finding 7 and 8.
