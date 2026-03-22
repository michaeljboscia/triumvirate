#!/usr/bin/env python3
"""
session-save-worker.py — Background Gemini summarizer for Session Notes v3.1

Spawned by the token gate when transcript growth exceeds threshold.
Reads state, extracts transcript delta, calls Gemini CLI for summarization,
and appends the result to the current session log segment.

Requirements:
- Must be spawned with start_new_session=True (own process group)
- Lifecycle lock must be held (PID updated in lock dir before disown)
- State must have running.save_id set via ctl reserve

Usage:
    session-save-worker.py --save-id tg-0017
"""

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time
import uuid as uuid_mod
from datetime import datetime
from pathlib import Path

try:
    from zoneinfo import ZoneInfo
    EASTERN = ZoneInfo('America/New_York')
except ImportError:
    EASTERN = None


# ─── Constants ────────────────────────────────────────────────────────────────

CTL = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                   'session-save-ctl.py')
STATE_FILE = Path.home() / '.triumvirate' / 'session-state.json'
STATUS_LOG_DIR = Path.home() / '.triumvirate'

GEMINI_TIMEOUT = 120
HEARTBEAT_INTERVAL = 5
MIN_SUMMARY_CHARS = 300
MAX_RAW_FALLBACK_CHARS = 5000
SEGMENT_SIZE_LIMIT = 200 * 1024  # 200KB

# Patterns indicating polluted/meta-recursive Gemini output
POLLUTION_PATTERNS = [
    '===BEGIN_SUMMARY',
    '===END_SUMMARY',
    '"tool_name"',
    '"tool_input"',
    'session-save-ctl.py',
    'session-save-worker.py',
    'additionalContext',
]

GEMINI_PROMPT_TEMPLATE = """You are an expert technical project manager and system logger.
Below is a raw JSONL transcript chunk from a developer's session using Claude Code.
Your job is to synthesize this into a concrete, dense technical log.

RULES:
1. NO FLUFF. Do not say "The team worked on...". Write like a Git commit history meets a technical design doc.
2. Focus strictly on file modifications, terminal commands executed, bugs encountered, and architectural decisions made.
3. Keep it dense. Use bullet points and code blocks.
4. Output ONLY valid Markdown. Do not include introductory text.

You MUST start your response with exactly {begin_marker} and end with exactly {end_marker}

## Required Output Schema:
### Checkpoint: [Extract approximate time from transcript or state "Mid-Session"]

**Files Modified / Created:**
- `path/to/file` - Brief reason for change.

**Commands Executed:**
- `command` - Outcome (Success/Error).

**Key Decisions & Progress:**
- [Concrete technical step taken]

**Blockers / Errors Addressed:**
- [Specific error message or stack trace fragment] - [How it was bypassed or fixed]

## STRUCTURED MILESTONE FACTS (treat as ground truth):
{milestone_markers}

## TRANSCRIPT CHUNK:
{transcript_delta}"""

SIMPLIFIED_PROMPT_TEMPLATE = """Summarize this Claude Code transcript chunk into a dense technical log.
Focus on: files changed, commands run, decisions made, errors hit.
Use bullet points and code blocks. Output ONLY valid Markdown.

You MUST start your response with exactly {begin_marker} and end with exactly {end_marker}

{transcript_delta}"""


# ─── Utilities ────────────────────────────────────────────────────────────────

def _now():
    if EASTERN:
        return datetime.now(EASTERN)
    return datetime.now()


def _now_iso():
    return _now().isoformat()


def status_log(message):
    """Append to daily-rotated status log."""
    date_str = _now().strftime('%Y%m%d')
    log_file = STATUS_LOG_DIR / f'stenographer-status-{date_str}.log'
    ts = _now().strftime('%H:%M:%S')
    try:
        with open(log_file, 'a') as f:
            f.write(f'[{ts}] {message}\n')
    except IOError:
        pass


# ─── Environment ──────────────────────────────────────────────────────────────

