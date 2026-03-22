#!/usr/bin/env python3
"""
session-save-ctl.py — Shared State Mutation CLI for Session Notes v3.1

Every state mutation for session-state.json goes through this CLI.
The hot path (bash token gate) reads state directly via jq.
This CLI handles: lock, reserve, complete, fail, precompact-start,
precompact-complete, unlock, marker, status, recover, migrate-v2,
and family/segment management.

Usage:
    session-save-ctl.py lock --transcript-key <key> --transcript-path <path>
    session-save-ctl.py reserve --start-byte N --end-byte N --pid <pid>
    session-save-ctl.py complete --save-id <id> --end-byte N
    session-save-ctl.py fail --save-id <id> --code <code> --message <msg>
    session-save-ctl.py phase --phase extract|summarize|append
    session-save-ctl.py heartbeat
    session-save-ctl.py precompact-start --transcript-key <key>
    session-save-ctl.py precompact-complete --end-byte N
    session-save-ctl.py unlock
    session-save-ctl.py marker --type <type> --data-json <json>
    session-save-ctl.py status
    session-save-ctl.py recover
    session-save-ctl.py migrate-v2
    session-save-ctl.py transcript-key --path <file>
    session-save-ctl.py init-family --transcript-uuid <uuid> --segment-path <path>
    session-save-ctl.py rotate-segment --transcript-uuid <uuid> --new-path <path> --end-byte N
    session-save-ctl.py update-segment --transcript-uuid <uuid> [--save-count-incr N] [--size-bytes N]
"""

import argparse
import fcntl
import glob as glob_mod
import hashlib
import json
import os
import random
import shutil
import signal
import sys
import time
from datetime import datetime, timedelta
from pathlib import Path
from typing import Optional

try:
    from zoneinfo import ZoneInfo
    EASTERN = ZoneInfo('America/New_York')
except ImportError:
    EASTERN = None


# ─── Constants ────────────────────────────────────────────────────────────────

VERSION = '3.1'
STATE_FILE = Path.home() / '.triumvirate' / 'session-state.json'
LOCK_PREFIX = '/tmp/claude-session-save-'
LOG_LOCK_PREFIX = '/tmp/claude-session-log-'

HEARTBEAT_STALE_SECS = 30
DEFAULT_COOLDOWN_SECS = 300
DEFAULT_COOLDOWN_BYTES = 131072  # 128KB
SEGMENT_SIZE_LIMIT = 200 * 1024  # 200KB
STATUS_LOG_DIR = Path.home() / '.triumvirate'
STATUS_LOG_RETENTION_DAYS = 14


# ─── Utilities ────────────────────────────────────────────────────────────────

def _now():
    """Current time in Eastern."""
    if EASTERN:
        return datetime.now(EASTERN)
    return datetime.now()


def _now_iso():
    """Current time as ISO string."""
    return _now().isoformat()


def _hash_key(transcript_key: str) -> str:
    """Hash transcript key to 16-char hex for lock dir name."""
    return hashlib.sha256(transcript_key.encode()).hexdigest()[:16]


def _lock_dir(key_hash: str) -> str:
    """Lock directory path for a given key hash."""
    return f"{LOCK_PREFIX}{key_hash}.lock"


def make_transcript_key(path: str) -> str:
    """Generate transcript identity key: dev:ino:birthtime (macOS) or dev:ino:ctime_ns:size (Linux)."""
    st = os.stat(path)
    birth = getattr(st, 'st_birthtime', None)
    if birth is not None:
        return f"{st.st_dev}:{st.st_ino}:{birth}"
    return f"{st.st_dev}:{st.st_ino}:{st.st_ctime_ns}:{st.st_size}"


def is_pid_alive(pid: int) -> bool:
    """Check if a process is alive via kill(0)."""
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
        return True
    except (OSError, ProcessLookupError):
        return False


def _ok(data: dict = None):
    """Print success JSON and exit 0."""
    out = {"ok": True}
    if data:
        out.update(data)
    print(json.dumps(out))
    sys.exit(0)


def _err(message: str, exit_code: int = 1):
    """Print error JSON and exit with code."""
    print(json.dumps({"ok": False, "error": message}))
    sys.exit(exit_code)


# ─── Status Logging ──────────────────────────────────────────────────────────

def status_log(message: str):
    """Append to daily-rotated status log: ~/.triumvirate/stenographer-status-YYYYMMDD.log"""
    date_str = _now().strftime('%Y%m%d')
    log_file = STATUS_LOG_DIR / f'stenographer-status-{date_str}.log'
    log_file.parent.mkdir(parents=True, exist_ok=True)
    ts = _now().strftime('%H:%M:%S')
    try:
        with open(log_file, 'a') as f:
            f.write(f'[{ts}] {message}\n')
    except IOError:
        pass
    # Probabilistic cleanup: 1-in-50 runs
    if random.randint(1, 50) == 1:
        _prune_status_logs()


def _prune_status_logs():
    """Remove status logs older than retention period."""
    cutoff = time.time() - (STATUS_LOG_RETENTION_DAYS * 86400)
    for f in STATUS_LOG_DIR.glob('stenographer-status-*.log'):
        try:
            if f.stat().st_mtime < cutoff:
                f.unlink()
        except OSError:
            pass


# ─── Lock Management ─────────────────────────────────────────────────────────

