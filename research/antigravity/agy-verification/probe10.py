#!/usr/bin/env python3
"""1.0.2 re-verification of REQ-060-064 + REQ-100 (binary upgraded 1.0.1 -> 1.0.2).
Go-safe: SIGKILL the process group on timeout. Safe prompts only (no writes)."""
import os, subprocess, signal, time, tempfile

PROMPT = "What is 2+2? Reply with only the digit."

def run_pipe(cmd, timeout, env=None):
    start = time.time()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         start_new_session=True, env=env)
    try:
        o, e = p.communicate(timeout=timeout)
        return p.returncode, False, o, e, round(time.time() - start, 1)
    except subprocess.TimeoutExpired:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL)
        o, e = p.communicate()
        return p.returncode, True, o, e, round(time.time() - start, 1)

print("=== REQ-062: non-TTY pipe capture (expect '4', exit 0, NON-empty) ===", flush=True)
rc, to, o, e, dt = run_pipe(["agy", "-p", PROMPT], 60)
print(f"exit={rc} timed_out={to} elapsed={dt}s stdout_bytes={len(o)} stderr_bytes={len(e)}")
print("stdout(repr):", repr(o[:300]))
pipe_ok = (rc == 0 and not to and b"4" in o)
print(f"PIPE CAPTURE CLEAN: {pipe_ok}")

print("\n=== REQ-100: --log-file carries model + auth lines ===", flush=True)
logf = tempfile.NamedTemporaryFile(prefix="agy_log_", suffix=".txt", delete=False).name
rc, to, o, e, dt = run_pipe(["agy", "-p", PROMPT, "--log-file", logf], 60)
print(f"exit={rc} timed_out={to} elapsed={dt}s stdout(repr)={o[:120]!r}")
log = ""
try:
    log = open(logf, errors="replace").read()
except Exception as ex:
    print("log read error:", ex)
print(f"log bytes={len(log)}")
model_lines = [l for l in log.splitlines() if "Propagating selected model" in l]
auth_lines  = [l for l in log.splitlines() if "authMethod=" in l]
print("MODEL line:", model_lines[0].strip()[-160:] if model_lines else "<<NOT FOUND>>")
print("AUTH  line:", auth_lines[-1].strip()[-160:] if auth_lines else "<<NOT FOUND>>")
try: os.remove(logf)
except OSError: pass

print("\n=== REQ-064: exit codes ===", flush=True)
rc, to, o, e, dt = run_pipe(["agy", "--nonsense-flag-xyz"], 15)
print(f"bad-flag: exit={rc} (expect 2) stderr={e[:160]!r}")

print("\n=== REQ-063: token reuse (second back-to-back call, expect no interactive prompt) ===", flush=True)
rc, to, o, e, dt = run_pipe(["agy", "-p", "What is 3+3? Reply with only the digit."], 60)
print(f"exit={rc} timed_out={to} elapsed={dt}s stdout(repr)={o[:120]!r}")
reuse_ok = (rc == 0 and b"6" in o)
print(f"TOKEN REUSE (no prompt, answered): {reuse_ok}")