def source_env():
    """Source .env files for Gemini API keys and config."""
    for env_path in [
        Path.home() / '.claude' / '.env',
        Path.home() / '.triumvirate' / '.env',
    ]:
        if env_path.exists():
            try:
                with open(env_path) as f:
                    for line in f:
                        line = line.strip()
                        if line and not line.startswith('#') and '=' in line:
                            key, _, value = line.partition('=')
                            key = key.strip()
                            value = value.strip().strip("'\"")
                            if key and value:
                                os.environ.setdefault(key, value)
            except IOError:
                pass


# ─── CTL Interface ────────────────────────────────────────────────────────────

def ctl(*args, timeout=30, required=False):
    """
    Call session-save-ctl.py and return parsed JSON response.

    If required=True, raises RuntimeError on non-OK response.
    Use required=True for lifecycle mutations (complete, fail, rotate).
    """
    cmd = [sys.executable, CTL] + list(args)
    try:
        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout)
        if result.stdout.strip():
            try:
                resp = json.loads(result.stdout.strip())
            except json.JSONDecodeError:
                resp = {"ok": result.returncode == 0,
                        "raw": result.stdout.strip()}
        else:
            resp = {"ok": result.returncode == 0}

        if required and not resp.get('ok') and \
                'ok' in resp:  # only check if response uses ok protocol
            raise RuntimeError(
                f"ctl {args[0]} failed: {resp.get('error', resp)}")
        if required and result.returncode != 0:
            raise RuntimeError(
                f"ctl {args[0]} exit code {result.returncode}")
        return resp
    except subprocess.TimeoutExpired:
        if required:
            raise RuntimeError(f"ctl {args[0]} timed out after {timeout}s")
        return {"ok": False, "error": "ctl timeout"}


def update_heartbeat():
    """Update heartbeat in state and lock dir. Short timeout, non-fatal."""
    resp = ctl('heartbeat', timeout=5)
    if not resp.get('ok'):
        # Heartbeat failure is a warning — if persistent, lock will go stale
        # and precompact or recover will clean up
        status_log(f"HEARTBEAT WARNING: {resp.get('error', 'failed')}")


def set_phase(phase):
    """Update running phase in state."""
    ctl('phase', '--phase', phase, timeout=10)


# ─── State Access ─────────────────────────────────────────────────────────────

def load_state():
    """Read state file directly (worker holds lifecycle lock)."""
    try:
        with open(STATE_FILE) as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return None


# ─── Transcript Parsing ──────────────────────────────────────────────────────

def parse_delta(transcript_path, start_byte, end_byte):
    """
    Read transcript bytes in range [start_byte, end_byte).
    Trims to whole JSONL line boundaries to avoid handing Gemini
    truncated JSON fragments at the edges.
    """
    with open(transcript_path, 'rb') as f:
        f.seek(start_byte)
        raw = f.read(end_byte - start_byte)

    text = raw.decode('utf-8', errors='replace')

    # Trim partial first line (unless we're at byte 0)
    if start_byte > 0:
        first_nl = text.find('\n')
        if first_nl != -1 and first_nl < 500:  # reasonable partial line
            text = text[first_nl + 1:]

    # Trim partial last line
    last_nl = text.rfind('\n')
    if last_nl != -1 and last_nl > len(text) - 500:
        text = text[:last_nl + 1]

    return text


def load_markers(markers_file, start_byte, end_byte):
    """Load markers from JSONL where transcript_byte in [start, end)."""
    markers = []
    if not os.path.exists(markers_file):
        return markers
    try:
        with open(markers_file) as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    marker = json.loads(line)
                    tb = marker.get('transcript_byte', 0)
                    if start_byte <= tb < end_byte:
                        markers.append(marker)
                except json.JSONDecodeError:
                    continue
    except IOError:
        pass
    return markers


# ─── Gemini Interaction ──────────────────────────────────────────────────────

def build_prompt(delta_text, markers, begin_marker, end_marker,
                 simplified=False):
    """Build the full prompt for Gemini."""
    if simplified:
        return SIMPLIFIED_PROMPT_TEMPLATE.format(
            begin_marker=begin_marker,
            end_marker=end_marker,
            transcript_delta=delta_text
        )

    markers_json = json.dumps(markers, indent=2) if markers else "[]"
    return GEMINI_PROMPT_TEMPLATE.format(
        begin_marker=begin_marker,
        end_marker=end_marker,
        milestone_markers=markers_json,
        transcript_delta=delta_text
    )


