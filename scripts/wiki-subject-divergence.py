#!/usr/bin/env python3
"""Measure the O-1 defect: WIKI_SUBJECT is one pattern applied to two different populations.

`scripts/wiki-controls.py:157` matches it against the WHOLE prompt text.
`scripts/wiki-usage-report.py:196` matches it against PATHS ONLY, because `pointed` is
`required_sources + evidence.prompt_paths` and `paths_in` (daemon/crates/triumvirate/src/
wiki_usage.rs:115) drops every token that is not an absolute-ish path.

Both import the pattern from one place, under a comment claiming there is one copy. The pattern
is shared. The population is not, so the two scripts answer differently about the same call.

This script quantifies the disagreement over real rows. PostHog holds the prompt text, so both
surfaces can be reconstructed faithfully from the same row: the full text as the controls script
sees it, and `paths_in(text)` as the daemon records it for the report.

Usage:  python3 scripts/wiki-subject-divergence.py
"""
from __future__ import annotations

import datetime as dt
import importlib.util
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]

_spec = importlib.util.spec_from_file_location("wiki_controls", REPO / "scripts" / "wiki-controls.py")
_wc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_wc)
WIKI_SUBJECT = _wc.WIKI_SUBJECT

# Port of paths_in (wiki_usage.rs:115), character for character in behaviour. Asserted against
# the Rust test's own fixture case below before any number is believed.
_SPLIT = re.compile(r"[\s`\"'()<>,]+")


def paths_in(text: str) -> list[str]:
    out = set()
    for tok in _SPLIT.split(text):
        tok = tok.rstrip(".:;")
        if (tok.startswith("/") or tok.startswith("~/")) and "/" in tok[1:]:
            out.add(tok)
    return sorted(out)[:50]


def parity_check() -> None:
    """The Rust test `paths_in_keeps_paths_and_drops_prose`, run against this port."""
    got = paths_in("Review `/Users/x/projects/mneme-bosciamem/aws-operations.md`, and ~/a/b. "
                   "Not a/b or /root.")
    want = ["/Users/x/projects/mneme-bosciamem/aws-operations.md", "~/a/b"]
    if got != want:
        raise SystemExit(f"PORT IS WRONG. paths_in gave {got}, the Rust test expects {want}. "
                         "Every number below would be measuring the wrong thing.")
    print("parity: the Python port of paths_in matches the Rust test fixture")


def main() -> int:
    parity_check()
    env = _wc.load_env()

    rows = _wc.hogql(
        "SELECT toString(uuid), timestamp, JSONExtractString(properties,'tv_agent'), input "
        "FROM posthog.ai_events WHERE event = '$ai_generation' "
        "AND distinct_id = 'triumvirate-daemon' "
        "ORDER BY timestamp LIMIT 10000",
        env,
    )
    print(f"rows: {len(rows)}")

    both, text_only, paths_only, neither = [], [], [], 0
    for uuid, ts, agent, inp in rows:
        text = _wc.response_text(inp)
        if not text.strip():
            continue
        by_text = bool(WIKI_SUBJECT.search(text))
        by_paths = any(WIKI_SUBJECT.search(p) for p in paths_in(text))
        row = {"uuid": uuid, "ts": str(ts), "agent": agent, "chars": len(text)}
        if by_text and by_paths:
            both.append(row)
        elif by_text:
            m = WIKI_SUBJECT.search(text)
            row["term"] = m.group(0)
            row["context"] = text[max(0, m.start() - 60):m.end() + 60].replace("\n", " ")
            text_only.append(row)
        elif by_paths:
            paths_only.append(row)
        else:
            neither += 1

    scored = len(both) + len(text_only) + len(paths_only) + neither
    flagged = len(both) + len(text_only) + len(paths_only)
    print(f"scored: {scored} rows with text\n")
    print("| surface | wiki-subject | what uses it |")
    print("|---|---|---|")
    print(f"| full prompt text | {len(both) + len(text_only)} | wiki-controls.py, the CONTROLS |")
    print(f"| paths only       | {len(both) + len(paths_only)} | wiki-usage-report.py, the RATES |")
    print()
    print(f"agree (both fire):      {len(both)}")
    print(f"agree (neither fires):  {neither}")
    print(f"DISAGREE, text only:    {len(text_only)}   <- the report counts these as ordinary use")
    print(f"DISAGREE, paths only:   {len(paths_only)}")
    disagreements = len(text_only) + len(paths_only)
    print(f"\ndisagreement: {disagreements} of {scored} rows "
          f"({disagreements / scored * 100:.1f}% of all), "
          f"{disagreements / flagged * 100:.1f}% of rows either surface flags"
          if flagged else "")

    if text_only:
        print("\n## Rows the CONTROLS call wiki-subject and the REPORT does not\n")
        for r in text_only[:12]:
            print(f"- `{r['term']}` {r['agent']} {r['ts'][:19]}")
            print(f"    ...{r['context'][:150]}...")

    out = REPO / "reports" / "wiki-usage" / f"subject-divergence-{dt.date.today().isoformat()}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({
        "scored": scored, "both": len(both), "neither": neither,
        "text_only": text_only, "paths_only": paths_only,
    }, indent=2))
    print(f"\nwrote {out}")
    return 1 if disagreements else 0


if __name__ == "__main__":
    sys.exit(main())
