#!/usr/bin/env python3
"""Easy-batch verification: env vars, multi-turn -c, concurrency, ARG_MAX, profile robustness."""
import os, subprocess, signal, time, tempfile, textwrap, shutil, threading

os.environ["ANTIGRAVITY_SKIP_UPDATE_CHECK"] = "true"
HOME = os.path.expanduser("~"); TMP = os.environ.get("TMPDIR", "/tmp").rstrip("/")

def run(cmd, cwd=None, timeout=120):
    start = time.time()
    try:
        p = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    except OSError as ex:
        return "OSERR", False, "", f"{type(ex).__name__}: {ex}", round(time.time() - start, 1)
    try:
        o, e = p.communicate(timeout=timeout); rc = p.returncode; to = False
    except subprocess.TimeoutExpired:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL); o, e = p.communicate(); rc = None; to = True
    return rc, to, o.decode(errors="replace"), e.decode(errors="replace"), round(time.time() - start, 1)

print("=" * 60, "\n1. ENV VAR DISCOVERY (strings, free, no model call)")
rc, to, o, e, dt = run(["bash", "-c", "strings $(which agy) | grep -aoiE '(ANTIGRAVITY|GEMINI)_[A-Z_]+' | sort -u | head -80"], timeout=120)
print(o.strip() or "(none found)")

print("=" * 60, "\n2. MULTI-TURN -c SEMANTICS")
rc, to, o, e, dt = run(["agy", "-p", "Remember this codeword: BANANA42. Reply only OK."], timeout=90)
print(f"  setup:       exit={rc} dt={dt}s out={o.strip()[:60]!r}")
rc, to, o2, e2, dt = run(["agy", "-c", "-p", "What codeword did I just tell you? Reply with only the word."], timeout=90)
print(f"  -c continue: exit={rc} dt={dt}s out={o2.strip()[:80]!r}")
print(f"  -> -c CONTINUED prior conversation? {'BANANA42' in o2}")

print("=" * 60, "\n3. CONCURRENCY (3 simultaneous agy -p — shared-state collision?)")
results = {}
def worker(i):
    results[i] = run(["agy", "-p", f"Reply with only this number: {i}"], timeout=120)[:4]
ths = [threading.Thread(target=worker, args=(i,)) for i in (101, 202, 303)]
t0 = time.time()
for t in ths: t.start()
for t in ths: t.join()
for i in sorted(results):
    rc, to, o, e = results[i]
    print(f"  worker {i}: exit={rc} timed_out={to} out={o.strip()[:40]!r} err={e.strip()[:80]!r}")
print(f"  wall={round(time.time()-t0,1)}s")
print(f"  -> all answered with their own number? {all(str(i) in results[i][2] for i in results)}")

print("=" * 60, "\n4. ARG_MAX (~280KB single-arg prompt, > macOS 256KB)")
big = "A" * 280000
rc, to, o, e, dt = run(["agy", "-p", f"Reply only OK. padding:{big}"], timeout=60)
print(f"  exit={rc} timed_out={to} dt={dt}s out={o.strip()[:60]!r} err={e.strip()[:200]!r}")

print("=" * 60, "\n5. PROFILE ROBUSTNESS (richer task under our sandbox-exec)")
ws = tempfile.mkdtemp(prefix="agy_pr_")
open(os.path.join(ws, "a.txt"), "w").write("alpha\n")
open(os.path.join(ws, "b.txt"), "w").write("bravo\n")
profile = textwrap.dedent(f'''
(version 1)
(allow default)
(deny file-write*)
(allow file-write* (subpath "{ws}"))
(allow file-write* (subpath "{HOME}/.gemini"))
(allow file-write* (subpath "{HOME}/.antigravitycli"))
(allow file-write* (subpath "{TMP}"))
(allow file-write* (subpath "/private/var/folders"))
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (literal "/dev/null") (literal "/dev/stdout") (literal "/dev/stderr") (literal "/dev/dtracehelper") (literal "/dev/tty"))
''').strip()
prof = os.path.join(ws, "p.sb"); open(prof, "w").write(profile)
task = ("In the current directory read a.txt and b.txt, then run the shell commands `ls -la` and `git status` "
        "(it is fine if git fails), then create out.txt containing the concatenation of a.txt and b.txt, then reply DONE.")
rc, to, o, e, dt = run(["sandbox-exec", "-f", prof, "agy", "-p", task], cwd=ws, timeout=200)
print(f"  exit={rc} timed_out={to} dt={dt}s")
print("  stdout:", o.strip()[:600])
print("  stderr:", e.strip()[:400])
print(f"  out.txt created in workspace? {os.path.exists(os.path.join(ws,'out.txt'))}")
blob = (o + e).lower()
print(f"  hit a denied path it NEEDED? (look for 'not permitted'/'denied'): {'not permitted' in blob or 'operation not permitted' in blob or 'denied' in blob}")
shutil.rmtree(ws, ignore_errors=True)
print("=" * 60, "\nBATCH DONE")