def call_gemini(payload_path, begin_marker, end_marker):
    """
    Call Gemini CLI via Popen + poll loop.

    Uses:
    - start_new_session=True (own process group, safe kill)
    - --approval-mode plan (read-only, no tool execution)
    - Unique temp file (concurrent-session safe)
    - 5s poll with heartbeat updates
    - SIGTERM → 3s → SIGKILL escalation on timeout

    Returns (returncode, stdout, stderr).
    Raises TimeoutError on timeout.
    """
    instruction = (
        f"Process the input and respond with ONLY the summary "
        f"wrapped in {begin_marker} and {end_marker} markers."
    )

    # Open payload as stdin — no shell=True, no injection risk
    # -p = headless (non-interactive) mode, no tool execution possible
    payload_file = open(payload_path, 'r', encoding='utf-8')

    try:
        proc = subprocess.Popen(
            ['gemini', '-p', instruction],
            stdin=payload_file,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            start_new_session=True
        )
    finally:
        payload_file.close()

    deadline = time.time() + GEMINI_TIMEOUT

    while True:
        rc = proc.poll()
        update_heartbeat()

        if rc is not None:
            stdout, stderr = proc.communicate()
            return rc, stdout, stderr

        if time.time() > deadline:
            # start_new_session=True makes proc.pid the group leader,
            # so proc.pid IS the pgid — no getpgid race needed
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
                proc.wait(timeout=2)
            raise TimeoutError(
                f"Gemini timed out after {GEMINI_TIMEOUT}s")

        time.sleep(HEARTBEAT_INTERVAL)


def extract_summary(stdout, begin_marker, end_marker):
    """Extract text between dynamic begin/end markers."""
    begin_idx = stdout.find(begin_marker)
    end_idx = stdout.find(end_marker)

    if begin_idx == -1 or end_idx == -1 or end_idx <= begin_idx:
        return None

    # Extract content between markers (skip the marker line itself)
    content = stdout[begin_idx + len(begin_marker):end_idx].strip()
    return content if content else None


def validate_summary(summary, begin_marker):
    """
    Validate summary quality.
    Returns (is_valid, reason) tuple.
    """
    if not summary:
        return False, "empty summary"

    if len(summary) < MIN_SUMMARY_CHARS:
        return False, f"too short ({len(summary)} chars, min {MIN_SUMMARY_CHARS})"

    # Check for pollution patterns
    for pattern in POLLUTION_PATTERNS:
        if pattern in summary:
            # Allow our own markers (they're dynamic)
            if pattern.startswith('===') and begin_marker[:20] in summary:
                continue
            return False, f"pollution detected: {pattern}"

    return True, "ok"


def build_fallback_summary(markers, delta_text, save_id, start_byte,
                           end_byte):
    """Build structured bullet summary from markers + raw delta head."""
    lines = []
    lines.append("### Fallback Summary (Gemini unavailable)\n")

    if markers:
        lines.append("**Milestone Facts:**")
        for m in markers:
            ts = m.get('ts', '?')
            if 'T' in str(ts):
                # Extract HH:MM from ISO timestamp
                try:
                    ts = ts.split('T')[1][:5]
                except (IndexError, AttributeError):
                    pass
            mtype = m.get('type', '?')
            data = m.get('data', {})

            if mtype == 'git_commit':
                lines.append(
                    f"- [{ts}] Commit {data.get('commit', '?')}: "
                    f"{data.get('message', '')}")
            elif mtype in ('test_pass', 'test_fail'):
                lines.append(
                    f"- [{ts}] {mtype}: {data.get('command', '')} "
                    f"(exit {data.get('exit_code', '?')})")
            elif mtype == 'bash_fail':
                lines.append(
                    f"- [{ts}] bash_fail: "
                    f"{data.get('command_prefix', '')} "
                    f"(exit {data.get('exit_code', '?')})")
            elif mtype == 'file_write_batch':
                files = data.get('files', [])
                lines.append(
                    f"- [{ts}] {len(files)} files written: "
                    f"{', '.join(files[:5])}"
                    f"{'...' if len(files) > 5 else ''}")
            else:
                lines.append(f"- [{ts}] {mtype}")
        lines.append("")

    # Raw delta head as recovery data
    raw_head = delta_text[:MAX_RAW_FALLBACK_CHARS]
    if len(delta_text) > MAX_RAW_FALLBACK_CHARS:
        raw_head += f"\n\n... ({len(delta_text) - MAX_RAW_FALLBACK_CHARS} chars truncated)"

    lines.append("**Raw Transcript Head:**")
    lines.append("```")
    lines.append(raw_head)
    lines.append("```")

    return '\n'.join(lines)


