#!/usr/bin/env python3
"""
Stenographer — Incremental Session Notes via Local LLM

The zero-context-cost session notes engine. Reads transcript deltas from
Claude, Gemini, or Codex sessions, feeds them to a local Ollama model,
and appends substantive narrative paragraphs to rolling session logs.

Called by the token gate hook (post-tool-use-token-gate.sh) when a
transcript growth threshold is crossed.

Usage:
    stenographer.py --agent claude --transcript /path/to/transcript.jsonl
    stenographer.py --agent gemini --transcript /path/to/session.json
    stenographer.py --agent codex --transcript /path/to/rollout.jsonl
    stenographer.py --agent claude --transcript /path --session-log /path/to/log.md

Architecture:
    1. Acquire per-transcript lock (mkdir-based, macOS compatible)
    2. Load state from ~/.triumvirate/stenographer-state.json
    3. Detect transcript rotation (path/size mismatch)
    4. Extract delta via agent-specific parser
    5. Skip if delta is below content threshold
    6. Health-check Ollama, then POST to /api/generate
    7. Append timestamped section to rolling session log
    8. Update state ONLY after successful append
    9. Log structured output for observability

State file: ~/.triumvirate/stenographer-state.json
Lock dir:   ~/.triumvirate/locks/stenographer-<transcript-hash>/
Log file:   ~/.triumvirate/stenographer.log
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import signal
import sys
import time
import urllib.request
import urllib.error
from datetime import datetime, timezone
from pathlib import Path
try:
    from zoneinfo import ZoneInfo
    # Use TZ env var if set, otherwise system local time
    _tz_name = os.environ.get('TZ', '')
    if _tz_name:
        LOCAL_TZ = ZoneInfo(_tz_name)
    else:
        LOCAL_TZ = None  # Will use naive local time (system timezone)
except ImportError:
    # Python 3.8 fallback
    LOCAL_TZ = None

# Add parent dir to path for parser imports
SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from parsers import claude as claude_parser
from parsers import gemini as gemini_parser
from parsers import codex as codex_parser
import session_log_path

# ─── Configuration ──────────────────────────────────────────────────────────

TRIUMVIRATE_DIR = Path.home() / '.triumvirate'
STATE_FILE = TRIUMVIRATE_DIR / 'stenographer-state.json'
LOCK_BASE = TRIUMVIRATE_DIR / 'locks'
LOG_FILE = TRIUMVIRATE_DIR / 'stenographer.log'

# Ollama host auto-detection: tries OLLAMA_HOST env first, then OLLAMA_LOCAL,
# then falls back to OLLAMA_TUNNEL (if set). Set OLLAMA_HOST to skip auto-detect.
OLLAMA_BASE = os.environ.get('OLLAMA_HOST', '')  # empty = auto-detect
OLLAMA_LOCAL = os.environ.get('OLLAMA_LOCAL', 'http://localhost:11434')
OLLAMA_TUNNEL = os.environ.get('OLLAMA_TUNNEL', '')  # optional remote endpoint
OLLAMA_MODEL = os.environ.get('STENOGRAPHER_MODEL', 'qwen2.5:7b')
OLLAMA_TIMEOUT = int(os.environ.get('STENOGRAPHER_TIMEOUT', '300'))  # seconds
OLLAMA_NUM_CTX = int(os.environ.get('STENOGRAPHER_NUM_CTX', '65536'))

# Optional Cloudflare Access service token — required when calling via tunnel.
# Create in Cloudflare Zero Trust → Access → Service Tokens.
# Set in shell env or ~/.triumvirate/.env (sourced by the token gate hook).
_CF_CLIENT_ID = os.environ.get('CF_ACCESS_CLIENT_ID', '')
_CF_CLIENT_SECRET = os.environ.get('CF_ACCESS_CLIENT_SECRET', '')
OLLAMA_HEADERS: dict = {}  # populated by _resolve_ollama_base()


def _resolve_ollama_base() -> str:
    """Auto-detect Ollama host: try local first (2s), fall back to tunnel.

    Sets OLLAMA_HEADERS with CF Access credentials when using the tunnel.
    Result is cached into OLLAMA_BASE for the lifetime of this process.
    """
    global OLLAMA_BASE, OLLAMA_HEADERS

    # Explicit override wins — OLLAMA_HOST env var or already resolved
    if OLLAMA_BASE:
        return OLLAMA_BASE

    # Try local first — 2s timeout keeps this invisible when Ollama is local
    try:
        req = urllib.request.Request(f"{OLLAMA_LOCAL}/api/tags")
        with urllib.request.urlopen(req, timeout=2) as resp:
            if resp.status == 200:
                OLLAMA_BASE = OLLAMA_LOCAL
                return OLLAMA_BASE
    except Exception:
        pass

    # Fall back to tunnel (if configured)
    if OLLAMA_TUNNEL:
        OLLAMA_BASE = OLLAMA_TUNNEL
        if _CF_CLIENT_ID and _CF_CLIENT_SECRET:
            OLLAMA_HEADERS = {
                'CF-Access-Client-Id': _CF_CLIENT_ID,
                'CF-Access-Client-Secret': _CF_CLIENT_SECRET,
            }
        return OLLAMA_BASE

    # No tunnel configured — use local anyway (will fail with clear error)
    OLLAMA_BASE = OLLAMA_LOCAL
    return OLLAMA_BASE


def _set_model(model: str):
    """Update the global model setting."""
    global OLLAMA_MODEL
    OLLAMA_MODEL = model

# Minimum chars of extracted content to warrant a save
MIN_CONTENT_THRESHOLD = 400

# Maximum chars to send to the model.
# Smaller models need smaller prompts to stay within timeout.
# qwen2.5:7b on M-series Mac: ~5000 chars ≈ 220s (within 300s timeout).
# Override with STENOGRAPHER_MAX_CHARS env var for faster hardware or bigger models.
MAX_PROMPT_CHARS = int(os.environ.get('STENOGRAPHER_MAX_CHARS', '5000'))


def _now_local() -> datetime:
    """Return current time in configured timezone."""
    if LOCAL_TZ:
        return datetime.now(LOCAL_TZ)
    return datetime.now()


# ─── Logging ────────────────────────────────────────────────────────────────

def log(level: str, msg: str, **kwargs):
    """Structured log entry."""
    ts = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    extras = ' '.join(f'{k}={v}' for k, v in kwargs.items())
    line = f"{ts} [{level}] {msg} {extras}".strip()
    try:
        LOG_FILE.parent.mkdir(parents=True, exist_ok=True)
        with open(LOG_FILE, 'a') as f:
            f.write(line + '\n')
    except OSError:
        pass
    if level == 'ERROR':
        print(line, file=sys.stderr)


# ─── State Management ──────────────────────────────────────────────────────

def load_state() -> dict:
    """Load stenographer state, returning defaults if missing."""
    default = {
        'sessions': {},  # keyed by agent name
    }
    if not STATE_FILE.exists():
        return default
    try:
        with open(STATE_FILE) as f:
            return json.load(f)
    except (json.JSONDecodeError, IOError):
        log('WARN', 'State file corrupt, resetting', path=str(STATE_FILE))
        return default


def save_state(state: dict):
    """Atomic state write via temp file."""
    STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
    tmp = str(STATE_FILE) + '.tmp'
    with open(tmp, 'w') as f:
        json.dump(state, f, indent=2)
    os.rename(tmp, str(STATE_FILE))


def get_session_state(state: dict, agent: str) -> dict:
    """Get or create per-agent session state."""
    if agent not in state['sessions']:
        state['sessions'][agent] = {
            'active_transcript': None,
            'last_save_bytes': 0,        # For JSONL agents (Claude, Codex)
            'last_message_index': 0,     # For JSON agents (Gemini)
            'last_save_time': 0,
            'saves_count': 0,
            'session_log_path': None,
        }
    return state['sessions'][agent]


# ─── Locking ───────────────────────────────────────────────────────────────

class TranscriptLock:
    """mkdir-based lock for macOS compatibility (no flock)."""

    def __init__(self, transcript_path: str):
        h = hashlib.md5(transcript_path.encode()).hexdigest()[:12]
        self.lock_dir = LOCK_BASE / f'stenographer-{h}'
        self.acquired = False

    def acquire(self) -> bool:
        """Try to acquire lock. Returns True if acquired."""
        try:
            self.lock_dir.parent.mkdir(parents=True, exist_ok=True)
            os.mkdir(self.lock_dir)
            self.acquired = True
            # Write PID for debugging
            with open(self.lock_dir / 'pid', 'w') as f:
                f.write(str(os.getpid()))
            return True
        except FileExistsError:
            # Check if lock is stale (PID no longer running)
            pid_file = self.lock_dir / 'pid'
            if pid_file.exists():
                pid = None
                try:
                    pid = int(pid_file.read_text().strip())
                    os.kill(pid, 0)  # Check if process exists
                except (ProcessLookupError, ValueError):
                    # Stale lock — remove and retry
                    log('WARN', 'Removing stale lock', pid=str(pid), lock=str(self.lock_dir))
                    shutil.rmtree(self.lock_dir, ignore_errors=True)
                    try:
                        os.mkdir(self.lock_dir)
                        self.acquired = True
                        with open(self.lock_dir / 'pid', 'w') as f:
                            f.write(str(os.getpid()))
                        return True
                    except FileExistsError:
                        return False
                except PermissionError:
                    # Process exists but we can't signal it — lock is valid
                    return False
            return False

    def release(self):
        """Release the lock."""
        if self.acquired:
            shutil.rmtree(self.lock_dir, ignore_errors=True)
            self.acquired = False

    def __enter__(self):
        if not self.acquire():
            raise RuntimeError(f"Could not acquire lock: {self.lock_dir}")
        return self

    def __exit__(self, *args):
        self.release()


# ─── Ollama Integration ────────────────────────────────────────────────────

def ollama_health_check() -> bool:
    """Quick health check — is Ollama running and reachable?"""
    url = f"{_resolve_ollama_base()}/api/tags"
    try:
        req = urllib.request.Request(url, headers=OLLAMA_HEADERS)
        with urllib.request.urlopen(req, timeout=3) as resp:
            return resp.status == 200
    except (urllib.error.URLError, OSError, TimeoutError):
        return False


def ollama_check_model() -> bool:
    """Check if the configured model is available."""
    url = f"{_resolve_ollama_base()}/api/tags"
    try:
        req = urllib.request.Request(url, headers=OLLAMA_HEADERS)
        with urllib.request.urlopen(req, timeout=5) as resp:
            data = json.loads(resp.read())
            models = [m.get('name', '') for m in data.get('models', [])]
            # Check both exact match and base name match
            model_base = OLLAMA_MODEL.split(':')[0]
            return any(
                OLLAMA_MODEL in m or model_base in m
                for m in models
            )
    except (urllib.error.URLError, OSError, TimeoutError, json.JSONDecodeError):
        return False


def ollama_generate(prompt: str) -> str:
    """
    Call Ollama REST API for generation.

    Uses /api/generate with stream=false for a single complete response.
    Returns the generated text, or raises on failure.
    """
    url = f"{_resolve_ollama_base()}/api/generate"
    payload = json.dumps({
        'model': OLLAMA_MODEL,
        'prompt': prompt,
        'stream': False,
        'options': {
            'temperature': 0,
            'num_ctx': OLLAMA_NUM_CTX,
        },
    }).encode('utf-8')

    req = urllib.request.Request(
        url,
        data=payload,
        headers={'Content-Type': 'application/json', **OLLAMA_HEADERS},
        method='POST',
    )

    try:
        with urllib.request.urlopen(req, timeout=OLLAMA_TIMEOUT) as resp:
            data = json.loads(resp.read())
            response_text = data.get('response', '').strip()
            if not response_text:
                raise ValueError("Empty response from Ollama")
            return response_text
    except urllib.error.URLError as e:
        raise RuntimeError(f"Ollama API error: {e}")
    except TimeoutError:
        raise RuntimeError(f"Ollama timed out after {OLLAMA_TIMEOUT}s")


# ─── Session Log Management ───────────────────────────────────────────────

def append_to_session_log(log_path: str, section_text: str, save_number: int, stats: dict):
    """Atomically append a new section to the session log."""
    now = _now_local()
    timestamp = now.strftime('%H:%M %Z')
    date_str = now.strftime('%Y-%m-%d')

    section = f"""