def _read_lock_pid(lock_dir: str) -> Optional[int]:
    """Read PID from lock directory."""
    try:
        with open(os.path.join(lock_dir, 'pid')) as f:
            return int(f.read().strip())
    except (FileNotFoundError, ValueError):
        return None


def _is_lock_stale(lock_dir: str) -> bool:
    """Lock is stale if PID is dead OR heartbeat > 30s old.

    TOCTOU safety: if the lock dir exists but has no pid file yet,
    check the dir creation time. If created < 2s ago, another process
    is still writing metadata — treat as NOT stale to avoid clobbering.
    """
    pid = _read_lock_pid(lock_dir)
    if pid is None:
        # No pid file — check if lock dir was just created
        try:
            dir_age = time.time() - os.path.getctime(lock_dir)
            if dir_age < 2.0:
                return False  # in-flight, not stale
        except OSError:
            pass
        return True
    if not is_pid_alive(pid):
        return True
    heartbeat = os.path.join(lock_dir, 'started_at')
    try:
        if time.time() - os.path.getmtime(heartbeat) > HEARTBEAT_STALE_SECS:
            return True
    except FileNotFoundError:
        return True
    return False


def acquire_lock(transcript_key: str, owner: str = 'token_gate',
                 pid: int = None, timeout: float = 5.0) -> str:
    """
    Acquire mkdir lock. Returns lock dir path.
    Raises TimeoutError if held by live process.
    """
    key_hash = _hash_key(transcript_key)
    ld = _lock_dir(key_hash)
    lock_pid = pid or os.getpid()
    deadline = time.time() + timeout

    while True:
        try:
            os.mkdir(ld)
            # Write lock metadata
            with open(os.path.join(ld, 'pid'), 'w') as f:
                f.write(str(lock_pid))
            with open(os.path.join(ld, 'owner'), 'w') as f:
                f.write(owner)
            with open(os.path.join(ld, 'started_at'), 'w') as f:
                f.write(_now_iso())
            return ld
        except FileExistsError:
            if _is_lock_stale(ld):
                shutil.rmtree(ld, ignore_errors=True)
                continue
            if time.time() > deadline:
                holder_pid = _read_lock_pid(ld)
                raise TimeoutError(
                    f"Lock held by live process (PID {holder_pid})")
            time.sleep(0.1)


def release_lock(transcript_key: str):
    """Release mkdir lock by removing the directory."""
    key_hash = _hash_key(transcript_key)
    ld = _lock_dir(key_hash)
    if os.path.isdir(ld):
        shutil.rmtree(ld, ignore_errors=True)


def verify_lock_held(transcript_key: str):
    """Verify lock directory exists. Raises RuntimeError if not."""
    key_hash = _hash_key(transcript_key)
    ld = _lock_dir(key_hash)
    if not os.path.isdir(ld):
        raise RuntimeError("Lock not held — call 'lock' first")


def update_lock_pid(transcript_key: str, new_pid: int):
    """Update PID in lock dir (used after spawning worker)."""
    key_hash = _hash_key(transcript_key)
    ld = _lock_dir(key_hash)
    pid_file = os.path.join(ld, 'pid')
    with open(pid_file, 'w') as f:
        f.write(str(new_pid))


# ─── State I/O ────────────────────────────────────────────────────────────────

def _clear_running() -> dict:
    """Return a cleared running state dict."""
    return {
        "save_id": None, "owner": None, "pid": None,
        "started_at": None, "heartbeat_at": None,
        "lease_expires_at": None,
        "reserved_start_byte": None, "reserved_end_byte": None,
        "phase": None, "transcript_path": None, "transcript_key": None
    }


def default_state() -> dict:
    """Return a fresh v3.1 state structure."""
    return {
        "version": VERSION,
        "save_seq": 0,
        "transcript": {
            "path": None,
            "key": None,
            "generation": 0,
            "current_bytes": 0
        },
        "cursor": {
            "last_completed_end_byte": 0,
            "last_completed_at": None,
            "last_completed_save_id": None
        },
        "running": _clear_running(),
        "cooldown": {
            "until_time": None,
            "until_byte": 0,
            "min_new_bytes": DEFAULT_COOLDOWN_BYTES
        },
        "precompact": {
            "active": False,
            "owner_save_id": None,
            "last_completed_at": None,
            "last_completed_end_byte": 0
        },
        "failures": {
            "consecutive": 0,
            "last_code": None,
            "last_at": None,
            "last_message": None,
            "silence_breached": False
        },
        "families": {}
    }


def load_state() -> dict:
    """Load state from JSON file. Falls back to .bak, then default."""
    if not STATE_FILE.exists():
        return default_state()
    try:
        with open(STATE_FILE) as f:
            state = json.load(f)
        if state.get('version') != VERSION:
            return default_state()
        return state
    except (json.JSONDecodeError, IOError):
        # Try backup
        bak = Path(str(STATE_FILE) + '.bak')
        if bak.exists():
            try:
                with open(bak) as f:
                    state = json.load(f)
                if state.get('version') == VERSION:
                    status_log("STATE RECOVERED from .bak")
                    return state
            except (json.JSONDecodeError, IOError):
                pass
        status_log("STATE CORRUPTED — reset to default")
        return default_state()


def save_state(state: dict):
    """Atomic write: backup existing → write .tmp → rename."""
    STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
    # Backup current
    if STATE_FILE.exists():
        bak = Path(str(STATE_FILE) + '.bak')
        try:
            shutil.copy2(str(STATE_FILE), str(bak))
        except IOError:
            pass
    # Atomic write
    tmp = str(STATE_FILE) + '.tmp'
    with open(tmp, 'w') as f:
        json.dump(state, f, indent=2)
    os.rename(tmp, str(STATE_FILE))