# ─── Segment Management ─────────────────────────────────────────────────────

def get_segment_info(state, transcript_uuid):
    """Get current segment path and metadata from state."""
    family = state.get('families', {}).get(transcript_uuid)
    if not family:
        return None, None

    current_num = family.get('current_segment', 1)
    for seg in family.get('segments', []):
        if seg['segment'] == current_num:
            return seg['path'], seg
    return None, None


def new_segment_path(current_path, new_num):
    """Derive new segment path by replacing _sNN_ in filename."""
    return re.sub(r'_s\d+_', f'_s{new_num:02d}_', current_path)


def create_segment_file(path, prev_path, segment_num, agent='claude'):
    """Create a new segment file with continuation header."""
    header = (
        f"# Session Log — Segment {segment_num} (continued)\n\n"
        f"**Continued from:** `{os.path.basename(prev_path)}`\n"
        f"**Created:** {_now().strftime('%Y-%m-%d %H:%M:%S %Z')}\n"
        f"**Agent:** {agent}\n\n"
        f"---\n\n"
    )
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with open(path, 'w') as f:
        f.write(header)


def check_rotation_needed(segment_path, append_block, save_count):
    """
    Check if rotation needed using PROJECTED post-append size.

    Rules from spec:
    - If projected > 200KB AND current has content: rotate
    - If projected > 200KB BUT empty: allow overshoot, don't split
    - NEVER split a single save/checkpoint block across segments
    """
    try:
        current_size = os.path.getsize(segment_path)
    except OSError:
        current_size = 0

    projected = current_size + len(append_block.encode('utf-8'))

    if projected > SEGMENT_SIZE_LIMIT:
        if save_count > 0:
            return True  # rotate
        # Empty segment + huge block → allow overshoot
    return False


# ─── Render ──────────────────────────────────────────────────────────────────

def render_block(summary, save_id, save_seq, start_byte, end_byte,
                 is_fallback=False):
    """Render the summary into a markdown block for the session log."""
    ts = _now().strftime('%H:%M %Z')
    date = _now().strftime('%Y-%m-%d')
    word_count = len(summary.split())

    fallback_tag = " [FALLBACK]" if is_fallback else ""

    block = (
        f"\n---\n\n"
        f"## {ts} — Incremental Update #{save_seq} ({date})"
        f"{fallback_tag}\n\n"
        f"{summary}\n\n"
        f"*Save: {save_id}, bytes {start_byte}-{end_byte}, "
        f"{word_count} words*\n\n"
    )
    return block


# ─── Main Worker Logic ────────────────────────────────────────────────────────

