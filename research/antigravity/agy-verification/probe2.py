#!/usr/bin/env python3
"""Characterize intermittent non-TTY hang: N piped agy -p calls, hard SIGKILL watchdog."""
import os, subprocess, signal, time

os.environ["ANTIGRAVITY_SKIP_UPDATE_CHECK"] = "true"
PROMPT = "What is 2+2? Reply with only the digit."
AGY = ["agy", "-p", PROMPT]
N, TIMEOUT = 6, 40

def run_pipe(cmd, timeout):
    start = time.time()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         start_new_session=True)
    try:
        o, e = p.communicate(timeout=timeout)
        return p.returncode, False, o, e, round(time.time() - start, 1)
    except subprocess.TimeoutExpired:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL)
        o, e = p.communicate()
        return None, True, o, e, round(time.time() - start, 1)

hangs = 0; oks = 0
for i in range(1, N + 1):
    rc, to, o, e, dt = run_pipe(AGY, TIMEOUT)
    status = "HANG(killed)" if to else ("OK" if (rc == 0 and o.strip()) else f"ODD rc={rc}")
    if to: hangs += 1
    elif rc == 0 and o.strip(): oks += 1
    print(f"#{i}: {status} elapsed={dt}s out={o.strip()[:40]!r} err={e.strip()[:80]!r}", flush=True)

print(f"\nSUMMARY: {oks}/{N} clean, {hangs}/{N} hung (killed at {TIMEOUT}s)")