# ─── Commands ─────────────────────────────────────────────────────────────────

def cmd_lock(args):
    """Acquire mkdir lock, update transcript info in state."""
    pid = args.pid
    try:
        ld = acquire_lock(
            args.transcript_key,
            owner=args.owner or 'token_gate',
            pid=pid
        )
    except TimeoutError as e:
        _err(str(e))

    # Update transcript in state
    state = load_state()
    old_key = state['transcript'].get('key')

    if old_key != args.transcript_key:
        # Transcript changed — reset cursor and byte cooldown
        state['transcript']['generation'] = \
            state['transcript'].get('generation', 0) + 1
        state['cursor']['last_completed_end_byte'] = 0
        state['cooldown']['until_byte'] = 0
        if old_key is not None:
            status_log(
                f"TRANSCRIPT CHANGE — gen "
                f"{state['transcript']['generation']}, cursor reset")

    state['transcript']['key'] = args.transcript_key
    if args.transcript_path:
        state['transcript']['path'] = args.transcript_path

    save_state(state)
    _ok({"lock_dir": ld, "generation": state['transcript']['generation']})


def cmd_reserve(args):
    """Write running state. Must hold lock."""
    state = load_state()
    tk = state['transcript'].get('key')
    if tk:
        try:
            verify_lock_held(tk)
        except RuntimeError as e:
            _err(str(e))

    state['save_seq'] = state.get('save_seq', 0) + 1
    save_id = args.save_id or f"tg-{state['save_seq']:04d}"

    state['running'] = {
        "save_id": save_id,
        "owner": "token_gate",
        "pid": args.pid,
        "started_at": _now_iso(),
        "heartbeat_at": _now_iso(),
        "lease_expires_at": (
            _now() + timedelta(seconds=150)).isoformat(),
        "reserved_start_byte": args.start_byte,
        "reserved_end_byte": args.end_byte,
        "phase": "extract",
        "transcript_path": state['transcript'].get('path'),
        "transcript_key": state['transcript'].get('key')
    }

    save_state(state)
    status_log(
        f"SAVE #{state['save_seq']} STARTED — {save_id}, "
        f"bytes {args.start_byte}-{args.end_byte}")
    _ok({"save_id": save_id, "save_seq": state['save_seq']})


def cmd_complete(args):
    """Advance cursor, set cooldown, clear running. Must hold lock."""
    state = load_state()
    tk = state['transcript'].get('key')
    if tk:
        try:
            verify_lock_held(tk)
        except RuntimeError as e:
            _err(str(e))

    if state['running'].get('save_id') != args.save_id:
        _err(
            f"save_id mismatch: state has "
            f"{state['running'].get('save_id')}, got {args.save_id}")

    # Advance cursor
    state['cursor']['last_completed_end_byte'] = args.end_byte
    state['cursor']['last_completed_at'] = _now_iso()
    state['cursor']['last_completed_save_id'] = args.save_id

    # Set cooldown (time + byte)
    state['cooldown']['until_time'] = (
        _now() + timedelta(seconds=DEFAULT_COOLDOWN_SECS)).isoformat()
    state['cooldown']['until_byte'] = \
        args.end_byte + DEFAULT_COOLDOWN_BYTES

    # Clear running
    state['running'] = _clear_running()

    # Reset failure counter on success
    state['failures']['consecutive'] = 0

    save_state(state)
    status_log(
        f"SAVE #{state['save_seq']} COMPLETE — "
        f"{args.save_id}, end_byte {args.end_byte}")
    _ok()


def cmd_fail(args):
    """Backoff cooldown, clear running, increment failures. Must hold lock."""
    state = load_state()
    tk = state['transcript'].get('key')
    if tk:
        try:
            verify_lock_held(tk)
        except RuntimeError as e:
            _err(str(e))

    # Validate save_id matches running state
    if state['running'].get('save_id') != args.save_id:
        _err(
            f"save_id mismatch: state has "
            f"{state['running'].get('save_id')}, got {args.save_id}")

    # Exponential backoff: base * 2^(n-1), capped at 600s
    consecutive = state['failures']['consecutive'] + 1
    backoff_secs = min(
        DEFAULT_COOLDOWN_SECS * (2 ** min(consecutive - 1, 4)), 600)

    state['cooldown']['until_time'] = (
        _now() + timedelta(seconds=backoff_secs)).isoformat()

    state['failures']['consecutive'] = consecutive
    state['failures']['last_code'] = args.code
    state['failures']['last_at'] = _now_iso()
    state['failures']['last_message'] = args.message

    state['running'] = _clear_running()

    save_state(state)
    status_log(
        f"SAVE #{state['save_seq']} FAILED — "
        f"{args.code}: {args.message}, backoff {backoff_secs}s")
    _ok({"consecutive_failures": consecutive, "backoff_secs": backoff_secs})


def cmd_phase(args):
    """Update the running phase. Worker calls this to track progress."""
    state = load_state()
    if state['running'].get('save_id') is None:
        _err("no running save")

    state['running']['phase'] = args.phase
    state['running']['heartbeat_at'] = _now_iso()
    save_state(state)
    _ok()


