#!/usr/bin/env python3
"""Wiki-usage report over every Triumvirate ledger (docs/briefs/wiki-usage-measurement-brief.md).

The daemon stores raw evidence per call (`wiki_call` events, schema 2). EVERY judgement is made
here, at report time, so a better rule can be re-run over old events: that was the one point the
design jury (2026-09-19) was unanimous on.

The rate (owner's decision, 2026-09-19: the map stays in the peers' own files, no holdout): the
share of eligible delivered calls that CONSULTED the wiki, meaning any successful tool call whose
arguments name the wiki. Opening a page is one way; the first ceiling probes showed it is not the
usual way. Grok answered six of six hinted probes correctly from `grep` and shell output with no
page opened, so a page-open rate would have reported zero use for a seat using the wiki every time. Its floor is zero by construction (the
page files did not exist before 2026-09-17, a peer without the map has no way to know a page path,
and a prompt that names a page is excluded). A seat's rate is published only when a HINTED ceiling
probe (scripts/wiki-probes.py) registered an open of its target page for that seat: the positive
control that the instrument can see an open at all. A seat without one gets no rate, only counts.
Page ids in the response text are reported as a secondary signal, because after delivery they
include map echo.

Usage:
  python3 scripts/wiki-usage-report.py                 # writes reports/wiki-usage/usage-<date>.md
  python3 scripts/wiki-usage-report.py --roots A B     # search other roots for ledgers
"""
from __future__ import annotations

import argparse
import collections
import datetime as dt
import importlib.util
import json
import sqlite3
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
HOME = Path.home()
DEFAULT_ROOTS = [HOME / "projects", HOME / ".triumvirate", Path("/private/tmp")]
MAX_DEPTH = 6
RETENTION_DAYS = 30  # the ledger sweeps events on created_at
PROBE_JOURNAL = REPO / "reports" / "wiki-usage" / "probes.jsonl"

# The subject rule and the thresholds come from the controls script, so there is ONE copy of each.
_spec = importlib.util.spec_from_file_location("wiki_controls", REPO / "scripts" / "wiki-controls.py")
_wc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_wc)
WIKI_SUBJECT = _wc.WIKI_SUBJECT
RECITATION_DISTINCT = _wc.RECITATION_DISTINCT

# When each seat started carrying the map (UTC). Before: baseline. After: delivered.
MAP_START = {
    "codex": dt.datetime(2026, 9, 19, 14, 23, tzinfo=dt.timezone.utc),
    "gemini": dt.datetime(2026, 9, 19, 14, 23, tzinfo=dt.timezone.utc),
    "grok": dt.datetime(2026, 9, 19, 15, 47, tzinfo=dt.timezone.utc),
}


def find_ledgers(roots: list[Path]) -> list[Path]:
    """Every `.triumvirate/ledger.db` under the roots. Enumerated, never remembered: the brief's
    first population lesson is a scan that opened one directory and printed a plausible number."""
    found: set[Path] = set()
    for root in roots:
        if (root / "ledger.db").is_file() and root.name == ".triumvirate":
            found.add((root / "ledger.db").resolve())
        if not root.is_dir():
            continue
        base = len(root.parts)
        for p in root.rglob("ledger.db"):
            if len(p.parts) - base > MAX_DEPTH or p.parent.name != ".triumvirate":
                continue
            if any(part in ("node_modules", "target") for part in p.parts):
                continue
            found.add(p.resolve())
    return sorted(found)


