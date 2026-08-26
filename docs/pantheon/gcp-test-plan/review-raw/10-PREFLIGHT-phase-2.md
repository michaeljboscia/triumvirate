# Raw peer output — 10-PREFLIGHT.md Phase 2 (lines 186-272)

Verbatim agent responses. Synthesized findings in `../REVIEW-PROGRESS.md`.

**Date:** 2026-08-25
**Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**High:** Phase 2 does not specify who runs these commands. If it is the Phase 1 service account, most commands fail. `roles/compute.instanceAdmin.v1` is not network admin; it does not grant `compute.networks.create`, `compute.subnetworks.create`, or `compute.firewalls.create`. So lines 192-194, 197-201, 204-209, and 213-218 cannot run as that SA. `roles/storage.objectAdmin` is object-level and does not grant `storage.buckets.create`, so all five bucket creates at lines 229-256 cannot run as that SA. `roles/artifactregistry.reader` is read-only; it lacks `artifactregistry.repositories.create`, so lines 262-265 cannot run as that SA. `gcloud auth configure-docker` at line 268 configures the local Docker credential helper for the active local user, not the runtime SA. Sources: Google IAM role docs for Compute, Storage, and Artifact Registry roles/permissions.

**High:** line 212 is fragile. `curl -s ifconfig.me` can return nothing, HTML/error text, or IPv6. Empty means line 218 expands to `--source-ranges=/32`, which should fail CLI/API validation rather than silently create a broad rule. IPv6 plus `/32` is invalid for IPv6 CIDR. CGNAT, VPN, office NAT, mobile hotspot, or changing ISP address can make the rule useless or misleading. Safer baseline: do not create public SSH ingress. Use IAP TCP forwarding and allow ingress from Google's IAP range `35.235.240.0/20` to TCP 22, or avoid SSH entirely and use OS Login/IAP-only admin access.

**High:** lines 220-222 are inconsistent with the claimed security posture. The comments say egress is "allowed to GCS + Artifact Registry + Google APIs only," but no rule is created and default VPC egress remains allow-all. If Gate 6 later claims air-gap, anything created before Gate 6 can egress freely until that later replacement happens. A restrictive egress baseline should exist from the start if the test depends on proving network isolation.

**Medium:** bucket immutability is not enforced. Lines 235-238 create the evidence bucket with uniform bucket-level access and public access prevention only. Those are access controls, not immutability controls. There is no `--retention-period`, bucket lock flow, object versioning, lifecycle policy, or explicit `--soft-delete-duration`. Current `gcloud storage buckets create` supports `--retention-period`, `--soft-delete-duration`, and `--enable-per-object-retention`; soft delete defaults to 7 days, but that is recoverability, not immutable client-facing evidence.

**Medium:** bucket names at lines 229, 235, 241, 247, and 253 are globally unique and generic. `pantheon-models`, `pantheon-evidence`, `pantheon-fixtures`, `pantheon-runners`, and `pantheon-pythia-corpus` have a material collision risk. Expected result is bucket creation failure, typically a conflict/"already exists" style error. Use project/environment suffixes.

**Low:** command syntax mostly still checks out. `gcloud compute networks create --subnet-mode=custom --mtu=1500`, subnet `--enable-private-ip-google-access`, firewall rule create flags, `gcloud storage buckets create --uniform-bucket-level-access --public-access-prevention`, `gcloud artifacts repositories create --repository-format=docker`, and `gcloud auth configure-docker REGION-docker.pkg.dev` are current valid patterns. No deprecated or renamed flags in this section.

**Low:** MTU 1500 at lines 192-194 is valid, but it is an intentional non-default. GCP VPC default is 1460; valid range is 1300-8896, and common custom values include 1500 and 8896. 1500 is reasonable for standard Ethernet and internet-adjacent traffic, but the doc should justify it because later GKE behavior can inherit VPC MTU depending on CNI/dataplane.

---

## GEMINI (strategic angle)

**1. Private Google Access (Line 201)**
Enabling `--enable-private-ip-google-access` at line 201 fundamentally invalidates any claim of an "air-gap." PGA provides a direct routing path to Google's public APIs (GCS, Logging, Artifact Registry). A network with PGA enabled cannot guarantee zero egress because the control plane remains reachable; data exfiltration to an attacker-controlled bucket via Google's APIs is trivial. The sovereign-proof network must be a separate, isolated VPC with PGA strictly disabled.