def run(save_id, precompact=False):
    """Main worker execution flow. precompact=True uses precompact-complete."""
    payload_path = None
    transcript_uuid = None

    try:
        # ── 1. Source environment ──
        source_env()

        # ── 2. Load and validate state ──
        state = load_state()
        if not state:
            raise RuntimeError("Cannot load state file")

        running = state.get('running', {})
        if running.get('save_id') != save_id:
            raise RuntimeError(
                f"save_id mismatch: state has {running.get('save_id')}, "
                f"expected {save_id}")

        transcript_path = running.get('transcript_path')
        transcript_key = running.get('transcript_key')
        # Use reserved range — NOT cursor (cursor may be 0 after
        # transcript change while reserved range is the actual bytes
        # computed by the token gate at launch time)
        start_byte = running.get('reserved_start_byte',
                                 state['cursor']['last_completed_end_byte'])
        end_byte = running['reserved_end_byte']
        save_seq = state['save_seq']

        if not transcript_path or not os.path.exists(transcript_path):
            raise RuntimeError(
                f"Transcript not found: {transcript_path}")

        transcript_uuid = Path(transcript_path).stem
        family12 = transcript_uuid[:12]

        # Find family and segment
        seg_path, seg_info = get_segment_info(state, transcript_uuid)
        if not seg_path:
            raise RuntimeError(
                f"No family/segment for transcript {transcript_uuid}")

        # Find markers file
        markers_dir = os.path.dirname(seg_path)
        markers_file = os.path.join(
            markers_dir, f'{family12}_markers.jsonl')

        status_log(
            f"WORKER {save_id} — extract {start_byte}-{end_byte} "
            f"({end_byte - start_byte} bytes)")

        # ── 3. Extract delta ──
        set_phase('extract')
        delta_text = parse_delta(transcript_path, start_byte, end_byte)

        if not delta_text.strip():
            raise RuntimeError("Empty transcript delta")

        # ── 4. Load milestone markers in range ──
        markers = load_markers(markers_file, start_byte, end_byte)

        # ── 5. Generate dynamic markers (meta-recursion safe) ──
        run_id = uuid_mod.uuid4().hex[:12]
        begin_marker = f"===BEGIN_SUMMARY_{run_id}==="
        end_marker = f"===END_SUMMARY_{run_id}==="

        # ── 6. Write payload to unique temp file ──
        prompt = build_prompt(
            delta_text, markers, begin_marker, end_marker)

        payload_fd, payload_path = tempfile.mkstemp(
            prefix='claude-save-', suffix='.txt')
        with os.fdopen(payload_fd, 'w', encoding='utf-8') as f:
            f.write(prompt)

        # ── 7. Call Gemini ──
        set_phase('summarize')
        summary = None
        is_fallback = False

        try:
            rc, stdout, stderr = call_gemini(
                payload_path, begin_marker, end_marker)

            if rc != 0:
                status_log(
                    f"WORKER {save_id} — Gemini exit {rc}: "
                    f"{stderr[:200] if stderr else 'no stderr'}")
                raise RuntimeError(f"Gemini exit code {rc}")

            # Extract and validate
            summary = extract_summary(stdout, begin_marker, end_marker)
            if summary:
                valid, reason = validate_summary(summary, begin_marker)
                if not valid:
                    status_log(
                        f"WORKER {save_id} — invalid output: {reason}")
                    summary = None

        except TimeoutError as e:
            status_log(f"WORKER {save_id} — {e}")

        # ── 8. Retry once with simplified prompt ──
        if summary is None:
            status_log(f"WORKER {save_id} — retrying with simplified prompt")

            retry_prompt = build_prompt(
                delta_text, markers, begin_marker, end_marker,
                simplified=True)

            # Rewrite temp file
            with open(payload_path, 'w', encoding='utf-8') as f:
                f.write(retry_prompt)

            try:
                rc, stdout, stderr = call_gemini(
                    payload_path, begin_marker, end_marker)

                if rc == 0:
                    summary = extract_summary(
                        stdout, begin_marker, end_marker)
                    if summary:
                        valid, reason = validate_summary(
                            summary, begin_marker)
                        if not valid:
                            summary = None
            except TimeoutError:
                pass

        # ── 9. Fallback: markers + raw head ──
        if summary is None:
            status_log(
                f"WORKER {save_id} — both attempts failed, "
                f"using fallback")
            summary = build_fallback_summary(
                markers, delta_text, save_id, start_byte, end_byte)
            is_fallback = True

        # ── 10. Render the block ──
        set_phase('append')
        rendered = render_block(
            summary, save_id, save_seq, start_byte, end_byte,
            is_fallback=is_fallback)

        # ── 11. Check segment rotation ──
        # Re-read state for fresh segment info (markers may have
        # updated it during our Gemini call)
        state = load_state()
        seg_path, seg_info = get_segment_info(state, transcript_uuid)

        if not seg_path:
            raise RuntimeError(
                f"No open segment after state reload for "
                f"{transcript_uuid}")

        save_count = seg_info.get('save_count', 0) if seg_info else 0

        if check_rotation_needed(seg_path, rendered, save_count):
            # Rotate: close current, open new
            family = state['families'][transcript_uuid]
            current_num = family['current_segment']
            new_num = current_num + 1
            new_path = new_segment_path(seg_path, new_num)

            create_segment_file(
                new_path, seg_path, new_num,
                agent=family.get('agent', 'claude'))

            ctl('rotate-segment',
                '--transcript-uuid', transcript_uuid,
                '--new-path', new_path,
                '--end-byte', str(end_byte),
                required=True)

            seg_path = new_path
            status_log(
                f"WORKER {save_id} — rotated to segment "
                f"s{new_num:02d}")

        # ── 12. Append to segment ──
        with open(seg_path, 'a', encoding='utf-8') as f:
            f.write(rendered)

        # ── 13. Update segment stats ──
        ctl('update-segment',
            '--transcript-uuid', transcript_uuid,
            '--save-count-incr', '1',
            required=True)

        # ── 14. Mark complete ──
        if precompact:
            ctl('precompact-complete',
                '--end-byte', str(end_byte),
                required=True)
        else:
            ctl('complete',
                '--save-id', save_id,
                '--end-byte', str(end_byte),
                required=True)

        word_count = len(summary.split())
        fallback_note = ", FALLBACK" if is_fallback else ""
        pc_note = " (precompact)" if precompact else ""
        status_log(
            f"WORKER {save_id} — complete{pc_note}, {word_count} words"
            f"{fallback_note} → {os.path.basename(seg_path)}")

    except Exception as e:
        # Record failure
        error_msg = str(e)[:200]
        error_code = type(e).__name__

        # Check for rate limiting
        if 'rate' in error_msg.lower() or '429' in error_msg:
            error_code = 'rate_limit'

        if precompact:
            # Clear precompact state on failure too
            ctl('precompact-complete',
                '--end-byte', str(state.get('cursor', {}).get(
                    'last_completed_end_byte', 0)))
        else:
            ctl('fail',
                '--save-id', save_id,
                '--code', error_code,
                '--message', error_msg)

        status_log(f"WORKER {save_id} — FAILED: {error_code}: {error_msg}")

    finally:
        # Clean up temp file
        if payload_path and os.path.exists(payload_path):
            try:
                os.unlink(payload_path)
            except OSError:
                pass

        # Release lock
        ctl('unlock')


