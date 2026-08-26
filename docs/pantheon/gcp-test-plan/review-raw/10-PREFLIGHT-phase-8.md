# Raw peer output — 10-PREFLIGHT.md Phase 8 (lines 677-726)

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. Line 698 does not pass the local `RUN_ID` into the container.**
The startup script is single-quoted from line 695. Inside it, `$RUN_ID` on line 698 is not expanded by the laptop shell; it is sent literally into instance metadata. On the VM the script runs under bash, but no `RUN_ID` shell variable is initialized from the separate metadata key. Result: `-e RUN_ID=` unless the guest environment defines it, which it normally will not.

**2. Line 699 expands `REGISTRY` locally, before metadata is sent.**
`'${REGISTRY}'` breaks out of the single-quoted script, expands on the laptop, then resumes quoting. If `REGISTRY` is unset locally the image becomes `/pantheon-test-harness:main`. Even with it set, `pantheon-test-harness:main` cannot be built, so `docker run` fails regardless.

**3. Line 701 expands `DEFAULT_ZONE` locally, but `$(hostname)` runs on the VM.**
Same quote-breaking pattern. If `DEFAULT_ZONE` is unset locally the VM runs `gcloud compute instances delete $(hostname) --zone= --quiet`.

**4. Line 701 self-delete is not reliable as written.**
`$(hostname)` often equals the instance name but depends on guest hostname configuration. The script already has the real name at create time (line 684) and does not pass it in. The service account (line 691) also needs `compute.instances.delete`; nothing here proves `pantheon-validator` has it.

**5. Self-delete from inside the VM is acceptable only if the API call is accepted before shutdown.**
Once the delete gets a successful API response the script being cut off does not matter. The bigger issue is earlier: `set -e` prevents line 701 from running after failures.

**6. Line 696's `set -e` makes failure leave a billable VM until the cap.**
If `docker run` fails (697-699) the script exits immediately, never runs `gsutil cp` (700), never self-deletes (701). If `docker run` succeeds but writes no `/tmp/smoke-result.json`, `gsutil cp` fails and again skips self-delete. Both leave the VM alive until `--max-run-duration=30m` (693-694).

**7. Lines 705, 708-709 do not validate job success.**
`sleep 300` waits a fixed interval, then `gsutil ls`/`cat` try to read evidence. If the VM failed before upload the operator sees storage errors, not a clear smoke-test failure. No polling of instance status, serial console output, startup-script exit status, or object existence with a controlled message.

**8. Step 8.2 does not prove the kill switch works.**
Lines 716-717 publish a synthetic alert, line 721 lists VMs. But the comment on line 719 says the expected empty list is because the smoke test already cleaned up. **The assertion is "there are no VMs after the smoke-test cleanup path," not "the hard-kill function killed a VM."** Given the function prints success unconditionally and swallows exceptions, this test can pass even if it did nothing.

**9. Step 8.2 can also mask Phase 8.1 failure.**
If line 701 never ran because of `set -e`, the smoke VM may still exist at line 721. A non-empty list is then ambiguous: startup script failure, delete-permission failure, hard-kill failure, or the function never deploying. No discriminator exists.

**10. Image family names match inside this section.** Lines 687-688 match Phase 6's family `pantheon-orchestrator`. The image name `pantheon-orchestrator-v1` is not a mismatch, since name and family are separate fields.

**11. Other billing risks.** The VM can linger to the cap if Docker fails, the result file is absent, `gsutil cp` fails, guest `gcloud` auth is unavailable, `DEFAULT_ZONE` is empty, or the SA lacks delete permission. The Pub/Sub publish can succeed while the Gen2 function is broken or absent, and line 721 cannot attribute the result to the function either way.

---

## GEMINI (strategic angle)

**1. The 8.2 assertion.** `gcloud compute instances list` (line 721) tests only that the VM created in 8.1 successfully deleted itself (line 701). **It will pass perfectly if the kill function is entirely absent, fundamentally broken, or never triggered.**

**2. Confidence justified: zero.** A green Phase 8 proves a VM can execute a startup script and delete itself. It licenses no belief whatsoever in the system's ability to detect or halt rogue spend.

**3. Genuine validation.** Create a persistent test VM that does *not* self-delete. Fire an under-threshold alert and assert the VM survives (false-positive safety). Then fire an over-threshold alert and poll until the VM is definitively destroyed by the kill switch.

**4. Survival.** It does not survive. With Track A local and the custom images and faulty function deleted, there is no GCP infrastructure to preflight and no cloud spend backstop left to validate. Delete the entire phase.

**5. The pattern: tautological testing, or validation theater.** Asserting a success condition trivially guaranteed by the test setup rather than by the system under test. **The same pattern appears in the hard-kill function itself**, which unconditionally prints its success string and swallows exceptions, guaranteeing a "pass" without doing the work.

---

## DEEPSEEK (adversarial logic angle)

Asked: name the anti-pattern and give the rule that prevents it.

> **Anti-pattern: *The Free Ride*** — a test that passes even though the mechanism it claims to verify is broken,
> because some other behavior (here, the smoke-test's self-deleting VM) already made the assertion true. It is a
> false-positive/vacuous assertion riding on a hidden test dependency.
>
> **Rule:** Couple the assertion to the mechanism under test so it is falsifiable by *that mechanism alone*. Verify the
> precondition (a VM exists), isolate or remove every other actor that could change the observed state, trigger only
> the mechanism under test, then assert the expected effect. **If you break the kill switch, the test must fail.**

**Three-peer convergence.** Codex from tracing the control flow, Gemini from asking what the assertion licenses, and
DeepSeek from the assertion's logical structure. All three independently concluded that the only test of the nuclear
spend backstop in the entire corpus proves nothing about it.
