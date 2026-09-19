#!/usr/bin/env python3
"""The Python half of the detector parity check. The Rust half is `wiki_usage::tests` in the daemon.

Both read scripts/fixtures/wiki-detector-cases.json and must return every case's `want` exactly.
"""
import importlib.util
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("wiki_controls", REPO / "scripts" / "wiki-controls.py")
wc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wc)

cases = json.loads((REPO / "scripts" / "fixtures" / "wiki-detector-cases.json").read_text())["cases"]
bare = wc.detector(wc.page_ids())
failed = 0
for c in cases:
    got = [m.group(1) for m in bare.finditer(c["text"])]
    if got != c["want"]:
        failed += 1
        print(f"FAIL {c['label']}: got {got}, want {c['want']}")
print(f"{len(cases) - failed}/{len(cases)} detector cases agree with the fixture")
sys.exit(1 if failed or not cases else 0)