---

## {timestamp} — Incremental Update #{save_number} ({date_str})

{section_text}

*Stats: {stats.get('thinking_blocks', 0)} thinking blocks, {stats.get('tool_calls', 0)} tool calls, {stats.get('user_messages', 0)} user messages, {stats.get('chars_emitted', 0)} chars processed*

"""
    with open(log_path, 'a') as f:
        f.write(section)


# ─── Notification Helper ───────────────────────────────────────────────────

def _write_notify(payload: dict):
    """Write a completion or error notification for the token gate hook to surface.

    Uses atomic write (tmp → rename) to prevent the hook from reading a
    partially-written file if it fires mid-write. Best-effort — never raises.
    Deleted by the hook after first read.
    """
    try:
        notify_path = TRIUMVIRATE_DIR / 'stenographer-notify.json'
        tmp_path = notify_path.with_suffix('.tmp')
        with open(tmp_path, 'w') as f:
            json.dump(payload, f)
        os.rename(tmp_path, notify_path)  # atomic on POSIX
    except OSError:
        pass


# ─── Main Pipeline ─────────────────────────────────────────────────────────

def run(agent: str, transcript_path: str, session_log_override: str = None, cwd: str = None):
    """
    Main stenographer pipeline.

    1. Acquire lock
    2. Load state, detect rotation
    3. Extract delta
    4. Call Ollama
    5. Append to session log
    6. Update state (ONLY after successful append)
    """
    transcript_path = os.path.abspath(transcript_path)

    if not os.path.exists(transcript_path):
        log('ERROR', 'Transcript not found', path=transcript_path)
        return False

    # ─── Lock ───
    lock = TranscriptLock(transcript_path)
    if not lock.acquire():
        log('INFO', 'Lock held, skipping (another instance running)', path=transcript_path)
        return False

    try:
        # ─── State ───
        state = load_state()
        session = get_session_state(state, agent)

        current_size = os.path.getsize(transcript_path)

        # ─── Rotation detection ───
        if session['active_transcript'] != transcript_path:
            if session['active_transcript'] is not None:
                log('INFO', 'Transcript rotation detected',
                    old=session['active_transcript'], new=transcript_path)
            session['active_transcript'] = transcript_path
            session['last_save_bytes'] = 0
            session['last_message_index'] = 0
            session['saves_count'] = 0
            session['session_log_path'] = None

        # Size shrink detection (file truncated/replaced)
        if agent != 'gemini' and current_size < session['last_save_bytes']:
            log('WARN', 'Transcript shrank — resetting pointer',
                expected=session['last_save_bytes'], actual=current_size)
            session['last_save_bytes'] = 0

        # ─── Extract delta ───
        if agent == 'claude':
            start_byte = session['last_save_bytes']
            result = claude_parser.parse_delta(
                transcript_path, start_byte, current_size, MAX_PROMPT_CHARS
            )
            new_cursor = current_size
            cursor_key = 'last_save_bytes'

        elif agent == 'gemini':
            start_idx = session['last_message_index']
            result = gemini_parser.parse_delta(
                transcript_path, start_idx, None, MAX_PROMPT_CHARS
            )
            new_cursor = result.get('total_messages', start_idx)
            cursor_key = 'last_message_index'

        elif agent == 'codex':
            start_byte = session['last_save_bytes']
            result = codex_parser.parse_delta(
                transcript_path, start_byte, current_size, MAX_PROMPT_CHARS
            )
            new_cursor = current_size
            cursor_key = 'last_save_bytes'

        else:
            log('ERROR', f'Unknown agent: {agent}')
            return False

        delta_text = result['text']
        stats = result['stats']

        # ─── Content threshold check ───
        if len(delta_text.strip()) < MIN_CONTENT_THRESHOLD:
            log('INFO', 'Delta below threshold, skipping',
                chars=len(delta_text), threshold=MIN_CONTENT_THRESHOLD, agent=agent)
            # Still advance cursor to avoid reprocessing noise
            session[cursor_key] = new_cursor
            save_state(state)
            return False

        log('INFO', 'Delta extracted',
            agent=agent, chars=len(delta_text),
            thinking=stats.get('thinking_blocks', 0),
            tools=stats.get('tool_calls', 0))

        # ─── Ollama health check ───
        if not ollama_health_check():
            log('ERROR', 'Ollama not reachable', host=OLLAMA_BASE)
            _write_notify({'status': 'error', 'error': f'Ollama not reachable at {OLLAMA_BASE}',
                           'completed_at': _now_local().strftime('%H:%M %Z'),
                           'save_number': session.get('saves_count', 0) + 1})
            return False

        if not ollama_check_model():
            log('ERROR', 'Model not available', model=OLLAMA_MODEL)
            _write_notify({'status': 'error', 'error': f'Model not pulled: {OLLAMA_MODEL} — run: ollama pull {OLLAMA_MODEL}',
                           'completed_at': _now_local().strftime('%H:%M %Z'),
                           'save_number': session.get('saves_count', 0) + 1})
            return False

        # ─── Build prompt ───
        prompt_template_path = SCRIPT_DIR / 'prompts' / 'incremental.txt'
        if prompt_template_path.exists():
            prompt_template = prompt_template_path.read_text()
        else:
            prompt_template = (
                "Write 3-5 substantive paragraphs summarizing this coding session segment. "
                "Include specific file names, decisions, and outcomes. Write in past tense.\n\n"
                "Transcript segment:\n"
            )

        # Hard truncate delta to MAX_PROMPT_CHARS (parser may slightly overshoot)
        if len(delta_text) > MAX_PROMPT_CHARS:
            delta_text = delta_text[:MAX_PROMPT_CHARS] + f'\n\n[truncated — {len(delta_text) - MAX_PROMPT_CHARS} chars omitted]'
        prompt = prompt_template + delta_text

        # ─── Generate notes ───
        log('INFO', 'Calling Ollama', model=OLLAMA_MODEL,
            prompt_chars=len(prompt), num_ctx=OLLAMA_NUM_CTX)

        try:
            notes = ollama_generate(prompt)
        except (RuntimeError, ValueError) as e:
            log('ERROR', f'Ollama generation failed: {e}')
            _write_notify({'status': 'error', 'error': str(e),
                           'completed_at': _now_local().strftime('%H:%M %Z'),
                           'save_number': session.get('saves_count', 0) + 1})
            return False

        if not notes or notes.strip() == '':
            log('ERROR', 'Ollama returned empty response')
            _write_notify({'status': 'error', 'error': 'Ollama returned empty response — model may be overloaded or context too large',
                           'completed_at': _now_local().strftime('%H:%M %Z'),
                           'save_number': session.get('saves_count', 0) + 1})
            return False

        log('INFO', 'Notes generated', words=len(notes.split()), chars=len(notes))

        # ─── Append to session log ───
        # Uses session_log_path module — single source of truth for log location.
        if session_log_override:
            log_path = session_log_override
        else:
            effective_cwd = cwd or os.getcwd()
            log_path = session_log_path.get_or_create_session_log(
                effective_cwd, transcript_path, agent
            )

        session['saves_count'] += 1
        append_to_session_log(log_path, notes, session['saves_count'], stats)
        session_log_path.update_append_time(transcript_path)

        log('INFO', 'Appended to session log',
            path=log_path, save_number=session['saves_count'])

        # ─── Update state ONLY after successful append ───
        session[cursor_key] = new_cursor
        session['last_save_time'] = int(time.time())
        session['session_log_path'] = log_path
        save_state(state)

        log('INFO', 'State updated successfully',
            agent=agent, cursor=new_cursor, saves=session['saves_count'])

        # ─── Write completion notification for hook to surface ───
        # The token gate hook checks this file on the next tool call and
        # displays a visible block. Deleted after first read.
        _write_notify({
            'status': 'ok',
            'completed_at': _now_local().strftime('%H:%M %Z'),
            'log_path': log_path,
            'log_basename': os.path.basename(log_path),
            'words': len(notes.split()),
            'chars': len(notes),
            'save_number': session['saves_count'],
            'tool_calls': stats.get('tool_calls', 0),
            'user_messages': stats.get('user_messages', 0),
            'chars_processed': stats.get('chars_emitted', 0),
        })

        return True

    except Exception as e:
        log('ERROR', f'Unhandled error: {e}', agent=agent, transcript=transcript_path)
        import traceback
        log('ERROR', traceback.format_exc())
        return False

    finally:
        lock.release()


# ─── CLI ────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description='Stenographer — Incremental session notes via local LLM'
    )
    parser.add_argument(
        '--agent', required=True, choices=['claude', 'gemini', 'codex'],
        help='Which agent transcript to process'
    )
    parser.add_argument(
        '--transcript', required=True,
        help='Path to the transcript file'
    )
    parser.add_argument(
        '--session-log',
        help='Override session log path (default: auto-detect/create)'
    )
    parser.add_argument(
        '--model',
        help='Ollama model to use (default: from env or qwen2.5:7b)'
    )
    parser.add_argument(
        '--dry-run', action='store_true',
        help='Extract and display delta without calling Ollama'
    )
    parser.add_argument(
        '--reset', action='store_true',
        help='Reset state for this agent (process from beginning)'
    )
    parser.add_argument(
        '--cwd',
        help='Working directory of the project (used to locate hook session log)'
    )

    args = parser.parse_args()

    # Override model if specified
    if args.model:
        _set_model(args.model)

    # Reset mode
    if args.reset:
        state = load_state()
        if args.agent in state.get('sessions', {}):
            del state['sessions'][args.agent]
            save_state(state)
            print(f"Reset state for agent: {args.agent}")
        else:
            print(f"No state found for agent: {args.agent}")
        return

    # Dry run mode
    if args.dry_run:
        transcript_path = os.path.abspath(args.transcript)
        state = load_state()
        session = get_session_state(state, args.agent)

        if session['active_transcript'] != transcript_path:
            start = 0
        elif args.agent == 'gemini':
            start = session['last_message_index']
        else:
            start = session['last_save_bytes']

        print(f"Agent: {args.agent}")
        print(f"Transcript: {transcript_path}")
        print(f"Starting from: {start}")
        print(f"File size: {os.path.getsize(transcript_path)}")
        print()

        if args.agent == 'claude':
            result = claude_parser.parse_delta(
                transcript_path, start, os.path.getsize(transcript_path)
            )
        elif args.agent == 'gemini':
            result = gemini_parser.parse_delta(transcript_path, start)
        elif args.agent == 'codex':
            result = codex_parser.parse_delta(
                transcript_path, start, os.path.getsize(transcript_path)
            )

        print("--- Stats ---")
        for k, v in result['stats'].items():
            print(f"  {k}: {v}")
        print(f"\n--- Extracted text ({len(result['text'])} chars) ---")
        print(result['text'][:5000])
        if len(result['text']) > 5000:
            print(f"\n...[{len(result['text']) - 5000} more chars]")
        return

    # Normal run
    success = run(args.agent, args.transcript, args.session_log, args.cwd)
    sys.exit(0 if success else 1)


if __name__ == '__main__':
    main()