def cmd_heartbeat(args):
    """Update heartbeat in state and touch lock file mtime."""
    state = load_state()
    if state['running'].get('save_id') is None:
        _err("no running save")

    state['running']['heartbeat_at'] = _now_iso()
    save_state(state)

    # Touch lock heartbeat file
    tk = state['transcript'].get('key')
    if tk:
        key_hash = _hash_key(tk)
        hb_file = os.path.join(_lock_dir(key_hash), 'started_at')
        try:
            Path(hb_file).touch()
        except OSError:
            pass

    _ok()


def _supersede_worker(state: dict) -> list:
    """
    Kill running worker for precompact supersession.
    Returns list of status messages.
    """
    messages = []
    pid = state['running'].get('pid')
    phase = state['running'].get('phase')
    save_id = state['running'].get('save_id')

    if not pid or not save_id:
        return messages

    if not is_pid_alive(pid):
        messages.append(
            f"Worker PID {pid} already dead (save {save_id})")
        state['running'] = _clear_running()
        return messages

    if phase == 'append':
        # Wait up to 5s for append to finish
        waited = 0.0
        while waited < 5.0:
            time.sleep(0.5)
            waited += 0.5
            state_fresh = load_state()
            if state_fresh['running'].get('save_id') is None:
                messages.append(
                    f"Worker {save_id} completed append during wait")
                state['running'] = _clear_running()
                return messages

    # Kill the worker process group
    try:
        os.killpg(os.getpgid(pid), signal.SIGTERM)
        time.sleep(1)
        if is_pid_alive(pid):
            os.killpg(os.getpgid(pid), signal.SIGKILL)
            time.sleep(0.5)
    except (ProcessLookupError, PermissionError, OSError):
        pass

    messages.append(
        f"Superseded worker PID {pid} (save {save_id}, phase: {phase})")
    state['running'] = _clear_running()
    return messages


def cmd_precompact_start(args):
    """Acquire lock, set precompact.active, handle worker supersession."""
    messages = []

    # Try to acquire lock
    try:
        acquire_lock(
            args.transcript_key, owner='precompact', timeout=2.0)
    except TimeoutError:
        # Lock held — check if we can supersede
        key_hash = _hash_key(args.transcript_key)
        ld = _lock_dir(key_hash)
        state = load_state()

        # Only force-steal if: worker exists in state, or lock is stale
        can_steal = False
        if state['running'].get('save_id') and \
                state['running'].get('pid'):
            msgs = _supersede_worker(state)
            messages.extend(msgs)
            save_state(state)
            can_steal = True
        elif os.path.isdir(ld) and _is_lock_stale(ld):
            messages.append("Lock stale, reclaiming for precompact")
            can_steal = True

        if not can_steal:
            _err(
                "Lock held by live process with no running worker "
                "to supersede")

        # Safe to force-acquire after confirmed supersession/stale
        shutil.rmtree(ld, ignore_errors=True)
        try:
            os.mkdir(ld)
            with open(os.path.join(ld, 'pid'), 'w') as f:
                f.write(str(os.getpid()))
            with open(os.path.join(ld, 'owner'), 'w') as f:
                f.write('precompact')
            with open(os.path.join(ld, 'started_at'), 'w') as f:
                f.write(_now_iso())
        except OSError as e:
            _err(f"Failed to acquire lock after supersede: {e}")

    state = load_state()

    # Clear any remaining running state
    if state['running'].get('save_id'):
        state['running'] = _clear_running()

    # Increment save sequence for precompact
    state['save_seq'] = state.get('save_seq', 0) + 1
    pc_save_id = f"pc-{state['save_seq']:04d}"

    state['precompact']['active'] = True
    state['precompact']['owner_save_id'] = pc_save_id

    # Handle transcript key change
    old_key = state['transcript'].get('key')
    if old_key != args.transcript_key:
        state['transcript']['generation'] = \
            state['transcript'].get('generation', 0) + 1
        state['cursor']['last_completed_end_byte'] = 0
        state['cooldown']['until_byte'] = 0
        messages.append(
            f"Transcript key changed, gen "
            f"{state['transcript']['generation']}")
    state['transcript']['key'] = args.transcript_key

    if args.transcript_path:
        state['transcript']['path'] = args.transcript_path

    save_state(state)
    for m in messages:
        status_log(f"PRECOMPACT — {m}")
    status_log(f"PRECOMPACT {pc_save_id} STARTED")
    _ok({"save_id": pc_save_id, "save_seq": state['save_seq'],
         "messages": messages})


def cmd_precompact_complete(args):
    """Advance cursor, clear precompact. Requires active precompact."""
    state = load_state()

    if not state['precompact'].get('active'):
        _err("no active precompact — call precompact-start first")

    pc_save_id = state['precompact'].get('owner_save_id')
    if not pc_save_id:
        _err("precompact active but no owner_save_id")

    # Advance cursor
    state['cursor']['last_completed_end_byte'] = args.end_byte
    state['cursor']['last_completed_at'] = _now_iso()
    state['cursor']['last_completed_save_id'] = pc_save_id

    # Set cooldown
    state['cooldown']['until_time'] = (
        _now() + timedelta(seconds=DEFAULT_COOLDOWN_SECS)).isoformat()
    state['cooldown']['until_byte'] = \
        args.end_byte + DEFAULT_COOLDOWN_BYTES

    # Clear precompact
    state['precompact']['active'] = False
    state['precompact']['last_completed_at'] = _now_iso()
    state['precompact']['last_completed_end_byte'] = args.end_byte
    state['precompact']['owner_save_id'] = None

    # Reset failures on success
    state['failures']['consecutive'] = 0

    save_state(state)
    status_log(f"PRECOMPACT {pc_save_id} COMPLETE — end_byte {args.end_byte}")
    _ok()


