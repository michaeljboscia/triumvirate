#!/usr/bin/env python3
"""Does ANTIGRAVITY_CONVERSATION_ID give caller-supplied, ISOLATED, resumable multi-turn?
If yes -> solves Issue #7 and reopens the single-turn decision."""
import os, subprocess, signal, time, uuid

BASE = dict(os.environ); BASE["ANTIGRAVITY_SKIP_UPDATE_CHECK"] = "true"

def run(prompt, conv_id, timeout=90):
    env = dict(BASE); env["ANTIGRAVITY_CONVERSATION_ID"] = conv_id
    start = time.time()
    p = subprocess.Popen(["agy", "-p", prompt], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         start_new_session=True, env=env)
    try:
        o, e = p.communicate(timeout=timeout); rc = p.returncode; to = False
    except subprocess.TimeoutExpired:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL); o, e = p.communicate(); rc = None; to = True
    return rc, to, o.decode(errors="replace").strip(), e.decode(errors="replace").strip(), round(time.time()-start, 1)

A = str(uuid.uuid4()); B = str(uuid.uuid4())
print(f"conv A = {A}\nconv B = {B}\n")

print("1. set codeword ALPHA1 in conv A")
print("  ", run("Remember this codeword: ALPHA1. Reply only OK.", A)[:4])
print("2. set codeword BETA2 in conv B")
print("  ", run("Remember this codeword: BETA2. Reply only OK.", B)[:4])
print("3. ask conv A for its codeword")
ra = run("What codeword did I tell you in this conversation? Reply with only the word.", A)
print("  ", ra[:4])
print("4. ask conv B for its codeword")
rb = run("What codeword did I tell you in this conversation? Reply with only the word.", B)
print("  ", rb[:4])

a_ok = "ALPHA1" in ra[2] and "BETA2" not in ra[2]
b_ok = "BETA2" in rb[2] and "ALPHA1" not in rb[2]
print(f"\nconv A resumed correctly & isolated: {a_ok}")
print(f"conv B resumed correctly & isolated: {b_ok}")
print(f"VERDICT: ANTIGRAVITY_CONVERSATION_ID gives isolated resumable multi-turn = {a_ok and b_ok}")
