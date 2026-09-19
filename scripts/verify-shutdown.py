#!/usr/bin/env python3
"""Reproduce the D-001 regression: SIGTERM with a connection open mid-request.

Runs a THROWAWAY daemon on its own port with its own home, so the real daemon on 8080 is never
touched. Opens a connection that sends request headers promising a body it never sends, so the
server is left waiting on it (an in-flight connection graceful shutdown must drain). Then SIGTERM,
and time how long the process takes to actually exit, capped.

This is the regression test for a hang introduced by the D-001 fix itself (2026-09-19): adding a
graceful shutdown made axum wait for every open connection, one never drained, and the daemon
stopped listening without exiting. It went unnoticed all day because `start-daemon.sh` escalates
a SIGTERM that is ignored to SIGKILL, so every restart LOOKED clean. The one time that escalation
was bypassed, the daemon went down. A guard that silently finishes the job hides the defect it
finishes, so this test runs the binary directly and never escalates.

Usage: verify-shutdown.py [binary]   (default: the installed ~/.local/bin/triumvirate)
Exit non-zero if the process is still running at the cap.
"""
import os, signal, socket, subprocess, sys, tempfile, time

binary = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.local/bin/triumvirate")
label = "shutdown with a stuck connection"
PORT = 18099
CAP = 25.0  # seconds; past this we call it a hang and SIGKILL

home = tempfile.mkdtemp(prefix="shutdown-hang-")
env = {
    "PATH": os.environ["PATH"],
    "HOME": os.environ["HOME"],
    "TRIUMVIRATE_HOME": home,
    "TRIUMVIRATE_DAEMON_BIND_ADDR": f"127.0.0.1:{PORT}",
    "TRIUMVIRATE_SHUTDOWN_DRAIN_SECS": "3",
    "TRIUMVIRATE_GEMINI_BACKEND": "agy",
}
proc = subprocess.Popen([binary, "daemon"], env=env,
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

# Wait for it to listen.
deadline = time.time() + 20
while time.time() < deadline:
    try:
        socket.create_connection(("127.0.0.1", PORT), timeout=0.5).close()
        break
    except OSError:
        time.sleep(0.2)
else:
    proc.kill()
    sys.exit(f"{label}: daemon never listened on {PORT}")

# The stuck connection: headers promise 100000 bytes of body, one byte arrives.
stuck = socket.create_connection(("127.0.0.1", PORT))
stuck.sendall(b"POST /ask-agent HTTP/1.1\r\nHost: localhost\r\n"
              b"Content-Type: application/json\r\nContent-Length: 100000\r\n\r\n{")
time.sleep(1.0)  # let the server accept it and start waiting on the body

t0 = time.time()
proc.send_signal(signal.SIGTERM)
try:
    proc.wait(timeout=CAP)
    took = time.time() - t0
    verdict = f"EXITED after {took:.1f}s"
except subprocess.TimeoutExpired:
    verdict = f"HUNG: still running {CAP:.0f}s after SIGTERM (SIGKILLed)"
    proc.kill()
    proc.wait()
stuck.close()
print(f"{label}: {verdict}")
sys.exit(1 if verdict.startswith("HUNG") else 0)
