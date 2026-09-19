#!/usr/bin/env python3
"""The two free controls from docs/briefs/wiki-usage-measurement-brief.md, run before any Rust.

1. NEGATIVE CONTROL. Grok received no wiki map from any route before 2026-09-19 15:47 UTC. The
   detector run over Grok's responses from before then must return ZERO. If it fires, the
   detector is wrong, and every later rate built on it is wrong too.
2. COINCIDENCE FLOOR. Codex and Gemini have carried the map since 2026-09-19 14:23 UTC. Their
   responses from before then show how often a page id turns up by coincidence. A delivered-arm
   rate means nothing until it is compared against this.

The peer responses come from PostHog's `posthog.ai_events.output_choices`: peers leave no
transcript on this machine, and the bridge's own ledger has never written an ask-path event.

The detector is NOT re-derived here. The page list is imported from the Claude-side scanner
(`mneme-bosciamem/ops/usage_events.py:page_ids`) and the pattern is the same bare-id expression it
uses, so the two sides cannot silently disagree about what a page id is. The positive control at
the top proves this detector can fire before any zero it returns is believed.

Usage:  python3 scripts/wiki-controls.py            # writes reports/wiki-usage/controls-<date>.md
"""
from __future__ import annotations

import datetime as dt
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
MNEME = Path("/Users/michaelboscia/projects/mneme-bosciamem")
ENV_FILE = Path("/Users/michaelboscia/projects/posthog-instrumentation/local_posthog.env")

# From the brief. Both are UTC.
GROK_MAP_START = "2026-09-19 15:47:00"
CODEX_GEMINI_MAP_START = "2026-09-19 14:23:00"
RECITATION_DISTINCT = 5  # the brief's rule: five or more distinct page ids is a recited map

# A call whose SUBJECT is the wiki itself: building it, labelling for it, reviewing it. In those
# calls naming page ids is the job, not use of the map. Found by the negative control on its first
# run: all 14 Grok hits before the map existed, and all 7 Gemini floor hits, were the mneme blind-
# labeler jury and the wiki bootstrap review, whose prompts point at the wiki's own repo. The page
# ids were in FILES the peer opened, so the prompt string held only a path to them.
#
# NEVER let a term of this rule appear in the map itself. The first version included
# `bosciamem wiki`, and the map's first line is `# bosciamem wiki index`: the moment the map is
# delivered through the prompt, every delivered call would classify as wiki-subject and the entire
# delivered arm would vanish from the measurement. Grok found it in review. `main` now refuses to
# run if any term here matches the map.
WIKI_SUBJECT = re.compile(
    r"mneme-bosciamem|CLAUDE-WIKI|WIKI-BRAIN|/gold/|LABELING-BRIEF", re.I)

sys.path.insert(0, str(MNEME / "ops"))
from usage_events import page_ids  # noqa: E402  single source for the page list


def detector(pages: list[str]) -> re.Pattern:
    """The Claude-side scanner's bare-id pattern, character for character (usage_events.py:167)."""
    return re.compile(r"(?<![\w/-])(" + "|".join(re.escape(p) for p in pages) + r")(?![\w-])")


def score(text: str, bare: re.Pattern) -> tuple[list[str], int, bool]:
    ids = [m.group(1) for m in bare.finditer(text)]
    distinct = len(set(ids))
    return ids, distinct, distinct >= RECITATION_DISTINCT


def positive_control(bare: re.Pattern, pages: list[str]) -> list[str]:
    """Prove the detector can fire and can refuse, before any zero it returns is believed.

    A scanner that reports zero is indistinguishable from a broken one. Returns failures.
    """
    p0, p1 = pages[0], pages[1]
    cases = [
        ("clean citation", f"See {p0} for the rule.", 1, 1, False),
        ("two citations", f"{p0} and {p1} both apply.", 2, 2, False),
        ("path-embedded near miss", f"research/{p0}-notes.md is not a citation", 0, 0, False),
        ("slash-prefixed near miss", f"mneme-bosciamem/{p0}.md", 0, 0, False),
        ("hyphen-suffixed near miss", f"{p0}-extended", 0, 0, False),
        ("recited map", " ".join(pages[:6]), 6, 6, True),
        ("no page at all", "a response about something else entirely", 0, 0, False),
    ]
    failures = []
    for label, text, want_refs, want_distinct, want_recite in cases:
        ids, distinct, recite = score(text, bare)
        if (len(ids), distinct, recite) != (want_refs, want_distinct, want_recite):
            failures.append(f"{label}: got refs={len(ids)} distinct={distinct} recitation={recite}, "
                            f"want {want_refs}/{want_distinct}/{want_recite}  text={text!r}")
    return failures