**2. Evidence Bucket Design (Lines 234-238)**
The `gs://pantheon-evidence` bucket created at lines 234-238 does nothing to solve the independence problem. The system generating the evidence has the same IAM rights to overwrite or delete it. This is computationally worthless to a security auditor. To be credible, this bucket requires WORM (Write Once, Read Many) compliance via a Retention Policy and Bucket Lock, and should reside in a separate GCP Project where the test system has strict, append-only (object creator) permissions.

**3. Purging the Buckets (Lines 225-257)**
Almost all of these are dead weight:
- **Cut `gs://pantheon-models`** (lines 228-232): Caching 250GB for demoted gates is a waste of money and state.
- **Cut `gs://pantheon-fixtures`** (lines 246-250): You confirmed these corpora do not exist.
- **Cut `gs://pantheon-runners`** (lines 252-256): If Track A is local, GCP VM startup scripts are useless.
- **Defer `gs://pantheon-pythia-corpus`** (lines 240-244): Unless the local Track A box specifically pulls this from GCS, it shouldn't exist in GCP yet.
- **Keep `gs://pantheon-evidence`** (lines 234-238): Keep this, but rebuild it with the immutability controls specified above.

**4. Premature Execution**
Yes, this entire phase is premature. Track A has moved to a local RTX 4000 Ada box. The VPC, subnet, firewalls, and SSH rules (lines 188-223) serve no immediate purpose. The only GCP resources that must exist right now are the Artifact Registry (lines 261-268) to serve container images to your local hardware, and a hardened Evidence bucket to receive the proofs. Everything else is speculative waste and should wait.

**5. Track C Readiness & Project Structure**
Bolting Track C onto this VPC and project is completely indefensible. This project was built as a disposable test rig (as evidenced by Phase 1's indiscriminate global kill-switch). Track C demands stable uptime, tenant isolation, IAP, and provable data destruction. Placing a client pilot in a project with a flat network and a global delete script is architectural malpractice. Track C requires a dedicated, isolated GCP Project per client to guarantee data boundaries and destruction logging.

**6. The Flat Internal Firewall (Lines 204-209)**
The `pantheon-allow-internal` rule at lines 204-209 allows unrestricted `tcp,udp,icmp` traffic across the entire `/20` subnet. While functional for a disposable solo rig, it is a catastrophic lateral movement risk for a shared environment. If a single Track C client pilot is compromised, the attacker has unfettered network access to exploit every other node. It is fundamentally incompatible with a production or pilot deployment.

---

## DEEPSEEK (adversarial logic angle)

Asked one focused question: given PGA enabled in the baseline subnet, is gate-6's air-gap claim survivable?

> No, the air-gap claim does not survive.
>
> Because of (A), instances with no external IP still have a valid egress path through Private Google Access. Then (B)
> explicitly keeps an allow rule for Google API ranges, so the "deny-all" firewall is really "default-deny, with an
> allowlist to Google APIs." That is a permitted network path, not an air gap.
>
> Does it constitute an exfiltration channel?
>
> - Yes, in the strict network sense: the workload can send data to Google-managed endpoints (GCS, Artifact Registry,
>   Logging, etc.).
> - Those endpoints are internet-accessible cloud services, so data written there is retrievable by anyone with
>   appropriate access.
> - tcpdump counting fewer than 5 outbound packets only proves no significant traffic was observed during that short
>   window; it does not prove egress is impossible, and it does not inspect payloads.
>
> Strongest honest claim this setup supports:
>
> > "The workload has no public IP and no general internet egress. Outbound traffic is blocked except to Google API
> > ranges via Private Google Access, and during the test window after applying deny-all-egress, fewer than 5 outbound
> > packets were observed."
>
> That is a restricted, auditable egress to Google APIs, not an air gap.

**Three-peer convergence.** Codex (from IAM and firewall mechanics), Gemini (from architecture), and DeepSeek (from
claim logic) independently concluded that PGA in the baseline makes "air gap" unsupportable in this VPC. Treat as
confirmed. DeepSeek additionally supplied the exact wording of the strongest honest claim, which is directly reusable
in the gate-6 rewrite.
