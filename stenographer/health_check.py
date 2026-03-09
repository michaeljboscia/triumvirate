#!/usr/bin/env python3
"""
Stenographer Health Check — Fast system status for session start.

Reports on all components of the session notes v2 pipeline.
Designed to run in <2 seconds — no model inference, just connectivity.

Usage:
    python3 health_check.py                     # full report
    python3 health_check.py --brief             # one-line status
    python3 health_check.py --json              # machine-readable

Exit codes:
    0 = all healthy
    1 = degraded (some components down, system can still function)
    2 = critical (session notes will not work)
"""

import json
import os
import sys
import time
import urllib.request
import urllib.error
from datetime import datetime
from pathlib import Path

try:
    from zoneinfo import ZoneInfo
    EASTERN = ZoneInfo('America/New_York')
except ImportError:
    EASTERN = None

TRIUMVIRATE_DIR = Path.home() / '.triumvirate'
STENOGRAPHER_STATE = TRIUMVIRATE_DIR / 'stenographer-state.json'
SESSION_LOG_STATE = TRIUMVIRATE_DIR / 'session-log-state.json'
STENOGRAPHER_LOG = TRIUMVIRATE_DIR / 'stenographer.log'
OLLAMA_BASE = os.environ.get('OLLAMA_HOST', 'http://localhost:11434')
OLLAMA_MODEL = os.environ.get('STENOGRAPHER_MODEL', 'qwen2.5:32b')


def _now():
    if EASTERN:
        return datetime.now(EASTERN)
    return datetime.now()


def check_ollama_reachable() -> dict:
    """Check if Ollama API is responding."""
    try:
        req = urllib.request.Request(f'{OLLAMA_BASE}/api/tags', method='GET')
        with urllib.request.urlopen(req, timeout=3) as resp:
            data = json.loads(resp.read())
            models = [m['name'] for m in data.get('models', [])]
            return {
                'status': 'ok',
                'models': models,
                'model_count': len(models),
            }
    except urllib.error.URLError as e:
        return {'status': 'error', 'error': f'Connection refused: {e.reason}'}
    except Exception as e:
        return {'status': 'error', 'error': str(e)}


def check_model_available(ollama_result: dict) -> dict:
    """Check if the configured model is pulled."""
    if ollama_result['status'] != 'ok':
        return {'status': 'skip', 'reason': 'Ollama not reachable'}

    models = ollama_result.get('models', [])
    # Check for exact match or prefix match (qwen2.5:32b matches qwen2.5:32b-instruct-q5_K_M)
    found = any(OLLAMA_MODEL in m or m.startswith(OLLAMA_MODEL.split(':')[0]) for m in models)
    if found:
        return {'status': 'ok', 'model': OLLAMA_MODEL}
    return {
        'status': 'error',
        'error': f'Model {OLLAMA_MODEL} not found',
        'available': models,
        'fix': f'ollama pull {OLLAMA_MODEL}',
    }


def check_stenographer_state() -> dict:
    """Check stenographer state file health."""
    if not STENOGRAPHER_STATE.exists():
        return {'status': 'ok', 'detail': 'No state yet (first run)'}

    try:
        with open(STENOGRAPHER_STATE) as f:
            state = json.load(f)

        sessions = state.get('sessions', {})
        if not sessions:
            return {'status': 'ok', 'detail': 'State exists, no active sessions'}

        # Report on each tracked agent
        agent_status = {}
        for agent, session in sessions.items():
            last_time = session.get('last_save_time', 0)
            saves = session.get('saves_count', 0)
            transcript = session.get('active_transcript', '')

            if last_time > 0:
                age_secs = int(time.time()) - last_time
                if age_secs < 3600:
                    age_str = f'{age_secs // 60}m ago'
                elif age_secs < 86400:
                    age_str = f'{age_secs // 3600}h ago'
                else:
                    age_str = f'{age_secs // 86400}d ago'
            else:
                age_str = 'never'

            agent_status[agent] = {
                'saves': saves,
                'last_save': age_str,
                'transcript': os.path.basename(transcript) if transcript else 'none',
            }

        return {'status': 'ok', 'agents': agent_status}
    except (json.JSONDecodeError, IOError) as e:
        return {'status': 'error', 'error': f'State file corrupt: {e}'}


def check_session_log_state() -> dict:
    """Check session_log_path state file."""
    if not SESSION_LOG_STATE.exists():
        return {'status': 'ok', 'detail': 'No state yet'}

    try:
        with open(SESSION_LOG_STATE) as f:
            state = json.load(f)

        entries = len(state)
        if entries == 0:
            return {'status': 'ok', 'detail': 'State exists, no tracked sessions'}

        # Find most recent entry
        latest = max(state.values(), key=lambda e: e.get('last_append_at', ''))
        return {
            'status': 'ok',
            'tracked_sessions': entries,
            'latest_repo': latest.get('repo', '?'),
            'latest_append': latest.get('last_append_at', '?'),
        }
    except (json.JSONDecodeError, IOError) as e:
        return {'status': 'error', 'error': f'State file corrupt: {e}'}


def check_gemini_cli() -> dict:
    """Check if Gemini CLI is available."""
    import shutil
    gemini_path = os.environ.get('GEMINI_CLI_PATH') or shutil.which('gemini')
    if gemini_path and os.path.isfile(gemini_path):
        return {'status': 'ok', 'path': gemini_path}
    return {
        'status': 'warning',
        'error': 'Gemini CLI not found in PATH',
        'impact': 'Gap-fill at compaction will use raw fallback',
    }