def load_env() -> dict:
    env = {}
    for line in ENV_FILE.read_text().splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            k, v = line.split("=", 1)
            env[k.strip()] = v.strip().strip('"').strip("'")
    return env


def hogql(query: str, env: dict) -> list[list]:
    body = json.dumps({"query": {"kind": "HogQLQuery", "query": query}})
    out = subprocess.run(
        ["curl", "-s", "--max-time", "180", "-X", "POST",
         f"https://us.posthog.com/api/projects/{env['POSTHOG_PROJECT_ID']}/query/",
         "-H", f"Authorization: Bearer {env['POSTHOG_API_KEY']}",
         "-H", "Content-Type: application/json", "-d", body],
        capture_output=True, text=True, check=True,
    ).stdout
    d = json.loads(out)
    # PostHog reports a failed query in `detail`, not `error`. Checking only `error` turned a
    # validation failure into an empty-looking result earlier today.
    err = d.get("error") or (d.get("detail") if d.get("type") else None)
    if err:
        raise SystemExit(f"QUERY FAILED: {str(err)[:400]}")
    return d.get("results") or []


def response_text(output_choices) -> str:
    """Flatten `output_choices` ([{role, content}], content a string or a list of parts)."""
    if output_choices is None:
        return ""
    if isinstance(output_choices, str):
        try:
            output_choices = json.loads(output_choices)
        except json.JSONDecodeError:
            return output_choices
    parts = []
    for choice in output_choices if isinstance(output_choices, list) else [output_choices]:
        content = choice.get("content") if isinstance(choice, dict) else choice
        if isinstance(content, str):
            parts.append(content)
        elif isinstance(content, list):
            parts.extend(p.get("text", "") if isinstance(p, dict) else str(p) for p in content)
    return "\n".join(parts)


def population(seat: str, before_utc: str, env: dict) -> list[dict]:
    """Every successful pre-map generation for one seat, with the fields the brief needs.

    Only `success`. A `degraded_success` was answered by a DIFFERENT seat (the brief: credit the
    seat that answered, never the seat that was asked), so it is counted separately and kept out
    of both seats' rates.
    """
    # ONE query, then PROVE it returned everything. PostHog refuses OFFSET with a personal key; its
    # suggested keyset `timestamp > last` skips rows that tie on the boundary, and HogQL has only a
    # seconds-precision parser, which would misorder rows within one second. Rather than trust a
    # pager, fetch with a generous LIMIT and assert the row count equals an independent count().
    where = ("FROM posthog.ai_events WHERE event = '$ai_generation' "
             "AND distinct_id = 'triumvirate-daemon' "
             f"AND JSONExtractString(properties,'tv_agent') = '{seat}' "
             "AND JSONExtractString(properties,'tv_outcome') = 'success' "
             f"AND timestamp < toDateTime('{before_utc}', 'UTC') ")
    expected = int(hogql("SELECT count() " + where, env)[0][0])
    batch = hogql(
        "SELECT toString(uuid), timestamp, JSONExtractString(properties,'tv_backend'), output_choices, input "
        + where + "ORDER BY timestamp LIMIT 10000",
        env,
    )
    if len(batch) != expected:
        raise SystemExit(f"INCOMPLETE: {seat} returned {len(batch)} rows but count() says {expected}. "
                         "A silently truncated sample would understate every rate; refusing to report.")
    return [{"uuid": u, "ts": ts, "backend": b or "", "text": response_text(oc),
             "wiki_subject": bool(WIKI_SUBJECT.search(response_text(inp)))}
            for u, ts, b, oc, inp in batch]


def call_class(backend: str) -> str:
    """The brief: label the call classes rather than merge them into one rate."""
    return "dispatch" if backend.startswith("dispatch_codex") else "ask"


def summarise(rows: list[dict], bare: re.Pattern) -> dict:
    by_class: dict[str, dict] = {}
    pages_seen: dict[str, int] = {}
    for r in rows:
        ids, distinct, recite = score(r["text"], bare)
        key = "wiki-subject" if r["wiki_subject"] else call_class(r["backend"])
        c = by_class.setdefault(key, {
            "calls": 0, "empty_text": 0, "referencing": 0, "references": 0, "recitations": 0})
        c["calls"] += 1
        if not r["text"].strip():
            c["empty_text"] += 1
        if recite:
            c["recitations"] += 1  # labelled, and excluded from the use numerator
        elif ids:
            c["referencing"] += 1
            c["references"] += len(ids)
            if not r["wiki_subject"]:
                for p in ids:
                    pages_seen[p] = pages_seen.get(p, 0) + 1
    return {"by_class": by_class, "pages": pages_seen}


