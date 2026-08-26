# Raw peer output: 10-PREFLIGHT.md Phase 4 (lines 372-476)

Verbatim agent responses. Synthesized findings in `../REVIEW-PROGRESS.md`.

**Date:** 2026-08-25 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. High: line 404 fails on Debian 12 before any model download.**
`pip install huggingface_hub 'huggingface_hub[cli]'` runs against the system Python after only `python3-pip` is installed on line 403. Debian 12 enforces PEP 668, so this fails with `externally-managed-environment`. Use a venv or pipx:
```bash
sudo apt-get update && sudo apt-get install -y python3-full python3-venv
python3 -m venv /opt/hf
/opt/hf/bin/pip install -U "huggingface_hub[cli]"
/opt/hf/bin/hf auth login
```

**2. High: lines 386 and 407 probably fit today, but with weak margin and wrong failure model.**
`/tmp` on Debian 12 is normally on the root filesystem, not tmpfs, so this uses the 500 GB boot disk from line 386. If `/tmp` were tmpfs, the failure would be immediate tmpfs exhaustion on an `e2-standard-4`.
Current HF file metadata totals for the eight repos: about **361.3 GB decimal** before cache/temp overhead (`2.2 + 19.3 + 41.6 + 31.4 + 4.0 + 29.3 + 24.7 + 208.7`).
500 GB holds the final trees but is fragile: `hf download --local-dir` still involves cache metadata, partial files, retries, and filesystem overhead. A larger attached disk mounted at `/models` is the safer design.

**3. High: line 391 can delete partial work.**
`--max-run-duration=12h` plus `--instance-termination-action=DELETE` means Compute Engine permanently deletes the VM at the limit. Anything not already copied to GCS is lost, and partial downloads under `/tmp/models` go with the boot disk.

**4. Medium: line 405 is not suitable for unattended execution.**
`huggingface-cli login` requires interactive token entry on a disposable VM. Brittle, and success depends on a human SSH session staying alive. Use `HF_TOKEN` from Secret Manager or metadata, then `hf auth login --token "$HF_TOKEN"`. Better: run the whole download/copy/checksum script under `tmux`, `systemd-run`, or startup-script logging.

**5. Medium: lines 405 and 410-438 use deprecated CLI spelling.**
Hugging Face renamed `huggingface-cli` to `hf`; current docs show `hf download` and `hf auth login`. Correct form:
```bash
hf download TinyLlama/TinyLlama-1.1B-Chat-v1.0 --local-dir /tmp/models/tinyllama-1.1b
```

**6. Medium: lines 448-453 checksum only some file types and the manifest is awkward to verify.**
The `find` expression parses as `( -type f AND -name '*.safetensors' ) OR -name '*.bin' OR -name '*.gguf'` because `-type f` applies only to the first `-name`. Directories named `*.bin` or `*.gguf` could be sent to `sha256sum`. Correct expression:
```bash
find . -type f \( -name '*.safetensors' -o -name '*.bin' -o -name '*.gguf' \) -print0 |
  sort -z |
  xargs -0 sha256sum > "/tmp/${model_name}.sha256"
```
The manifest contains relative paths like `./model-00001-of-00044.safetensors` because the subshell `cd`s into each model dir. Verifiable later only if verification also runs from the model root. It also excludes tokenizer/config/model index files, so it is not a full repo integrity manifest.

**7. Medium: lines 444, 452, 456, 470 should move off `gsutil`; line 470 cost math is broken for human units.**
Google now recommends `gcloud storage`. `gsutil du -sh` still exists, but `awk '{print $1*0.020}'` is invalid when `$1` is human-readable, e.g. `1.2 TiB`; awk treats only `1.2` numerically and ignores the unit. Use `gcloud storage du --summarize gs://pantheon-models/` and convert bytes explicitly.

**8. Low: model IDs currently resolve; no gated repos found through public HF metadata.**
- `TinyLlama/TinyLlama-1.1B-Chat-v1.0`: exists, not gated.
- `Qwen/Qwen2.5-Coder-32B-Instruct-AWQ`: exists, not gated.
- `Qwen/Qwen2.5-72B-Instruct-AWQ`: exists, not gated.
- `deepseek-ai/DeepSeek-Coder-V2-Lite-Instruct`: exists, not gated. **Line 421 comment is wrong: this is not the AWQ variant.**
- `BAAI/bge-large-en-v1.5`: exists, not gated.
- `microsoft/phi-4`: exists, not gated.
- `openai/whisper-large-v3`: exists, not gated.
- `hugging-quants/Meta-Llama-3.1-405B-Instruct-AWQ-INT4`: exists, not gated.

**9. Correct: line 388/389 credentials are likely sufficient for GCS writes.**
With `--service-account=$SA_EMAIL` and `--scopes=cloud-platform`, the VM uses the SA's IAM permissions. Given `roles/storage.objectAdmin`, it should have `storage.objects.create`, assuming the role is bound at a scope covering `gs://pantheon-models`.