def cmd_unlock(args):
    """Release mkdir lock."""
    # Prefer explicit key, fall back to state
    tk = getattr(args, 'transcript_key', None)
    if not tk:
        state = load_state()
        tk = state['transcript'].get('key')
    if tk:
        release_lock(tk)
    _ok()


def cmd_marker(args):
    """Write milestone marker to family JSONL + human-readable mirror."""
    state = load_state()

    transcript_path = state['transcript'].get('path')
    if not transcript_path or not os.path.exists(transcript_path):
        _err("no active transcript")

    # Current transcript byte position
    transcript_byte = os.path.getsize(transcript_path)

    # Find active family
    transcript_uuid = Path(transcript_path).stem
    family12 = transcript_uuid[:12]
    family = state['families'].get(transcript_uuid)

    if not family:
        _err(f"no family for transcript {transcript_uuid}")

    if not family.get('segments'):
        _err("no segments in family")

    # Markers file lives alongside segments
    seg_path = family['segments'][0]['path']
    markers_dir = os.path.dirname(seg_path)
    markers_file = os.path.join(markers_dir, f'{family12}_markers.jsonl')

    # Compute seq from existing line count
    seq = 0
    if os.path.exists(markers_file):
        try:
            with open(markers_file) as f:
                seq = sum(1 for _ in f)
        except IOError:
            pass
    seq += 1

    # Parse marker data
    try:
        data = json.loads(args.data_json) if args.data_json else {}
    except json.JSONDecodeError:
        data = {"raw": args.data_json}

    # Write JSONL marker (atomic append)
    marker = {
        "seq": seq,
        "ts": _now_iso(),
        "type": args.type,
        "transcript_byte": transcript_byte,
        "data": data
    }
    try:
        with open(markers_file, 'a') as f:
            f.write(json.dumps(marker) + '\n')
    except IOError as e:
        _err(f"failed to write marker: {e}")

    # Write human-readable bullet to current open segment
    current_seg_num = family.get('current_segment', 1)
    for seg in family['segments']:
        if seg['segment'] == current_seg_num and \
                seg.get('status') == 'open':
            ts_str = _now().strftime('%H:%M')
            bullet = _format_marker_bullet(ts_str, args.type, data)
            try:
                with open(seg['path'], 'a') as f:
                    f.write(bullet)
            except IOError:
                pass
            break

    status_log(
        f"MARKER #{seq} — {args.type} "
        f"{_marker_summary(args.type, data)}")
    _ok({"seq": seq, "transcript_byte": transcript_byte})


def _format_marker_bullet(ts: str, marker_type: str, data: dict) -> str:
    """Format a human-readable marker bullet for the session log."""
    if marker_type == 'git_commit':
        return (f"- [{ts}] git_commit {data.get('commit', '?')} "
                f"{data.get('message', '')}\n")
    elif marker_type in ('test_pass', 'test_fail'):
        return (f"- [{ts}] {marker_type} "
                f"{data.get('command', '')} "
                f"exit={data.get('exit_code', '?')}\n")
    elif marker_type == 'bash_fail':
        return (f"- [{ts}] bash_fail "
                f"{data.get('command_prefix', '')} "
                f"exit={data.get('exit_code', '?')}\n")
    elif marker_type == 'file_write_batch':
        files = data.get('files', [])
        return f"- [{ts}] file_write_batch ({len(files)} files)\n"
    elif marker_type == 'subagent_result':
        return (f"- [{ts}] subagent_result "
                f"count={data.get('count', '?')}\n")
    else:
        return f"- [{ts}] {marker_type}\n"


def _marker_summary(marker_type: str, data: dict) -> str:
    """One-line summary for status log."""
    if marker_type == 'git_commit':
        return f"{data.get('commit', '?')} {data.get('message', '')}"
    elif marker_type in ('test_pass', 'test_fail', 'bash_fail'):
        return f"{data.get('command', data.get('command_prefix', ''))}"
    elif marker_type == 'file_write_batch':
        return f"{len(data.get('files', []))} files"
    return ""


def cmd_status(args):
    """Print current effective state as JSON."""
    state = load_state()

    # Derive effective state
    now = _now()
    effective = 'idle'

    if state['precompact'].get('active'):
        effective = 'precompact_active'
    elif state['running'].get('save_id'):
        # Check if worker is actually alive
        pid = state['running'].get('pid')
        if pid and not is_pid_alive(pid):
            effective = 'idle'  # orphaned
        else:
            effective = 'running'
    else:
        # Check cooldown
        cd_time_str = state['cooldown'].get('until_time')
        cd_byte = state['cooldown'].get('until_byte', 0)
        time_expired = True
        byte_expired = True

        if cd_time_str:
            try:
                cd_time = datetime.fromisoformat(cd_time_str)
                if cd_time.tzinfo and not now.tzinfo:
                    now = now.replace(tzinfo=cd_time.tzinfo)
                elif now.tzinfo and not cd_time.tzinfo:
                    cd_time = cd_time.replace(tzinfo=now.tzinfo)
                if now < cd_time:
                    time_expired = False
            except (ValueError, TypeError):
                pass

        # Check byte cooldown against current transcript size
        tp = state['transcript'].get('path')
        if tp and os.path.exists(tp) and cd_byte > 0:
            current = os.path.getsize(tp)
            if current < cd_byte:
                byte_expired = False

        if not time_expired or not byte_expired:
            effective = 'cooldown'

    state['_effective'] = effective
    state['_timestamp'] = _now_iso()
    print(json.dumps(state, indent=2))