def read_ledger(db: Path) -> tuple[dict, list[dict]]:
    """(inventory row, wiki_call events). The inventory is what stops an empty table being read
    as "nothing happened": sqlite_sequence says how many rows were EVER written."""
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        tables = {r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        if "events" not in tables:
            return {"db": str(db), "error": "no events table"}, []
        seq = conn.execute("SELECT seq FROM sqlite_sequence WHERE name='events'").fetchone()
        total, oldest = conn.execute("SELECT COUNT(*), MIN(created_at) FROM events").fetchone()
        rows = conn.execute(
            "SELECT session_id, timestamp, payload_json FROM events WHERE event_type='wiki_call'"
        ).fetchall()
    finally:
        conn.close()
    events = []
    for sid, ts, payload in rows:
        try:
            p = json.loads(payload)
        except json.JSONDecodeError:
            p = {"_unparseable": True}
        events.append({"db": str(db), "session_id": sid, "timestamp": ts, **p})
    inv = {"db": str(db), "wiki_calls": len(events), "events_now": total,
           "events_ever": seq[0] if seq else 0, "oldest_created_at": oldest}
    return inv, events


def parse_ts(ts: str) -> dt.datetime | None:
    try:
        t = dt.datetime.fromisoformat(ts.replace("Z", "+00:00"))
    except (ValueError, AttributeError):
        return None
    return t if t.tzinfo else t.replace(tzinfo=dt.timezone.utc)


def load_probes(journal: Path) -> dict[str, dict]:
    """request_id -> probe row, for every answered probe in the journal."""
    if not journal.is_file():
        return {}
    rows = [json.loads(line) for line in journal.read_text().splitlines() if line.strip()]
    return {r["request_id"]: r for r in rows if r.get("request_id")}


def consulted(ev: dict) -> bool | None:
    """Did this call touch the wiki with any tool? None when the parser cannot see tool calls.

    Reads both evidence shapes: schema 2 counted wiki touches by kind ({"grep": 1}), schema 3
    lists them ([{"kind": ..., "pages": [...], "index": bool}]).
    """
    calls = ev.get("wiki_tool_calls")
    opened = ev.get("pages_opened")
    if calls is None and opened is None:
        return None
    if calls is None:  # schema 2 events written before the field existed
        return bool(opened) or None
    return bool(calls) or bool(opened)


def wilson(k: int, n: int, z: float = 1.96) -> tuple[float, float]:
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * ((p * (1 - p) / n + z * z / (4 * n * n)) ** 0.5) / d
    return (max(0.0, c - h), min(1.0, c + h))


def classify(e: dict, probes: dict | None = None) -> dict:
    """One call's labels. Order matters: a call leaves the use population at the first rule it
    meets, and every rule that removes it is named in the output."""
    seat = e.get("answered_by_agent") or e.get("agent_requested") or "?"
    ev = e.get("evidence")
    out = {"seat": seat, "db": e["db"], "exclusion": None, "arm": None,
           "text_ids": [], "opened": None, "consulted": None, "call_class": "peer_review" if e.get("is_peer_review") else "ask"}

    probe = (probes or {}).get(e.get("request_id") or e.get("session_id"))
    if probe:
        out["probe"] = probe
    if e.get("_unparseable"):
        out["exclusion"] = "unparseable payload"
    elif e.get("outcome") != "answered":
        out["exclusion"] = "failed call"
    elif e.get("schema", 1) < 2:
        out["exclusion"] = "schema 1 (no evidence recorded)"
    elif not isinstance(ev, dict):
        out["exclusion"] = "evidence missing"
    elif ev.get("harness"):
        # A test-suite call with a stand-in agent, not a peer (see wiki_evidence in agent_exec.rs).
        out["exclusion"] = f"test harness ({ev['harness']})"
    elif "error" in (ev.get("wiki") or {}):
        out["exclusion"] = "wiki not loadable at call time"
    elif (e.get("answered_by_agent") and e.get("answered_by_agent") != e.get("agent_requested")) \
            or e.get("degraded_from_backend"):
        # Credit the seat that answered, and leave it out of BOTH seats' rates.
        out["exclusion"] = "degraded"
    if out["exclusion"]:
        return out

    out["text_ids"] = ev.get("text_ids") or []
    out["opened"] = ev.get("pages_opened")  # None = the parser cannot see reads
    out["consulted"] = consulted(ev)
    if probe:
        out["exclusion"] = "ceiling probe"
        return out

    wiki_dir = (ev.get("wiki") or {}).get("dir") or ""
    pointed = [s for s in (e.get("required_sources") or []) + (ev.get("prompt_paths") or [])]
    if ev.get("prompt_ids") or any(wiki_dir and s.startswith(wiki_dir.rstrip("/") + "/")
                                   and s.endswith(".md") and not Path(s).name.startswith("_")
                                   for s in pointed):
        # "Read this page" is obedience, not map-driven lookup (Grok, jury).
        out["exclusion"] = "prompt named a page"
    elif any(WIKI_SUBJECT.search(s) for s in pointed):
        out["exclusion"] = "wiki is the subject"

    if not out["exclusion"] and len(set(out["text_ids"])) >= RECITATION_DISTINCT:
        out["exclusion"] = "recitation"

    start = MAP_START.get(seat)
    t = parse_ts(e.get("timestamp", ""))
    out["arm"] = "no map" if start is None else ("unknown time" if t is None
                                                  else ("delivered" if t >= start else "baseline"))
    return out


def summarise(classified: list[dict]) -> dict:
    groups: dict[tuple, dict] = collections.defaultdict(lambda: {
        "calls": 0, "text_cited": 0, "page_refs": 0, "opens_observable": 0, "opened_any": 0,
        "consult_observable": 0, "consulted": 0, "pages": collections.Counter()})
    excluded = collections.Counter()
    ceiling: dict[tuple, dict] = collections.defaultdict(lambda: {
        "probes": 0, "opens_observable": 0, "opened_target": 0, "opened_any": 0, "answer_matched": 0,
        "consult_observable": 0, "consulted": 0})
    for c in classified:
        if c.get("probe") and c["exclusion"] == "ceiling probe":
            pr = c["probe"]
            k = ceiling[(c["seat"], pr["variant"])]
            k["probes"] += 1
            k["answer_matched"] += bool(pr.get("answer_matched"))
            if c["consulted"] is not None:
                k["consult_observable"] += 1
                k["consulted"] += bool(c["consulted"])
            if c["opened"] is not None:
                k["opens_observable"] += 1
                k["opened_any"] += bool(c["opened"])
                k["opened_target"] += pr["target_page"] in c["opened"]
        if c["exclusion"]:
            excluded[(c["seat"], c["exclusion"])] += 1
            continue
        g = groups[(c["seat"], c["arm"], c["call_class"])]
        g["calls"] += 1
        if c["consulted"] is not None:
            g["consult_observable"] += 1
            g["consulted"] += bool(c["consulted"])
        if c["text_ids"]:
            g["text_cited"] += 1
            g["page_refs"] += len(c["text_ids"])
            g["pages"].update(c["text_ids"])
        if c["opened"] is not None:
            g["opens_observable"] += 1
            if c["opened"]:
                g["opened_any"] += 1
                g["pages"].update(c["opened"])
    rates = {}
    for seat in sorted({k[0] for k in groups} | {k[0] for k in ceiling}):
        # The positive control: a hinted probe in which the instrument SAW the seat consult the
        # wiki. Requiring a page open here would fail a seat that uses the wiki by searching it.
        proven = ceiling.get((seat, "hinted"), {}).get("consulted", 0) > 0
        pool = [g for (s_, arm, cls), g in groups.items() if s_ == seat and arm == "delivered"]
        n = sum(g["consult_observable"] for g in pool)
        k = sum(g["consulted"] for g in pool)
        rates[seat] = {"proven": proven, "n": n, "k": k,
                       "opened_n": sum(g["opens_observable"] for g in pool),
                       "opened_k": sum(g["opened_any"] for g in pool),
                       "text_n": sum(g["calls"] for g in pool), "text_k": sum(g["text_cited"] for g in pool)}
    return {"groups": dict(groups), "excluded": dict(excluded), "ceiling": dict(ceiling), "rates": rates}


def rate_section(summary: dict) -> list[str]:
    rates = summary["rates"]
    published = [s for s, r in rates.items() if r["proven"]]
    lines = ["## Rate: delivered calls that consulted the wiki", ""]
    if not published:
        lines += ["**NO RATE IS PUBLISHED.** No seat has a hinted ceiling probe in which the instrument",
                  "saw it consult the wiki, so for every seat a zero here could be a blind instrument.", ""]
    lines += ["Floor: 0 by construction (no page files before 2026-09-17; without the map a peer cannot",
              "know a page path; prompt-named pages are excluded). Interval: Wilson 95%.", "",
              "| seat | instrument proven | consulted | rate | 95% interval | of which opened a page | text-cited (secondary, includes map echo) |",
              "|---|---|---:|---:|---|---:|---:|"]
    for seat, r in sorted(rates.items()):
        text = f"{r['text_k']}/{r['text_n']}"
        opened = f"{r['opened_k']}/{r['opened_n']}"
        if r["proven"] and r["n"]:
            lo, hi = wilson(r["k"], r["n"])
            lines.append(f"| {seat} | yes | {r['k']}/{r['n']} | {r['k'] / r['n']:.1%} | {lo:.1%} to {hi:.1%} | {opened} | {text} |")
        elif r["proven"]:
            lines.append(f"| {seat} | yes | 0/0 | no eligible calls yet | | {opened} | {text} |")
        else:
            lines.append(f"| {seat} | NO | {r['k']}/{r['n']} | not published | | {opened} | {text} |")
    ceiling = summary["ceiling"]
    lines += ["", "## Ceiling probes (held apart from the rate)", "",
              "Questions answerable only from one page body, page never named. `hinted` tells the peer to",
              "check its knowledge base and is the positive control; `unhinted` is need alone. An answer",
              "can also come from a peer's other memory tools, so `answer matched` is not proof of a page read.", "",
              "| seat | variant | probes | consult observable | consulted wiki | opened target page | answer matched |",
              "|---|---|---:|---:|---:|---:|---:|"]
    for (seat, variant), k in sorted(ceiling.items()):
        lines.append(f"| {seat} | {variant} | {k['probes']} | {k['consult_observable']} | {k['consulted']} "
                     f"| {k['opened_target']} | {k['answer_matched']} |")
    if not ceiling:
        lines.append("| (none run) | | 0 | | | | |")
    return lines


def render(inventory: list[dict], summary: dict, roots: list[Path]) -> str:
    now = dt.datetime.now(dt.timezone.utc)
    lines = [
        f"# Wiki usage, {now:%Y-%m-%d %H:%M} UTC",
        "",
        *rate_section(summary),
        "",
        f"## Ledgers found: {len(inventory)}",
        "",
        f"Searched {', '.join(str(r) for r in roots)} (depth {MAX_DEPTH}). `events_ever` is",
        f"sqlite_sequence: rows ever written. Retention removes events {RETENTION_DAYS} days after",
        "`created_at`, so `events_now` below `events_ever` is pruning, not silence.",
        "",
        "| ledger | wiki_call | events_now | events_ever | oldest created_at |",
        "|---|---:|---:|---:|---|",
    ]
    for i in inventory:
        if "error" in i:
            lines.append(f"| {i['db']} | {i['error']} | | | |")
        else:
            lines.append(f"| {i['db']} | {i['wiki_calls']} | {i['events_now']} | {i['events_ever']} "
                         f"| {i['oldest_created_at'] or ''} |")
    lines += [
        "",
        "## Calls in the use population",
        "",
        "Keyed on the seat that ANSWERED. `opens observable` counts calls whose parser can tell a",
        "read from anything else; an empty list from a blind parser is not zero opens.",
        "",
        "| seat | arm | class | calls | text-cited calls | page refs | opens observable | opened a page | top pages |",
        "|---|---|---|---:|---:|---:|---:|---:|---|",
    ]
    for (seat, arm, cls), g in sorted(summary["groups"].items()):
        top = ", ".join(f"{p} ({n})" for p, n in g["pages"].most_common(3))
        lines.append(f"| {seat} | {arm} | {cls} | {g['calls']} | {g['text_cited']} | {g['page_refs']} "
                     f"| {g['opens_observable']} | {g['opened_any']} | {top} |")
    if not summary["groups"]:
        lines.append("| (none) | | | 0 | | | | | |")
    lines += ["", "## Held apart, by reason", "", "| seat | reason | calls |", "|---|---|---:|"]
    for (seat, reason), n in sorted(summary["excluded"].items()):
        lines.append(f"| {seat} | {reason} | {n} |")
    if not summary["excluded"]:
        lines.append("| (none) | | 0 |")
    return "\n".join(lines) + "\n"


def run(roots: list[Path], journal: Path = PROBE_JOURNAL) -> tuple[list[dict], dict, str]:
    inventory, events = [], []
    for db in find_ledgers(roots):
        try:
            inv, evs = read_ledger(db)
        except sqlite3.Error as e:
            inv, evs = {"db": str(db), "error": f"unreadable: {e}"}, []
        inventory.append(inv)
        events.extend(evs)
    probes = load_probes(journal)
    summary = summarise([classify(e, probes) for e in events])
    return inventory, summary, render(inventory, summary, roots)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--roots", nargs="+", type=Path, default=DEFAULT_ROOTS)
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()
    inventory, summary, text = run(args.roots)
    if not inventory:
        print("NO LEDGERS FOUND under", ", ".join(map(str, args.roots)), "- refusing to write a report of nothing")
        return 2
    out = args.out or REPO / "reports" / "wiki-usage" / f"usage-{dt.date.today():%Y-%m-%d}.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text)
    print(text)
    print(f"wrote {out} ({len(inventory)} ledgers)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