**Sources:** HF CLI docs, HF CLI rename announcement, Google gsutil transition docs, Google VM runtime limit docs, `instances create` reference.

---

## GEMINI (strategic angle)

**1. Which of the eight models still earns its place?**
Only three can operate on a 12GB VRAM local box:
- **Keep:** TinyLlama 1.1B (line 410) for plumbing smoke tests.
- **Keep:** BGE-large-en-v1.5 (line 426) for embeddings; ~1.5GB VRAM.
- **Keep:** Whisper-large-v3 (line 434).

Cut the other five from this phase. Qwen2.5-Coder-32B-AWQ (line 414) needs ~18GB. Qwen2.5-72B-AWQ (line 418) needs ~40GB. Phi-4 14B (line 430) and DeepSeek-Coder-V2-Lite-16B (line 422) are downloaded unquantized and demand 25-30GB. Llama-3.1-405B (line 438) needs a multi-GPU node. Defer until a rented sizing sweep requires them.

**2. Is downloading and storing the 405B model defensible?**
No. Lines 437-439 download a ~200GB+ model for a Gate 5 test demoted to a pricing sweep that may never run. It burns compute duration (line 391) and incurs continuous storage cost for zero immediate utility.

**3. Model selection is April 2026 vintage and outdated for late August 2026.**
- **Plumbing:** TinyLlama works, but a newer 1B-3B model is a better modern baseline.
- **Coding:** Qwen 2.5 Coder and DeepSeek-Coder-V2-Lite are stale; target the DeepSeek-Coder-V3/V4 era or whatever current 8B-10B coding model fits in 12GB.
- **Embeddings:** BGE-large-en-v1.5 is ancient. BGE-M3 or Nomic-Embed-Text v2.
- **Large reasoning:** Llama-3.1-405B is a massive dependency; DeepSeek-R1 or Llama 4 class offer better reasoning-to-weight.

**4. Storage cost claim.**
Arithmetic works on paper: ~350GB at $0.020/GB/mo (line 469) is roughly $7.00/month. But line 470 is fundamentally broken: `gsutil du -sh | awk '{print $1*0.020}'` coerces `350G` to `350` and yields `7`. **If the bucket hits `1.2T`, awk computes `1.2 * 0.020` and outputs a falsely reassuring `$0.024/month`.**

**5. Is caching weights in GCS the right architecture at all?**
No, it is a catastrophic anti-pattern. Pulling weights from GCS (line 444) to a RunPod or other non-GCP node triggers GCP internet egress at roughly $0.08-0.12/GB. Downloading the 405B model to RunPod costs over $20 in egress **every single time you boot a node**. Downloading directly from HuggingFace to RunPod is free. Cut the `gs://pantheon-models` bucket entirely.

**6. What is missing?**
- **Sovereignty and provenance:** lines 410-439 download without a `--revision` hash. HF repos can be updated silently. The SHA256 checksums at lines 448-453 verify the transfer to GCS, not upstream provenance. The sovereignty claim breaks if you cannot cryptographically guarantee which weights were downloaded.
- **Licensing:** no mechanism logs model licenses or acceptable-use policies. If a client pilot serves these weights, terms compliance (e.g. Meta's MAU limits for Llama) is unmanaged.

---

## DEEPSEEK (adversarial logic angle)

Asked: what does a post-hoc SHA256 manifest of downloaded weights actually establish?

> It establishes **integrity**, not **provenance**.
>
> - **Integrity:** the SHA256 manifest proves the `.safetensors` files are byte-for-byte identical to whatever the
>   pipeline downloaded. If you recompute the hashes later and they match, you know the files haven't been corrupted or
>   altered since the manifest was recorded.
> - **Provenance:** it does **not** establish where the files came from, whether they are the intended/canonical model
>   release, or that they were not substituted by an attacker. Downloading "by repo name only" is unstable: a repo can
>   be updated, retagged, force-pushed, deleted, or hijacked. The manifest records *what arrived*, not *what was
>   supposed to arrive*.
>
> So "we know exactly what weights are in the box" is currently inaccurate. You know the exact bytes in the box, but you
> don't know that those bytes are the authoritative weights from a specific, immutable source release.
>
> **Minimum change:** pin the exact Hugging Face revision when downloading and record it in the manifest:
> `--revision <full-git-commit-sha>`. Store `repo + commit SHA + file SHA256s`. That binds the hashes to a specific
> immutable snapshot of the source repository. For stronger provenance, also sign the manifest and attach the upstream
> publisher's attestation for that commit.

**Two-peer convergence with Gemini**, which independently flagged the missing `--revision` pin and the same
integrity-versus-provenance gap. This is the same shape as the Phase 3 supply-chain finding: the artifact records what
arrived rather than what was supposed to arrive.
