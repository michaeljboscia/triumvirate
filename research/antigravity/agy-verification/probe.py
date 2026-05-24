#!/usr/bin/env python3
"""agy verification probe — Go-safe (SIGKILL process group), bounded timeouts.
REQ-062 (PTY vs non-TTY), REQ-064 (exit codes). PTY test runs first (decisive)."""
import os, pty, subprocess, select, signal, time, sys

PROMPT = "What is 2+2? Reply with only the digit."
os.environ["ANTIGRAVITY_SKIP_UPDATE_CHECK"] = "true"

def run_pty(cmd, timeout):
    mfd, sfd = pty.openpty()
    p = subprocess.Popen(cmd, stdin=sfd, stdout=sfd, stderr=sfd,
                         start_new_session=True, close_fds=True)
    os.close(sfd)
    out = b""; start = time.time(); timed_out = False
    while True:
        if time.time() - start > timeout:
            timed_out = True
            try: os.killpg(os.getpgid(p.pid), signal.SIGKILL)
            except ProcessLookupError: pass
            break
        r, _, _ = select.select([mfd], [], [], 1.0)
        if r:
            try: chunk = os.read(mfd, 4096)
            except OSError: break
            if not chunk: break
            out += chunk
        if p.poll() is not None:
            try:
                while True:
                    r, _, _ = select.select([mfd], [], [], 0.3)
                    if not r: break
                    chunk = os.read(mfd, 4096)
                    if not chunk: break
                    out += chunk
            except OSError: pass
            break
    try: p.wait(timeout=3)
    except Exception: pass
    os.close(mfd)
    return p.poll(), timed_out, out, round(time.time() - start, 1)

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
        return p.returncode, True, o, e, round(time.time() - start, 1)

AGY = ["agy", "-p", PROMPT]

print("=== REQ-062b: PTY (decisive) ===", flush=True)
rc, to, out, dt = run_pty(AGY, 90)
print(f"exit={rc} timed_out={to} elapsed={dt}s bytes={len(out)}")
print("output(repr):", repr(out[:600]))

print("\n=== REQ-062a: non-TTY pipe (expect hang/empty) ===", flush=True)
rc, to, o, e, dt = run_pipe(AGY, 35)
print(f"exit={rc} timed_out={to} elapsed={dt}s stdout_bytes={len(o)} stderr_bytes={len(e)}")
print("stdout(repr):", repr(o[:400]))
print("stderr(repr):", repr(e[:400]))

print("\n=== REQ-064: bad flag (no model call) ===", flush=True)
rc, to, o, e, dt = run_pipe(["agy", "--nonsense-flag-xyz"], 15)
print(f"exit={rc} timed_out={to} stdout={o[:200]!r} stderr={e[:200]!r}")