def degraded_count(env: dict) -> int:
    rows = hogql(
        "SELECT count() FROM posthog.ai_events WHERE event = '$ai_generation' "
        "AND distinct_id = 'triumvirate-daemon' "
        "AND JSONExtractString(properties,'tv_outcome') = 'degraded_success' "
        f"AND timestamp < toDateTime('{CODEX_GEMINI_MAP_START}', 'UTC')",
        env,
    )
    return int(rows[0][0]) if rows else 0


def main() -> int:
    pages = page_ids()
    if len(pages) != 20:
        print(f"WARNING: expected 20 wiki pages, the menu lists {len(pages)}")
    bare = detector(pages)

    map_text = (MNEME / "_index.md").read_text()
    if WIKI_SUBJECT.search(map_text):
        print(f"SUBJECT RULE MATCHES THE MAP ITSELF ({WIKI_SUBJECT.search(map_text).group(0)!r}). "
              "Delivering the map through a prompt would erase the delivered arm. Refusing to run.")
        return 2

    failures = positive_control(bare, pages)
    if failures:
        print("POSITIVE CONTROL FAILED. No zero below can be trusted:")
        for f in failures:
            print("  -", f)
        return 2
    print(f"positive control: the detector fires and refuses correctly on {7} cases")

    env = load_env()
    runs = {
        "grok (NEGATIVE control, must be zero)": population("grok", GROK_MAP_START, env),
        "codex (coincidence floor)": population("codex", CODEX_GEMINI_MAP_START, env),
        "gemini (coincidence floor)": population("gemini", CODEX_GEMINI_MAP_START, env),
    }
    results = {name: summarise(rows, bare) for name, rows in runs.items()}
    held_apart = degraded_count(env)

    today = dt.date.today().isoformat()
    out = REPO / "reports" / "wiki-usage" / f"controls-{today}.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        f"# Wiki usage: the two free controls ({today})",
        "",
        "Run before any Rust, per `docs/briefs/wiki-usage-measurement-brief.md`. Source: "
        "`posthog.ai_events.output_choices`, successful `$ai_generation` rows from "
        "`triumvirate-daemon`, before each seat's map-start time.",
        "",
        f"Detector: the Claude-side bare-id pattern over {len(pages)} page ids imported from "
        "`mneme-bosciamem/ops/usage_events.py`. A response naming "
        f"{RECITATION_DISTINCT}+ distinct page ids is labelled recitation and kept out of the "
        "use numerator.",
        "",
        "Positive control: PASSED (a clean citation, two citations, three near misses, a recited map "
        "and a no-page response each scored as intended), so a zero below is a real zero.",
        "",
        "Rows labelled `wiki-subject` are calls whose prompt points at the wiki's own repo or build "
        "artifacts (the blind-labeler jury, the bootstrap review). Naming page ids is their job, so "
        "they are shown but excluded from the negative control and from the pages list.",
        "",
        "| population | class | calls | empty text | referencing a page | page references | recitations |",
        "|---|---|---|---|---|---|---|",
    ]
    for name, res in results.items():
        for cls, c in sorted(res["by_class"].items()):
            lines.append(f"| {name} | {cls} | {c['calls']} | {c['empty_text']} | "
                         f"{c['referencing']} | {c['references']} | {c['recitations']} |")
    lines += [
        "",
        f"Held apart: **{held_apart}** `degraded_success` generations before {CODEX_GEMINI_MAP_START} UTC. "
        "Each was answered by a seat other than the one asked, so it is credited to neither.",
        "",
        "## Pages referenced, per population",
        "",
    ]
    for name, res in results.items():
        top = sorted(res["pages"].items(), key=lambda kv: -kv[1])
        lines.append(f"- {name}: " + (", ".join(f"`{p}` x{n}" for p, n in top) if top else "none"))
    out.write_text("\n".join(lines) + "\n")

    neg = results["grok (NEGATIVE control, must be zero)"]
    neg_hits = sum(c["referencing"] + c["recitations"]
                   for cls, c in neg["by_class"].items() if cls != "wiki-subject")
    print(out.read_text())
    print(f"wrote {out}")
    if neg_hits:
        print(f"NEGATIVE CONTROL FIRED ({neg_hits} Grok responses): the detector is wrong or the "
              "cutoff is. Stop and fix it before building anything on it.")
        return 1
    print("NEGATIVE CONTROL: zero, as required.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
