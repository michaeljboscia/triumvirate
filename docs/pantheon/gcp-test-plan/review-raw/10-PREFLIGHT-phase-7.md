# Raw peer output — 10-PREFLIGHT.md Phase 7 (lines 643-676)

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

- **Line 650:** `/Users/michaelboscia/projects/triumvirate/data/pythia.db` does not exist. `test -e` returned false. Step 7.1 is currently non-runnable.

- **Line 650:** SQLite `.backup` is the right primitive for a live snapshot. It uses SQLite's online backup API and gives a consistent snapshot while another connection may be writing. It can still be delayed or fail with locking/busy behavior under contention. The doc does not set `busy_timeout`, check exit status, or verify the output DB.

- **Line 651:** `tar czf` around one SQLite DB is packaging, not meaningful compression. If compression matters, `gzip` is simpler; if integrity matters, add a checksum.

- **Lines 645, 648:** "Server" is undefined in this section. Pantheon docs define cloud VMs (`zeus`, `athena`, `vulcan`, `orch`) and Vulcan/Homebox hardware, but nothing defines this capital-S "Server" as a current machine. The operator cannot know where to run it.

- **Lines 671-672: the dangerous one.** Because `.../gcp-test-plan/fixtures` is absent, `cd` fails loudly with `cd: no such file or directory`. **But without `set -e`, the next command still runs from the previous working directory.** Then `*` expands to whatever is in *that* directory. In this repo that means it could try uploading repo-root contents to `gs://pantheon-fixtures/`, which is worse than a clean failure.

- **Lines 664-669:** this is not "a few JSON files." To make it real someone must author: the `test-corpus-triumvirate` embedding corpus or export spec, canonical Rust/Python/SQL task suites, expected outputs or validation harnesses, a curated LoRA training corpus, eval scorers, scoring rubrics, fixture manifests, versioning, checksums, and runbook/document name reconciliation. The LoRA corpus, rubrics, and 50KLOC embedding corpus make it a substantial authoring project.

- **Line 643:** "1-2 hours, $0" is not defensible. The artifacts do not exist, the corpus names conflict, and GCS storage/operations are not literally free.

**Bottom line:** Phase 7 is a placeholder dressed as an executable preflight step. It needs fixture authoring, naming reconciliation, explicit machine/source definition, shell safety (`set -euo pipefail`, quoted paths, explicit source dirs), and artifact validation before it belongs in a runbook.

---

## GEMINI (strategic angle)

**1. Time estimate vs reality (line 643).** "1-2 hours" is absurd. It accounts only for the runtime of the `gsutil cp` commands (lines 654, 672). Authoring a curated LoRA dataset (line 668), 12 canonical tasks (lines 665-667), and scoring rubrics (line 669) is substantial engineering. **The estimate treats the hardest part of evaluation as a zero-cost assumption.**

**2. Surviving fixtures (lines 664-669).** The LoRA training corpus (line 668) is dead with the cancelled GPU purchase. The canonical agent tasks (lines 665-667), scoring rubrics (line 669), and embedding corpus (line 664) survive conceptually for local Track A and pricing sweeps, but they must actually be created.

**3. Git vs GCS (lines 659, 663).** Fixtures belong in the git repo. Storing canonical test inputs (text files of code and prompts) in `gs://pantheon-fixtures/` creates opaque, detached state and breaks test reproducibility. With Track A local, relying on a GCS bucket for test files is an anti-pattern.

**4. Pythia corpus export (lines 645-655).** Shipping a SQLite database to `gs://pantheon-pythia-corpus/` (line 654) is a useless artifact of the dead strategy. With Track A local, bouncing a database into the cloud just to download it elsewhere violates the rejection of GCP lock-in.

**5. The core conclusion.** The total absence of these fixtures proves the plan was an architectural hallucination. **The gates were never going to run because the inputs to run them did not exist.** The rewrite must conclude that future test plans start by defining, authoring, and versioning the inputs in git *before* designing any execution infrastructure.

---

## DEEPSEEK (adversarial logic angle)

Asked: what does it reveal that the fixtures were never authored and nobody noticed for four months?

> The failure order shows the plan was built as a forward-looking dependency graph rather than a backward-verified
> chain: later gates were allowed to assume fixtures that had never been checked into existence, and since no gate was
> ever run, the missing inputs stayed invisible. The single sequencing rule that would have prevented it: before
> committing any gate, require its canonical fixtures to already exist and pass a validation check, so downstream work
> can never depend on unbuilt inputs.

**Converges with Gemini's conclusion 5.** Both independently reached "the gates were never going to run because their
inputs did not exist," and both name the same fix: author and version the inputs before designing execution
infrastructure. Adopt this as a rule in the rewritten plan.
