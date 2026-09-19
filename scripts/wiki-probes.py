#!/usr/bin/env python3
"""Ceiling probes for the wiki-usage rate (docs/briefs/wiki-usage-measurement-brief.md).

A rate built on "calls that opened a wiki page" is only believable if the instrument is shown to
SEE an open when one happens. Each probe asks a question whose answer lives in one wiki page body
and nowhere a peer could otherwise get it (checked below before any call is spent), without
naming the page. Two variants per question:

  hinted    "check the knowledge base listed in your instructions first". The POSITIVE CONTROL:
            a seat whose hinted probes never register an open has an instrument that cannot be
            trusted to report zero, and the report will not publish that seat's rate.
  unhinted  the bare question. How often need alone sends the peer to the wiki: the ceiling.

Every call goes through the daemon's real /ask-agent route with strict_agent (no substitution)
and a fresh empty cwd, so the evidence comes from the same recorder as organic traffic. Results
are appended to reports/wiki-usage/probes.jsonl as each call returns: request id, seat, probe,
variant, whether the answer matched. No response text.

Usage:
  python3 scripts/wiki-probes.py                    # all seats that carry the map
  python3 scripts/wiki-probes.py --seats grok       # one seat
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import datetime as dt
import json
import re
import sys
import tempfile
import threading
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
WIKI = Path.home() / "projects" / "mneme-bosciamem"
TOKEN = Path.home() / ".triumvirate" / "daemon.token"
DAEMON = "http://127.0.0.1:8080/ask-agent"
JOURNAL = REPO / "reports" / "wiki-usage" / "probes.jsonl"
PROBE_ROOT = Path("/private/tmp/wiki-probe")
# Every place a peer's instructions come from. An answer found in any of these could be given
# without opening a page, so that probe would test nothing.
INSTRUCTION_FILES = [Path.home() / ".codex" / "AGENTS.md", Path.home() / ".gemini" / "GEMINI.md",
                     *sorted((Path.home() / ".grok" / "rules").glob("*.md"))]
SEATS = ["grok", "gemini", "codex"]  # the seats that carry the map; deepseek has none

# (probe id, question, page that holds the answer, answer pattern)
PROBES = [
    ("langfuse-port", "On which port does our self-hosted Langfuse run on the homebox?",
     "home-infra-and-remote-access", r"\b3300\b"),
    ("graphiti-migration", "On what date was our Graphiti memory backend migrated from FalkorDB to Neo4j?",
     "graphiti-memory-operations", r"2026-05-26|May 26,? 2026"),
    ("posthog-project-name", "What is the name of our PostHog Cloud project 523080?",
     "posthog-and-observability", r"Entire Enchilada"),
    ("roofprint-residential", "What roofprint-to-recorded-building-area ratio did we measure for residential properties?",
     "tellus-and-real-estate-data", r"0?\.49\b"),
    ("gemini-spend-cap", "What per-session spend cap applies to Gemini API calls in our agent system?",
     "how-mike-works", r"\$\s?5(\.00)?\b|GEMINI_SESSION_CAP"),
    ("jira-description-cap", "Roughly how many words does our Jira economy gate allow in an issue description?",
     "jira-and-work-tracking", r"\b450\b"),
]
VARIANTS = {
    "unhinted": "Question about our own setup: {q} If you do not know, reply UNKNOWN. Answer in one line.",
    "hinted": "Question about our own setup: {q} Before answering, check the knowledge base listed in "
              "your instructions. If you do not know, reply UNKNOWN. Answer in one line.",
}

_journal_lock = threading.Lock()


def preflight() -> list[str]:
    """Refuse to spend a call on a probe that cannot test anything."""
    problems = []
    context = "\n".join(p.read_text(errors="replace") for p in INSTRUCTION_FILES if p.is_file())
    for pid, q, page, pattern in PROBES:
        body = (WIKI / f"{page}.md")
        if not body.is_file():
            problems.append(f"{pid}: page {body} is missing")
            continue
        if not re.search(pattern, body.read_text(errors="replace")):
            problems.append(f"{pid}: the answer /{pattern}/ is no longer in {page}.md")
        if re.search(pattern, context):
            problems.append(f"{pid}: the answer is in an instruction file, so it needs no lookup")
        if page in q or re.search(r"\b" + re.escape(page) + r"\b", q):
            problems.append(f"{pid}: the question names its page")
    return problems


def ask(seat: str, prompt: str, cwd: Path, token: str) -> dict:
    body = json.dumps({"agent": seat, "message": prompt, "cwd": str(cwd), "strict_agent": True,
                       "context": "wiki-usage ceiling probe"}).encode()
    req = urllib.request.Request(DAEMON, data=body, method="POST", headers={
        "Content-Type": "application/json", "Authorization": f"Bearer {token}"})
    with urllib.request.urlopen(req, timeout=900) as r:
        return json.loads(r.read())


def run_one(seat: str, probe: tuple, variant: str, token: str) -> dict:
    pid, q, page, pattern = probe
    cwd = Path(tempfile.mkdtemp(prefix=f"{seat}-{pid}-{variant}-", dir=PROBE_ROOT))
    row = {"ts": dt.datetime.now(dt.timezone.utc).isoformat(), "seat": seat, "probe": pid,
           "variant": variant, "target_page": page, "cwd": str(cwd)}
    try:
        resp = ask(seat, VARIANTS[variant].format(q=q), cwd, token)
        row.update(request_id=resp.get("request_id"), answered=True,
                   answer_matched=bool(re.search(pattern, resp.get("response", ""), re.I)),
                   tool_calls_made=resp.get("tool_calls_made"))
    except urllib.error.HTTPError as e:
        row.update(answered=False, error=f"HTTP {e.code}: {e.read()[:200].decode(errors='replace')}")
    except Exception as e:  # noqa: BLE001  a probe failure is recorded, never fatal
        row.update(answered=False, error=f"{type(e).__name__}: {e}"[:300])
    with _journal_lock, JOURNAL.open("a") as f:
        f.write(json.dumps(row) + "\n")
    return row


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--seats", nargs="+", default=SEATS, choices=SEATS)
    args = ap.parse_args()
    problems = preflight()
    if problems:
        print("PREFLIGHT FAILED, no calls made:")
        for p in problems:
            print("  -", p)
        return 2
    token = TOKEN.read_text().strip()
    PROBE_ROOT.mkdir(parents=True, exist_ok=True)
    JOURNAL.parent.mkdir(parents=True, exist_ok=True)
    jobs = [(s, p, v) for s in args.seats for p in PROBES for v in VARIANTS]
    print(f"{len(jobs)} probe calls across {', '.join(args.seats)}; journal {JOURNAL}")
    # One worker per seat: a seat's calls run in order, seats run side by side.
    by_seat = {s: [j for j in jobs if j[0] == s] for s in args.seats}
    failed = 0
    with cf.ThreadPoolExecutor(max_workers=len(by_seat)) as pool:
        futures = [pool.submit(lambda js: [run_one(*j, token) for j in js], js) for js in by_seat.values()]
        for fut in cf.as_completed(futures):
            for row in fut.result():
                failed += not row["answered"]
                print(f"{row['seat']:7} {row['probe']:22} {row['variant']:9} "
                      + ("ANSWERED matched=" + str(row["answer_matched"]) if row["answered"] else "FAILED " + row["error"][:90]))
    print(f"done: {len(jobs) - failed} answered, {failed} failed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
