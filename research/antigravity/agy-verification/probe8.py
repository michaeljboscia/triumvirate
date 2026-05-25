#!/usr/bin/env python3
"""Can we recover WHICH model served an agy -p request? Check --log-file and transcript JSON."""
import os, subprocess, signal, time, glob, re

BASE = dict(os.environ)
logf = "/tmp/agy_modelprobe.log"
try: os.remove(logf)
except FileNotFoundError: pass
proj = os.path.expanduser("~/.gemini/config/projects")

PAT = re.compile(r'(?i)("?model"?\s*[:=]\s*"?[\w./-]+|gemini[\w.-]*|riftrunner|orionfire|pureprism|cosmicforge|infinityjet|nemosreef|rainsong|horizondawn|gentleisland|flash|pro\b)')

start = time.time()
p = subprocess.Popen(["agy", "-p", "Say hi in exactly one word.", "--log-file", logf],
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True, env=BASE)
try:
    o, e = p.communicate(timeout=60); rc = p.returncode
except subprocess.TimeoutExpired:
    os.killpg(os.getpgid(p.pid), signal.SIGKILL); o, e = p.communicate(); rc = None
print("exit:", rc, "| out:", o.decode(errors="replace").strip()[:80])

print("\n=== --log-file written? ===", os.path.exists(logf))
if os.path.exists(logf):
    txt = open(logf, errors="replace").read()
    print(f"  size={len(txt)}  model-ish hits:", sorted(set(m.group(0).strip()[:40] for m in PAT.finditer(txt)))[:25])

print("\n=== newest transcripts (~/.gemini/config/projects) ===")
files = sorted(glob.glob(proj + "/*.json"), key=os.path.getmtime, reverse=True)[:2]
for f in files:
    print(" ", os.path.basename(f), "mtime", time.ctime(os.path.getmtime(f)))
    try:
        t = open(f, errors="replace").read()
        print(f"    size={len(t)}  model-ish hits:", sorted(set(m.group(0).strip()[:40] for m in PAT.finditer(t)))[:25])
    except Exception as ex:
        print("    err", ex)