def cmd_recover(args):
    """Detect orphaned running state, clean up stale locks."""
    state = load_state()
    changes = []

    # Check for orphaned running save (PID dead OR heartbeat stale)
    if state['running'].get('save_id'):
        pid = state['running'].get('pid')
        alive = pid and is_pid_alive(pid)

        # Also check heartbeat staleness even if PID is alive
        heartbeat_stale = False
        hb_at = state['running'].get('heartbeat_at')
        if hb_at and alive:
            try:
                hb_time = datetime.fromisoformat(hb_at)
                now = _now()
                if hb_time.tzinfo and not now.tzinfo:
                    now = now.replace(tzinfo=hb_time.tzinfo)
                elif now.tzinfo and not hb_time.tzinfo:
                    hb_time = hb_time.replace(tzinfo=now.tzinfo)
                age_secs = (now - hb_time).total_seconds()
                if age_secs > HEARTBEAT_STALE_SECS * 2:  # 60s = generous
                    heartbeat_stale = True
            except (ValueError, TypeError):
                pass

        if not alive or heartbeat_stale:
            save_id = state['running']['save_id']
            reason = "PID dead" if not alive else \
                f"heartbeat stale ({int(age_secs)}s)"
            changes.append(
                f"Cleared orphaned save {save_id} "
                f"(PID {pid} {reason})")

            # Check transcript existence for recovery context
            tp = state['running'].get('transcript_path')
            tk = state['running'].get('transcript_key')
            if tp and os.path.exists(tp):
                try:
                    current_key = make_transcript_key(tp)
                    if current_key == tk:
                        changes.append(
                            "Transcript exists, key matches — "
                            "catch-up eligible")
                    else:
                        changes.append(
                            "Transcript exists but key changed — "
                            "new transcript")
                except OSError:
                    changes.append("Transcript stat failed")
            elif tp:
                changes.append(f"Transcript gone: {tp}")

            # Clear running (don't advance cursor — data may be lost)
            state['running'] = _clear_running()

    # Clean up any stale lock dirs
    for ld in glob_mod.glob(f"{LOCK_PREFIX}*.lock"):
        if os.path.isdir(ld) and _is_lock_stale(ld):
            shutil.rmtree(ld, ignore_errors=True)
            changes.append(f"Removed stale lock: {ld}")

    if changes:
        save_state(state)
        for c in changes:
            status_log(f"RECOVER — {c}")

    _ok({"changes": changes})


def cmd_migrate_v2(args):
    """Migrate v2 state files to v3.1 format."""
    if STATE_FILE.exists() and not getattr(args, 'force', False):
        existing = load_state()
        if existing.get('version') == VERSION and \
                existing.get('save_seq', 0) > 0:
            _err(
                f"v{VERSION} state already exists with "
                f"save_seq={existing['save_seq']}. "
                f"Use --force to overwrite.")

    tg_path = Path.home() / '.claude' / 'token-gate-state.json'
    sten_path = Path.home() / '.triumvirate' / 'stenographer-state.json'
    sl_path = Path.home() / '.triumvirate' / 'session-log-state.json'

    state = default_state()
    sources = []

    # Migrate token-gate state
    if tg_path.exists():
        try:
            with open(tg_path) as f:
                tg = json.load(f)
            state['save_seq'] = tg.get('saves_this_session', 0)
            state['cursor']['last_completed_end_byte'] = \
                tg.get('last_save_bytes', 0)
            lst = tg.get('last_save_time')
            if lst:
                state['cursor']['last_completed_at'] = \
                    datetime.fromtimestamp(lst).isoformat()
            sources.append('token-gate-state.json')
        except (json.JSONDecodeError, IOError) as e:
            status_log(f"MIGRATE-V2 WARNING — token-gate read failed: {e}")

    # Migrate stenographer state
    if sten_path.exists():
        try:
            with open(sten_path) as f:
                sten = json.load(f)
            sessions = sten.get('sessions', {})
            claude = sessions.get('claude', {})
            if claude.get('active_transcript'):
                tp = claude['active_transcript']
                state['transcript']['path'] = tp
                try:
                    state['transcript']['key'] = make_transcript_key(tp)
                    state['transcript']['current_bytes'] = \
                        os.path.getsize(tp)
                except OSError:
                    pass
            sources.append('stenographer-state.json')
        except (json.JSONDecodeError, IOError) as e:
            status_log(
                f"MIGRATE-V2 WARNING — stenographer read failed: {e}")

    # Migrate session-log state (into families)
    if sl_path.exists():
        try:
            with open(sl_path) as f:
                sl = json.load(f)
            for uuid, entry in sl.items():
                log_path = entry.get('log_path', '')
                if not log_path:
                    continue
                family12 = uuid[:12]
                exists = os.path.exists(log_path)
                try:
                    size = os.path.getsize(log_path) if exists else 0
                except OSError:
                    size = 0
                state['families'][uuid] = {
                    "family_id": family12,
                    "agent": entry.get('agent', 'claude'),
                    "repo": entry.get('repo', 'unknown'),
                    "feature": entry.get('feature', 'general'),
                    "current_segment": 1,
                    "segments": [{
                        "segment": 1,
                        "path": log_path,
                        "created_at": entry.get('created_at'),
                        "closed_at": None,
                        "start_byte": 0,
                        "end_byte": None,
                        "save_count": 0,
                        "size_bytes": size,
                        "status": "open" if exists else "orphaned"
                    }]
                }
            sources.append('session-log-state.json')
        except (json.JSONDecodeError, IOError) as e:
            status_log(
                f"MIGRATE-V2 WARNING — session-log read failed: {e}")

    save_state(state)
    status_log(
        f"MIGRATE-V2 — migrated to v{VERSION} from [{', '.join(sources)}], "
        f"seq={state['save_seq']}, families={len(state['families'])}")
    _ok({
        "save_seq": state['save_seq'],
        "families": len(state['families']),
        "sources": sources
    })