# ─── CLI ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description='Session Save Worker — v3.1 Gemini Summarizer')
    parser.add_argument(
        '--save-id', required=True,
        help='Save ID from ctl reserve (e.g., tg-0017)')
    parser.add_argument(
        '--precompact', action='store_true',
        help='Run in precompact mode (calls precompact-complete)')
    args = parser.parse_args()

    # Verify lock is held with our PID
    state = load_state()
    if state:
        running = state.get('running', {})
        tk = running.get('transcript_key') or \
            state.get('transcript', {}).get('key')
        if tk:
            import hashlib
            key_hash = hashlib.sha256(tk.encode()).hexdigest()[:16]
            lock_dir = f"/tmp/claude-session-save-{key_hash}.lock"
            if not os.path.isdir(lock_dir):
                print(json.dumps({
                    "ok": False,
                    "error": "Lock not held — worker cannot proceed"
                }))
                sys.exit(1)
            # Verify PID matches this worker.
            # Retry for up to 2s because the token gate updates
            # the lock PID AFTER spawning us — there's a brief
            # race window between background spawn and PID handoff.
            pid_file = os.path.join(lock_dir, 'pid')
            pid_matched = False
            for _attempt in range(20):
                try:
                    with open(pid_file) as f:
                        lock_pid = int(f.read().strip())
                    if lock_pid == os.getpid():
                        pid_matched = True
                        break
                except (FileNotFoundError, ValueError):
                    pass
                import time as _time
                _time.sleep(0.1)

            if not pid_matched:
                try:
                    with open(pid_file) as f:
                        lock_pid = int(f.read().strip())
                except Exception:
                    lock_pid = '?'
                print(json.dumps({
                    "ok": False,
                    "error": f"Lock PID mismatch after 2s: lock has "
                             f"{lock_pid}, worker is {os.getpid()}"
                }))
                sys.exit(1)

    run(args.save_id, precompact=args.precompact)


if __name__ == '__main__':
    main()