def check_recent_log_entries() -> dict:
    """Check stenographer.log for recent errors."""
    if not STENOGRAPHER_LOG.exists():
        return {'status': 'ok', 'detail': 'No log file yet'}

    try:
        # Read last 20 lines
        with open(STENOGRAPHER_LOG) as f:
            lines = f.readlines()

        recent = lines[-20:] if len(lines) > 20 else lines
        errors = [l.strip() for l in recent if '[ERROR]' in l]
        warns = [l.strip() for l in recent if '[WARN]' in l]

        if errors:
            return {
                'status': 'warning',
                'recent_errors': len(errors),
                'last_error': errors[-1][:200],
                'total_lines': len(lines),
            }
        return {
            'status': 'ok',
            'total_lines': len(lines),
            'recent_warnings': len(warns),
        }
    except IOError as e:
        return {'status': 'error', 'error': str(e)}


def run_all_checks() -> dict:
    """Run all health checks and return combined report."""
    ollama = check_ollama_reachable()
    model = check_model_available(ollama)
    steno_state = check_stenographer_state()
    slp_state = check_session_log_state()
    gemini = check_gemini_cli()
    log = check_recent_log_entries()

    checks = {
        'ollama': ollama,
        'model': model,
        'stenographer_state': steno_state,
        'session_log_state': slp_state,
        'gemini_cli': gemini,
        'recent_logs': log,
    }

    # Determine overall status
    statuses = [c['status'] for c in checks.values()]
    if 'error' in statuses:
        if ollama['status'] == 'error' or model['status'] == 'error':
            overall = 'critical'
        else:
            overall = 'degraded'
    elif 'warning' in statuses:
        overall = 'degraded'
    else:
        overall = 'healthy'

    return {
        'overall': overall,
        'timestamp': _now().strftime('%Y-%m-%d %H:%M:%S %Z'),
        'checks': checks,
    }


def format_brief(report: dict) -> str:
    """One-line status for additionalContext injection."""
    overall = report['overall']
    checks = report['checks']

    icon = {'healthy': 'ok', 'degraded': 'degraded', 'critical': 'CRITICAL'}[overall]

    parts = []

    # Ollama
    if checks['ollama']['status'] == 'ok':
        parts.append(f"Ollama: ok ({checks['model'].get('model', '?')})")
    else:
        parts.append(f"Ollama: DOWN")

    # Stenographer saves
    steno = checks['stenographer_state']
    if steno.get('agents'):
        for agent, info in steno['agents'].items():
            parts.append(f"{agent}: {info['saves']} saves, last {info['last_save']}")
    else:
        parts.append("Stenographer: no saves yet")

    # Gemini
    if checks['gemini_cli']['status'] != 'ok':
        parts.append("Gemini CLI: missing")

    # Errors
    log = checks['recent_logs']
    if log.get('recent_errors'):
        parts.append(f"Errors: {log['recent_errors']} recent")

    return f"Session Notes v2 [{icon}]: {' | '.join(parts)}"


def format_full(report: dict) -> str:
    """Multi-line status report."""
    overall = report['overall']
    ts = report['timestamp']
    checks = report['checks']

    icon = {'healthy': 'ok', 'degraded': 'WARNING', 'critical': 'CRITICAL'}[overall]

    lines = [
        f"Session Notes v2 Health Check [{icon}] — {ts}",
        "",
    ]

    # Ollama
    o = checks['ollama']
    if o['status'] == 'ok':
        lines.append(f"  Ollama:      ok ({o['model_count']} models loaded)")
    else:
        lines.append(f"  Ollama:      FAILED — {o.get('error', '?')}")

    # Model
    m = checks['model']
    if m['status'] == 'ok':
        lines.append(f"  Model:       ok ({m['model']})")
    elif m['status'] == 'error':
        lines.append(f"  Model:       MISSING — {m.get('error', '?')}")
        lines.append(f"               Fix: {m.get('fix', '?')}")

    # Gemini
    g = checks['gemini_cli']
    if g['status'] == 'ok':
        lines.append(f"  Gemini CLI:  ok ({g['path']})")
    else:
        lines.append(f"  Gemini CLI:  {g.get('error', 'missing')}")

    # Stenographer state
    s = checks['stenographer_state']
    if s.get('agents'):
        for agent, info in s['agents'].items():
            lines.append(f"  {agent:12s}  {info['saves']} saves, last: {info['last_save']}, transcript: {info['transcript']}")
    else:
        lines.append(f"  Stenographer: {s.get('detail', 'unknown')}")

    # Session log state
    sl = checks['session_log_state']
    if sl.get('tracked_sessions'):
        lines.append(f"  Log paths:   {sl['tracked_sessions']} tracked, latest: {sl.get('latest_repo', '?')} ({sl.get('latest_append', '?')})")
    else:
        lines.append(f"  Log paths:   {sl.get('detail', 'unknown')}")

    # Recent logs
    lg = checks['recent_logs']
    if lg.get('recent_errors'):
        lines.append(f"  Log errors:  {lg['recent_errors']} recent — last: {lg.get('last_error', '?')[:100]}")
    elif lg.get('total_lines'):
        lines.append(f"  Log file:    {lg['total_lines']} lines, clean")

    return '\n'.join(lines)


def main():
    import argparse
    parser = argparse.ArgumentParser(description='Stenographer Health Check')
    parser.add_argument('--brief', action='store_true', help='One-line status')
    parser.add_argument('--json', action='store_true', help='JSON output')
    args = parser.parse_args()

    report = run_all_checks()

    if args.json:
        json.dump(report, sys.stdout, indent=2)
    elif args.brief:
        print(format_brief(report))
    else:
        print(format_full(report))

    # Exit code
    if report['overall'] == 'critical':
        sys.exit(2)
    elif report['overall'] == 'degraded':
        sys.exit(1)
    sys.exit(0)


if __name__ == '__main__':
    main()
