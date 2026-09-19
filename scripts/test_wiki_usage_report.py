#!/usr/bin/env python3
"""Positive control for scripts/wiki-usage-report.py (the brief's mandatory fixture).

A report that says zero is indistinguishable from a broken one. This builds two throwaway ledgers
holding one of every signal and one of every near miss, runs the real report over them, and
fails if any expected count comes back wrong or zero. The brief's minimum: a clean citation, a
path-embedded near miss, a recited map, a degraded row, and a call from a second ledger.

The path near miss is scored by the DETECTOR (Rust, checked against the shared fixture in
scripts/fixtures/wiki-detector-cases.json), so here it arrives as the evidence the daemon would
have written for it: a response that mentioned a page only inside a path has `text_ids: []`.
"""
import importlib.util
import json
import sqlite3
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("wiki_usage_report", REPO / "scripts" / "wiki-usage-report.py")
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)

SCHEMA = """CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, event_type TEXT NOT NULL,
    sequence INTEGER NOT NULL, timestamp TEXT NOT NULL, payload_json TEXT NOT NULL,
    compression_state TEXT NOT NULL DEFAULT 'pending', compression_heartbeat TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')), UNIQUE(session_id, event_type, sequence))"""
WIKI = "/w/mneme-bosciamem"
AFTER = "2026-09-20T12:00:00+00:00"
BEFORE = "2026-09-18T12:00:00+00:00"
IDS = ["how-mike-works", "writing-rules-no-dashes-no-timelines", "graphiti-memory-operations",
       "peer-review-and-delegation", "verify-by-reading-back", "aws-operations"]


def ev(agent="codex", text_ids=(), opened=(), blind=False, prompt_ids=(), prompt_paths=(), **top):
    evidence = {"parser_mode": "x", "backend": agent, "tool_records": not blind,
                "reads_classified": not blind, "prompt_paths": list(prompt_paths),
                "wiki": {"dir": WIKI, "generated": "2026-09-19", "pages": 20, "map_bytes": 1},
                "text_ids": list(text_ids), "prompt_ids": list(prompt_ids),
                "pages_opened": None if blind else list(opened)}
    p = {"schema": 2, "agent_requested": agent, "answered_by_agent": agent, "outcome": "answered",
         "is_peer_review": False, "required_sources": [], "degraded_from_backend": None,
         "evidence": evidence}
    p.update(top)
    return p


CASES_A = [
    ("clean citation", AFTER, ev(text_ids=["aws-operations"])),
    ("path near miss", AFTER, ev(text_ids=[])),
    ("opened a page", AFTER, ev(opened=["verify-by-reading-back"])),
    ("recited map", AFTER, ev(text_ids=IDS)),
    ("degraded", AFTER, ev(agent="gemini", answered_by_agent="codex", degraded_from_backend="agy")),
    ("prompt named a page", AFTER, ev(prompt_ids=["aws-operations"], text_ids=["aws-operations"])),
    ("prompt pointed at a page path", AFTER, ev(prompt_paths=[f"{WIKI}/aws-operations.md"])),
    ("wiki is the subject", AFTER, ev(prompt_paths=["/x/CLAUDE-WIKI/plan.md"])),
    ("failed", AFTER, {"schema": 2, "agent_requested": "codex", "outcome": "failed"}),
    ("schema 1", AFTER, {"schema": 1, "agent_requested": "codex", "answered_by_agent": "codex", "outcome": "answered"}),
    ("wiki unloadable", AFTER, {**ev(), "evidence": {"wiki": {"error": "missing"}}}),
    ("evidence missing", AFTER, {**ev(), "evidence": "missing"}),
    ("test harness", AFTER, {**ev(text_ids=["aws-operations"]), "evidence": {**ev(text_ids=["aws-operations"])["evidence"], "harness": "cargo-test"}}),
]
CASES_B = [  # the SECOND ledger: a reader that opens one database misses all of these
    ("second ledger citation", AFTER, ev(agent="grok", text_ids=["how-mike-works"], blind=True)),
    ("baseline arm", BEFORE, ev(agent="grok", text_ids=[])),
    ("no-map seat", AFTER, ev(agent="deepseek", text_ids=[])),
]


def build(root: Path, name: str, cases) -> None:
    d = root / name / ".triumvirate"
    d.mkdir(parents=True)
    conn = sqlite3.connect(d / "ledger.db")
    conn.execute(SCHEMA)
    for i, (label, ts, payload) in enumerate(cases):
        conn.execute("INSERT INTO events (session_id,event_type,sequence,timestamp,payload_json) "
                     "VALUES (?,?,?,?,?)", (f"{name}-{i}", "wiki_call", 1, ts, json.dumps(payload)))
    # A pruned row: sqlite_sequence must still show it was written.
    conn.execute("INSERT INTO events (session_id,event_type,sequence,timestamp,payload_json) "
                 "VALUES ('gone','other',1,'x','{}')")
    conn.execute("DELETE FROM events WHERE session_id='gone'")
    conn.commit()
    conn.close()


def main() -> int:
    failures = []

    def check(label, got, want):
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        build(root, "proj-a", CASES_A)
        build(root, "proj-b", CASES_B)
        inventory, summary, text = r.run([root])

    check("ledgers found", len(inventory), 2)
    check("pruned rows visible", sorted(i["events_ever"] - i["events_now"] for i in inventory), [1, 1])
    g = summary["groups"]
    x = summary["excluded"]
    codex = g.get(("codex", "delivered", "ask"), {})
    check("codex calls in population", codex.get("calls"), 3)
    check("codex text-cited calls (clean citation only; path near miss is not)", codex.get("text_cited"), 1)
    check("codex opened a page", codex.get("opened_any"), 1)
    check("codex opens observable", codex.get("opens_observable"), 3)
    check("recitation held apart", x.get(("codex", "recitation")), 1)
    check("degraded keyed to the seat that answered", x.get(("codex", "degraded")), 1)
    check("degraded never credited to the asked seat", any(k[0] == "gemini" for k in list(g) + list(x)), False)
    check("prompt-named pages held apart", x.get(("codex", "prompt named a page")), 2)
    check("wiki-subject held apart", x.get(("codex", "wiki is the subject")), 1)
    check("failed held apart", x.get(("codex", "failed call")), 1)
    check("schema 1 held apart", x.get(("codex", "schema 1 (no evidence recorded)")), 1)
    check("unloadable wiki held apart", x.get(("codex", "wiki not loadable at call time")), 1)
    check("missing evidence held apart", x.get(("codex", "evidence missing")), 1)
    check("test-suite calls held apart", x.get(("codex", "test harness (cargo-test)")), 1)
    grok = g.get(("grok", "delivered", "ask"), {})
    check("second ledger counted", grok.get("text_cited"), 1)
    check("blind parser: opens not observable, not zero", grok.get("opens_observable"), 0)
    check("baseline arm split", g.get(("grok", "baseline", "ask"), {}).get("calls"), 1)
    check("seat with no map", g.get(("deepseek", "no map", "ask"), {}).get("calls"), 1)
    check("report refuses to publish a rate", "NO RATE IS PUBLISHED" in text, True)

    # Zero counts are the failure this fixture exists for.
    zeros = [k for k, v in {**{str(k): v for k, v in x.items()}}.items() if not v]
    check("no zero exclusion counts", zeros, [])

    for f in failures:
        print("FAIL", f)
    print(f"{'FAILED' if failures else 'ok'}: positive control, {len(CASES_A) + len(CASES_B)} fixture calls in 2 ledgers")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
