#!/usr/bin/env python3
"""Probe every Triumvirate path that spawns an agent CLI, and journal each result as it lands.

Why this exists (2026-09-13): codex 0.154.0 removed `--full-auto` from `codex exec`, Google
retired the Gemini CLI individual tier, and three Triumvirate dispatch surfaces were emitting
argv the installed binaries reject while every unit test stayed green. The tests assert what
Triumvirate BUILDS; this script asserts what the installed binaries ACCEPT, end to end, through
a FRESH `triumvirate mcp` process on the installed binary (so a stale bridge in an open Claude
Code session cannot contaminate the result).

Atomic by design: every probe appends one JSON line to the journal the moment it finishes. A
crash at probe 17 leaves 16 results on disk. Re-running skips probes already recorded as ok
unless --rerun-all is given.

Usage:
  python3 scripts/audit-cli-paths.py --repo <throwaway git repo> [--journal PATH] [--only id,id]
                                     [--agents codex,gemini,grok] [--rerun-all]
"""
import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

BIN = os.path.expanduser("~/.local/bin/triumvirate")
PER_CALL_TIMEOUT = 900  # s; sight-gated reviews can run past the daemon's 180s connector timeout x3


class Mcp:
    def __init__(self):
        self.p = subprocess.Popen([BIN, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.n = 0
        self._call("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                                  "clientInfo": {"name": "audit-cli-paths", "version": "1"}})
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def _send(self, o):
        self.p.stdin.write(json.dumps(o) + "\n")
        self.p.stdin.flush()

    def _call(self, method, params, timeout=PER_CALL_TIMEOUT):
        self.n += 1
        rid = self.n
        self._send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        deadline = time.time() + timeout
        while time.time() < deadline:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("mcp server closed stdout")
            try:
                o = json.loads(line)
            except json.JSONDecodeError:
                continue
            if o.get("id") == rid:
                return o
        raise TimeoutError(f"{method} exceeded {timeout}s")

    def tool(self, name, args, timeout=PER_CALL_TIMEOUT):
        o = self._call("tools/call", {"name": name, "arguments": args}, timeout)
        if "error" in o:
            return False, json.dumps(o["error"])
        res = o["result"]
        text = "\n".join(c.get("text", "") for c in res.get("content", []) if c.get("type") == "text")
        return not res.get("isError", False), text

    def close(self):
        try:
            self.p.terminate()
        except Exception:
            pass


def journal_append(path: Path, rec: dict):
    with path.open("a") as f:
        f.write(json.dumps(rec, ensure_ascii=False) + "\n")


def already_ok(path: Path):
    done = set()
    if path.exists():
        for line in path.read_text().splitlines():
            try:
                r = json.loads(line)
            except json.JSONDecodeError:
                continue
            if r.get("ok"):
                done.add(r["id"])
    return done


def head(s, n=600):
    s = s or ""
    return s if len(s) <= n else s[:n] + f"... [+{len(s) - n} chars]"


def run_probe(journal, pid, kind, fn, meta):
    started = time.time()
    rec = {"id": pid, "kind": kind, **meta, "started": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
    try:
        ok, detail = fn()
    except Exception as e:  # journal the crash, keep going
        ok, detail = False, f"harness exception: {type(e).__name__}: {e}"
    rec.update({"ok": bool(ok), "elapsed_s": round(time.time() - started, 1), "detail": head(detail)})
    journal_append(journal, rec)
    print(f"[{'OK ' if ok else 'FAIL'}] {pid} ({rec['elapsed_s']}s) {head(detail, 160)!r}", flush=True)
    return ok, detail


def cli(cmd, timeout=180, cwd=None):
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=cwd,
                             stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        return False, f"timed out after {timeout}s"
    body = (out.stdout + "\n" + out.stderr).strip()
    return out.returncode == 0, f"exit={out.returncode}\n{body}"


def parse_json_text(text):
    try:
        return json.loads(text)
    except Exception:
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True, help="throwaway git repo for write-capable probes")
    ap.add_argument("--journal", default=None)
    ap.add_argument("--only", default=None, help="comma-separated probe ids")
    ap.add_argument("--agents", default="codex,gemini,grok")
    ap.add_argument("--rerun-all", action="store_true")
    a = ap.parse_args()

    repo = Path(a.repo).resolve()
    assert (repo / ".git").exists(), f"{repo} is not a git repo"
    journal = Path(a.journal) if a.journal else Path(__file__).resolve().parent.parent / "docs" / "audits" / \
        f"{time.strftime('%Y-%m-%d')}-cli-paths" / "progress.jsonl"
    journal.parent.mkdir(parents=True, exist_ok=True)
    agents = [x.strip() for x in a.agents.split(",") if x.strip()]
    only = set(a.only.split(",")) if a.only else None
    skip = set() if a.rerun_all else already_ok(journal)
    src_file = str(repo / "calc.py")
    sha = subprocess.run(["git", "rev-parse", "HEAD"], cwd=repo, capture_output=True, text=True).stdout.strip()

    probes = []  # (id, kind, meta, fn)

    def add(pid, kind, meta, fn):
        probes.append((pid, kind, meta, fn))

    # ---- baselines: the CLIs themselves, outside Triumvirate -------------------------------
    add("cli.codex.exec", "cli", {"agent": "codex"},
        lambda: cli(["codex", "exec", "--sandbox", "read-only", "--skip-git-repo-check", "--",
                     "Reply with exactly OK."], cwd=str(repo)))
    add("cli.agy.print", "cli", {"agent": "gemini"},
        lambda: cli(["agy", "-p", "Reply with exactly OK.", "--output-format", "stream-json",
                     "--print-timeout", "120s"], cwd=str(repo)))
    add("cli.gemini.print", "cli", {"agent": "gemini-cli"},
        lambda: cli(["gemini", "-p", "Reply with exactly OK."], timeout=90, cwd=str(repo)))

    # ---- MCP surfaces --------------------------------------------------------------------
    mcp = Mcp()
    add("mcp.ping", "mcp", {}, lambda: mcp.tool("ping", {}))
    add("mcp.daemon_health", "mcp", {}, lambda: mcp.tool("daemon_health", {}))

    review_msg = (f"Read {src_file} in full with one whole-file read (no head, tail, limit or offset). "
                  "Reply with exactly the name of the function it defines and nothing else.")

    for ag in agents:
        add(f"ask_agent.{ag}.consult", "mcp", {"agent": ag, "tool": "ask_agent"},
            lambda ag=ag: mcp.tool("ask_agent", {"agent": ag, "cwd": str(repo),
                                                 "message": "Reply with exactly OK and nothing else."}))
        add(f"ask_agent.{ag}.review_sight", "mcp", {"agent": ag, "tool": "ask_agent+required_sources"},
            lambda ag=ag: mcp.tool("ask_agent", {"agent": ag, "cwd": str(repo), "message": review_msg,
                                                 "require_sight": True, "required_sources": [src_file],
                                                 **({"grok_depth": "deep"} if ag == "grok" else {})}))
        add(f"review_agent.{ag}", "mcp", {"agent": ag, "tool": "review_agent"},
            lambda ag=ag: mcp.tool("review_agent", {"agent": ag, "cwd": str(repo), "message": review_msg,
                                                    "sources": [src_file]}))

        def session_roundtrip(ag=ag):
            name = f"audit-{ag}-{int(time.time())}"
            ok, t = mcp.tool("spawn_session", {"agent": ag, "name": name, "cwd": str(repo)})
            if not ok:
                return False, f"spawn_session: {t}"
            ok, t1 = mcp.tool("ask_session", {"name": name, "message": "Remember the word PELICAN. Reply OK."})
            if not ok:
                return False, f"ask_session turn 1: {t1}"
            ok, t2 = mcp.tool("ask_session", {"name": name, "message": "What word did I ask you to remember? One word."})
            mcp.tool("dismiss_session", {"name": name})
            if not ok:
                return False, f"ask_session turn 2: {t2}"
            resumed = "PELICAN" in t2.upper()
            return resumed, f"turn1={head(t1, 200)}\nturn2={head(t2, 200)}\nresume_carried_state={resumed}"
        add(f"session.{ag}.spawn_ask_resume", "mcp", {"agent": ag, "tool": "spawn_session+ask_session"}, session_roundtrip)

    if "gemini" in agents:
        add("query_antigravity", "mcp", {"agent": "gemini", "tool": "query_antigravity"},
            lambda: mcp.tool("query_antigravity", {"query": "Reply with exactly OK."}))
        add("query_gemini.alias", "mcp", {"agent": "gemini", "tool": "query_gemini"},
            lambda: mcp.tool("query_gemini", {"query": "Reply with exactly OK."}))

    if "codex" in agents:
        def abe_wait(task_id, budget=600):
            deadline = time.time() + budget
            last = ""
            while time.time() < deadline:
                ok, t = mcp.tool("get_task_status", {"task_id": task_id}, timeout=60)
                last = t
                j = parse_json_text(t) or {}
                status = str(j.get("status") or j.get("state") or "").lower()
                if status and status not in ("running", "pending", "queued", "started", "working"):
                    return status, t
                time.sleep(5)
            return "timeout", last

        def dispatch_plain():
            marker = f"touched-by-dispatch_codex-{int(time.time())}"
            ok, t = mcp.tool("dispatch_codex", {"cwd": str(repo), "timeout_sec": 300,
                                                "prompt": f"Append the single line '{marker}' to README.md. Do nothing else. Do not commit."})
            if not ok:
                return False, f"dispatch: {t}"
            j = parse_json_text(t) or {}
            task_id = j.get("task_id") or j.get("id")
            if not task_id:
                return False, f"no task_id in dispatch response: {head(t, 300)}"
            status, st = abe_wait(task_id)
            _, out = mcp.tool("get_task_output", {"task_id": task_id}, timeout=60)
            wrote = marker in (repo / "README.md").read_text()
            subprocess.run(["git", "checkout", "--", "README.md"], cwd=repo)
            return wrote, f"status={status} file_written={wrote}\nstatus={head(st, 300)}\noutput={head(out, 400)}"
        add("dispatch_codex.plain", "mcp", {"agent": "codex", "tool": "dispatch_codex"}, dispatch_plain)

        def dispatch_worktree():
            contract = {
                "task_id": f"AUDIT-{int(time.time())}", "req_ids": ["AUDIT-1"], "wave": 1,
                "file_policy": "default-deny", "allowed_files": ["calc.py"], "forbidden_files": [],
                "allowed_commands": [["python3", "-c", "import calc"]], "forbidden_commands": [],
                "commit_format": "feat(AUDIT-1): {summary}", "test_command": "python3 -c 'import calc; assert calc.sub(3,1)==2'",
                "task_timeout_sec": 300, "done_when": "calc.sub(a, b) exists and returns a - b",
                "reality_test": "python3 -c 'import calc; print(calc.sub(3,1))' prints 2",
            }
            ok, t = mcp.tool("dispatch_codex_worktree", {
                "project_root": str(repo), "sha": sha, "contract_fields": contract,
                "briefing_content": "Add `def sub(a, b): return a - b` to calc.py. Commit with the contract's format. Nothing else."})
            if not ok:
                return False, f"dispatch: {t}"
            j = parse_json_text(t) or {}
            task_id = j.get("task_id") or j.get("id")
            if not task_id:
                return False, f"no task_id in dispatch response: {head(t, 300)}"
            status, st = abe_wait(task_id)
            _, out = mcp.tool("get_task_output", {"task_id": task_id}, timeout=60)
            good = status in ("completed", "succeeded", "success", "done")
            return good, f"status={status}\nstatus={head(st, 400)}\noutput={head(out, 500)}"
        add("dispatch_codex.worktree", "mcp", {"agent": "codex", "tool": "dispatch_codex_worktree"}, dispatch_worktree)

    def fleet_repo(ag):
        # One repo per fleet probe: task ids are a repo-wide primary key (D-015), so a second
        # fleet in the same repo fails on a duplicate T-001 and would mask the real result.
        sub = repo.parent / f"{repo.name}-fleet-{ag}"
        if sub.exists():
            subprocess.run(["rm", "-rf", str(sub)])
        sub.mkdir(parents=True)
        subprocess.run(["git", "init", "-q"], cwd=sub, check=True)
        (sub / "calc.py").write_text("def add(a, b):\n    return a + b\n")
        (sub / "README.md").write_text(f"# fleet probe {ag}\n")
        subprocess.run(["git", "add", "-A"], cwd=sub, check=True)
        subprocess.run(["git", "-c", "user.email=audit@local", "-c", "user.name=audit", "commit", "-qm", "init"], cwd=sub, check=True)
        return sub

    for ag in agents:
        def fleet(ag=ag):
            frepo = fleet_repo(ag)
            ok, t = mcp.tool("fleet_spawn", {"project_root": str(frepo), "agents": [ag], "dry_run": False, "wait": False,
                                             "task_description": "Add a function mul(a, b) returning a * b to calc.py."})
            if not ok:
                return False, f"fleet_spawn: {t}"
            j = parse_json_text(t) or {}
            fid = j.get("fleet_id")
            if not fid:
                return False, f"no fleet_id: {head(t, 300)}"
            deadline = time.time() + 600
            last = ""
            while time.time() < deadline:
                ok, s = mcp.tool("fleet_status", {"fleet_id": fid}, timeout=60)
                last = s
                low = s.lower()
                if any(k in low for k in ('"completed"', '"failed"', '"done"', '"error"', '"cancelled"')):
                    break
                time.sleep(10)
            # The ledger's terminal success state is "done" (fleet_status reports the ledger
            # since recovery step 6); "completed" was the harness's guess and never matched.
            low = last.lower()
            good = '"failed"' not in low and '"error"' not in low and ('"done"' in low or '"completed"' in low)
            mcp.tool("fleet_cancel", {"fleet_id": fid}, timeout=60)
            return good, f"fleet_id={fid}\n{head(last, 600)}"
        add(f"fleet_spawn.{ag}", "mcp", {"agent": ag, "tool": "fleet_spawn"}, fleet)

    # ---- run -----------------------------------------------------------------------------
    print(f"journal: {journal}\nrepo: {repo}\nprobes: {len(probes)}", flush=True)
    for pid, kind, meta, fn in probes:
        if only and pid not in only:
            continue
        if pid in skip:
            print(f"[skip] {pid} already ok in journal", flush=True)
            continue
        run_probe(journal, pid, kind, fn, meta)
    mcp.close()

    # ---- summary -------------------------------------------------------------------------
    rows = [json.loads(l) for l in journal.read_text().splitlines() if l.strip()]
    latest = {}
    for r in rows:
        latest[r["id"]] = r
    print("\n== summary ==")
    for pid, r in latest.items():
        print(f"{'OK ' if r['ok'] else 'FAIL'}  {pid:40s} {r['elapsed_s']:>7}s  {head(r['detail'], 110)!r}")


if __name__ == "__main__":
    main()