def cmd_update_lock_pid(args):
    """Update PID in lock dir — used after spawning worker, before disown."""
    state = load_state()
    tk = state['transcript'].get('key')
    if not tk:
        _err("no transcript key in state")
    try:
        update_lock_pid(tk, args.new_pid)
    except (FileNotFoundError, OSError) as e:
        _err(f"failed to update lock pid: {e}")
    _ok({"new_pid": args.new_pid})


def cmd_transcript_key(args):
    """Generate transcript key from file path."""
    if not os.path.exists(args.path):
        _err(f"file not found: {args.path}")
    key = make_transcript_key(args.path)
    _ok({"key": key, "path": args.path})


def cmd_init_family(args):
    """Create a family entry in state for a new transcript."""
    state = load_state()

    uuid = args.transcript_uuid
    if uuid in state['families']:
        _err(f"family already exists for {uuid}")

    family12 = uuid[:12]
    state['families'][uuid] = {
        "family_id": family12,
        "agent": args.agent or 'claude',
        "repo": args.repo or 'unknown',
        "feature": args.feature or 'general',
        "current_segment": 1,
        "segments": [{
            "segment": 1,
            "path": args.segment_path,
            "created_at": _now_iso(),
            "closed_at": None,
            "start_byte": 0,
            "end_byte": None,
            "save_count": 0,
            "size_bytes": 0,
            "status": "open"
        }]
    }

    save_state(state)
    status_log(
        f"INIT-FAMILY {family12} — {args.repo}/{args.feature}")
    _ok({"family_id": family12})


def cmd_rotate_segment(args):
    """Close current segment and open a new one."""
    state = load_state()
    uuid = args.transcript_uuid
    family = state['families'].get(uuid)
    if not family:
        _err(f"no family for {uuid}")

    # Close current segment
    current_num = family['current_segment']
    for seg in family['segments']:
        if seg['segment'] == current_num:
            seg['status'] = 'closed'
            seg['closed_at'] = _now_iso()
            seg['end_byte'] = args.end_byte
            try:
                seg['size_bytes'] = os.path.getsize(seg['path'])
            except OSError:
                pass
            break

    # Open new segment
    new_num = current_num + 1
    family['current_segment'] = new_num
    family['segments'].append({
        "segment": new_num,
        "path": args.new_path,
        "created_at": _now_iso(),
        "closed_at": None,
        "start_byte": args.end_byte + 1,
        "end_byte": None,
        "save_count": 0,
        "size_bytes": 0,
        "status": "open"
    })

    save_state(state)
    status_log(
        f"ROTATE — {family['family_id']} s{current_num:02d} → "
        f"s{new_num:02d}")
    _ok({"new_segment": new_num, "new_path": args.new_path})


def cmd_update_segment(args):
    """Update current segment stats (save_count, size_bytes)."""
    state = load_state()
    uuid = args.transcript_uuid
    family = state['families'].get(uuid)
    if not family:
        _err(f"no family for {uuid}")

    current_num = family['current_segment']
    for seg in family['segments']:
        if seg['segment'] == current_num:
            if args.save_count_incr:
                seg['save_count'] = \
                    seg.get('save_count', 0) + args.save_count_incr
            if args.size_bytes is not None:
                seg['size_bytes'] = args.size_bytes
            elif seg.get('path') and os.path.exists(seg['path']):
                seg['size_bytes'] = os.path.getsize(seg['path'])
            if args.end_byte is not None:
                seg['end_byte'] = args.end_byte
            break

    save_state(state)
    _ok()


