#!/usr/bin/env python3
"""Hang characterization: 3 conditions x N calls, per-call SIGKILL watchdog.
A=plain, B=-c (resume most recent), C=fixed ANTIGRAVITY_CONVERSATION_ID (resume by id)."""
import os, subprocess, signal, time, uuid

BASE = dict(os.environ)
TIMEOUT, N = 30, 10

def run(args, conv_id=None, timeout=TIMEOUT):
    env = dict(BASE)
    if conv_id:
        env["ANTIGRAVITY_CONVERSATION_ID"] = conv_id
    start = time.time()
    p = subprocess.Popen(["agy"] + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         start_new_session=True, env=env)
    try:
        o, e = p.communicate(timeout=timeout); rc = p.returncode; to = False
    except subprocess.TimeoutExpired:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL); o, e = p.communicate(); rc = None; to = True
    return to, round(time.time() - start, 1), rc

def trial(label, mk):
    hangs, times = 0, []
    print(f"\n[{label}]", flush=True)
    for i in range(1, N + 1):
        to, dt, rc = mk(i)
        times.append(dt); hangs += 1 if to else 0
        print(f"  {i:2d}: {'HANG' if to else 'ok  '} {dt:5.1f}s rc={rc}", flush=True)
    med = sorted(times)[len(times) // 2]
    print(f"  => {label}: {hangs}/{N} hung; t min/med/max = {min(times)}/{med}/{max(times)}", flush=True)
    return hangs

print(f"Hang characterization: 3 conditions x {N} calls, {TIMEOUT}s SIGKILL watchdog", flush=True)
a = trial("A plain (no resume)", lambda i: run(["-p", f"Reply only with the number {i}."]))
b = trial("B -c resume-most-recent", lambda i: run(["-c", "-p", f"Reply only with the number {i}."]))
C = str(uuid.uuid4())
c = trial("C fixed CONVERSATION_ID resume", lambda i: run(["-p", f"Reply only with the number {i}."], conv_id=C))
print(f"\nSUMMARY hangs:  A(plain)={a}/{N}   B(-c)={b}/{N}   C(conv-id)={c}/{N}", flush=True)
