#!/usr/bin/env python3
"""Can we get token counts headlessly? Test: slash-cmd-as-prompt, ask-the-model, usage keys in log."""
import os, subprocess, signal, time, re

BASE = dict(os.environ)
USAGE = re.compile(r'(?i)(usageMetadata|promptTokenCount|candidatesTokenCount|totalTokenCount|thoughtsTokenCount|cachedContentTokenCount|quotaRemaining|input_tokens|output_tokens|tokenCount)')

def run(args, timeout=60):
    lf = f"/tmp/agy_p9_{int(time.time()*1000)}.log"
    p = subprocess.Popen(["agy"] + args + ["--log-file", lf], stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, start_new_session=True, env=BASE)
    try:
        o, e = p.communicate(timeout=timeout); rc = p.returncode; to = False
    except subprocess.TimeoutExpired:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL); o, e = p.communicate(); rc = None; to = True
    log = open(lf, errors="replace").read() if os.path.exists(lf) else ""
    try: os.remove(lf)
    except FileNotFoundError: pass
    return rc, to, o.decode(errors="replace"), log

for cmd in (["-p", "/context"], ["-p", "/usage"],
            ["-p", "How many tokens did this exact request use? Reply with just the number."]):
    rc, to, out, log = run(cmd)
    hits = sorted(set(m.group(0) for m in USAGE.finditer(out + log)))
    print(f"\n=== agy {' '.join(cmd)} ===")
    print(f"  exit={rc} to={to}")
    print(f"  STDOUT: {out.strip()[:220]!r}")
    print(f"  usage-key hits in stdout+log: {hits or 'NONE'}")

print("\n=== binary strings: log-verbosity & usage-metadata keys ===")
s = subprocess.run(["bash", "-c",
    "strings $(which agy) | grep -aoiE '(LOG_LEVEL|VERBOSE|DEBUG_|GLOG|usageMetadata|promptTokenCount|totalTokenCount|tokenCount)[A-Za-z_]*' | sort -u | head -40"],
    capture_output=True, text=True)
print(s.stdout or "(none)")