# ─── CLI ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description='Session Save Control — v3.1',
        formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)

    # lock
    p = sub.add_parser('lock', help='Acquire mkdir lock')
    p.add_argument('--transcript-key', required=True,
                   help='Transcript identity key (dev:ino:birth)')
    p.add_argument('--transcript-path',
                   help='Absolute path to transcript file')
    p.add_argument('--owner', default='token_gate',
                   choices=['token_gate', 'precompact'])
    p.add_argument('--pid', type=int, required=True,
                   help='PID to write in lock (caller must pass $$)')

    # reserve
    p = sub.add_parser('reserve', help='Write running state')
    p.add_argument('--start-byte', type=int, required=True)
    p.add_argument('--end-byte', type=int, required=True)
    p.add_argument('--save-id',
                   help='Save ID (auto-generated if omitted)')
    p.add_argument('--pid', type=int, required=True,
                   help='Worker PID')

    # complete
    p = sub.add_parser('complete', help='Advance cursor, clear running')
    p.add_argument('--save-id', required=True)
    p.add_argument('--end-byte', type=int, required=True)

    # fail
    p = sub.add_parser('fail', help='Record failure, backoff cooldown')
    p.add_argument('--save-id', required=True)
    p.add_argument('--code', required=True,
                   help='Failure code (timeout, invalid, rate_limit)')
    p.add_argument('--message', required=True,
                   help='Human-readable failure message')

    # phase
    p = sub.add_parser('phase', help='Update running phase')
    p.add_argument('--phase', required=True,
                   choices=['extract', 'summarize', 'append'])

    # heartbeat
    sub.add_parser('heartbeat', help='Update heartbeat timestamp')

    # precompact-start
    p = sub.add_parser('precompact-start',
                       help='Start precompact (supersedes worker)')
    p.add_argument('--transcript-key', required=True)
    p.add_argument('--transcript-path',
                   help='Absolute path to transcript file')

    # precompact-complete
    p = sub.add_parser('precompact-complete',
                       help='Complete precompact')
    p.add_argument('--end-byte', type=int, required=True)

    # unlock
    p = sub.add_parser('unlock', help='Release lock')
    p.add_argument('--transcript-key',
                   help='Key to unlock (reads from state if omitted)')

    # marker
    p = sub.add_parser('marker', help='Write milestone marker')
    p.add_argument('--type', required=True,
                   help='Marker type (git_commit, test_pass, etc.)')
    p.add_argument('--data-json', default='{}',
                   help='JSON data for the marker')

    # status
    sub.add_parser('status', help='Print current state as JSON')

    # recover
    sub.add_parser('recover',
                   help='Clean up orphaned state and stale locks')

    # migrate-v2
    p = sub.add_parser('migrate-v2',
                       help='Migrate from v2 state files')
    p.add_argument('--force', action='store_true',
                   help='Overwrite existing v3.1 state')

    # update-lock-pid
    p = sub.add_parser('update-lock-pid',
                       help='Update PID in lock dir (worker handoff)')
    p.add_argument('--new-pid', type=int, required=True,
                   help='New PID to write (worker PID)')

    # transcript-key
    p = sub.add_parser('transcript-key',
                       help='Generate transcript key from file path')
    p.add_argument('--path', required=True,
                   help='Path to transcript file')

    # init-family
    p = sub.add_parser('init-family',
                       help='Create family entry in state')
    p.add_argument('--transcript-uuid', required=True)
    p.add_argument('--segment-path', required=True,
                   help='Path to first segment file')
    p.add_argument('--agent', default='claude')
    p.add_argument('--repo', default='unknown')
    p.add_argument('--feature', default='general')

    # rotate-segment
    p = sub.add_parser('rotate-segment',
                       help='Close current segment, open new one')
    p.add_argument('--transcript-uuid', required=True)
    p.add_argument('--new-path', required=True,
                   help='Path to new segment file')
    p.add_argument('--end-byte', type=int, required=True,
                   help='Transcript byte at rotation point')

    # update-segment
    p = sub.add_parser('update-segment',
                       help='Update current segment stats')
    p.add_argument('--transcript-uuid', required=True)
    p.add_argument('--save-count-incr', type=int,
                   help='Increment save count by N')
    p.add_argument('--size-bytes', type=int,
                   help='Set segment size (auto-detected if omitted)')
    p.add_argument('--end-byte', type=int,
                   help='Set segment end byte')

    args = parser.parse_args()

    # Acquire state file flock for mutating commands.
    # Process-lifetime: held until exit, covers entire R-M-W cycle.
    # Read-only commands (status, transcript-key) use shared lock.
    # NOTE: Concurrent multi-session is a known limitation of the
    # singleton state file design. This flock prevents R-M-W races
    # within a single session's concurrent hooks.
    _mutating = {
        'lock', 'reserve', 'complete', 'fail', 'phase',
        'heartbeat', 'precompact-start', 'precompact-complete',
        'marker', 'recover', 'migrate-v2', 'init-family',
        'rotate-segment', 'update-segment', 'update-lock-pid',
    }
    STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
    _flock_path = str(STATE_FILE) + '.flock'
    _flock_fd = open(_flock_path, 'w')
    if args.command in _mutating:
        fcntl.flock(_flock_fd, fcntl.LOCK_EX)
    else:
        fcntl.flock(_flock_fd, fcntl.LOCK_SH)
    # flock released automatically when process exits

    dispatch = {
        'lock': cmd_lock,
        'reserve': cmd_reserve,
        'complete': cmd_complete,
        'fail': cmd_fail,
        'phase': cmd_phase,
        'heartbeat': cmd_heartbeat,
        'precompact-start': cmd_precompact_start,
        'precompact-complete': cmd_precompact_complete,
        'unlock': cmd_unlock,
        'marker': cmd_marker,
        'status': cmd_status,
        'recover': cmd_recover,
        'migrate-v2': cmd_migrate_v2,
        'update-lock-pid': cmd_update_lock_pid,
        'transcript-key': cmd_transcript_key,
        'init-family': cmd_init_family,
        'rotate-segment': cmd_rotate_segment,
        'update-segment': cmd_update_segment,
    }

    try:
        dispatch[args.command](args)
    except Exception as e:
        _err(f"unexpected error: {e}")


if __name__ == '__main__':
    main()
