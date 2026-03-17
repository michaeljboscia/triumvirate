#!/usr/bin/env python3
"""
Gap Fill Extractor — Finds what the stenographer missed.

Called by pre-compact.sh at compaction time. Extracts the gap between
the stenographer's last save point and the current transcript EOF.

Outputs JSON to stdout:
{
    "log_path": "/full/path/to/session-log.md",
    "existing_notes": "content of existing session log",
    "gap_text": "extracted transcript text not yet captured",
    "has_gap": true,
    "stenographer_bytes": 450000,
    "transcript_bytes": 600000
}

Usage:
    python3 gap_fill.py --project-dir /path --transcript /path/to/uuid.jsonl
    python3 gap_fill.py --project-dir /path --transcript /path/to/uuid.jsonl --agent claude
"""

import json
import os
import sys
from pathlib import Path

# Add parent dir for imports
SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import session_log_path
from parsers import claude as claude_parser

# Stenographer state file (same one the stenographer uses)
STENOGRAPHER_STATE = Path.home() / '.triumvirate' / 'stenographer-state.json'

# Maximum chars to extract for gap-fill (Gemini can handle more than Ollama)
MAX_GAP_CHARS = 500000


def load_stenographer_state() -> dict:
    """Load the stenographer's state file."""
    if not STENOGRAPHER_STATE.exists():
        return {'sessions': {}}
    try:
        with open(STENOGRAPHER_STATE) as f:
            return json.load(f)
    except (json.JSONDecodeError, IOError):
        return {'sessions': {}}


def get_last_save_bytes(state: dict, agent: str) -> int:
    """Get the byte offset of the stenographer's last successful save."""
    session = state.get('sessions', {}).get(agent, {})
    return session.get('last_save_bytes', 0)


def main():
    import argparse

    parser = argparse.ArgumentParser(description='Gap Fill Extractor')
    parser.add_argument('--project-dir', required=True,
                        help='Project working directory')
    parser.add_argument('--transcript', required=True,
                        help='Path to transcript JSONL file')
    parser.add_argument('--agent', default='claude',
                        help='Agent name (default: claude)')

    args = parser.parse_args()

    project_dir = args.project_dir
    transcript_path = os.path.abspath(args.transcript)
    agent = args.agent

    # Validate transcript exists
    if not os.path.exists(transcript_path):
        json.dump({
            'error': f'Transcript not found: {transcript_path}',
            'has_gap': False,
        }, sys.stdout)
        return

    # Find or create session log via single source of truth
    log_path = session_log_path.get_or_create_session_log(
        project_dir, transcript_path, agent
    )

    # Read existing session log content
    existing_notes = ''
    if os.path.exists(log_path):
        try:
            with open(log_path) as f:
                existing_notes = f.read()
        except IOError:
            pass

    # Find stenographer's last save point
    steno_state = load_stenographer_state()
    last_save_bytes = get_last_save_bytes(steno_state, agent)

    # Check if the stenographer was tracking a DIFFERENT transcript
    # (session rotation). If so, the gap is the entire file.
    steno_session = steno_state.get('sessions', {}).get(agent, {})
    if steno_session.get('active_transcript') != transcript_path:
        last_save_bytes = 0

    transcript_size = os.path.getsize(transcript_path)

    # If stenographer has covered the whole file, no gap
    if last_save_bytes >= transcript_size:
        json.dump({
            'log_path': log_path,
            'existing_notes': existing_notes,
            'gap_text': '',
            'has_gap': False,
            'stenographer_bytes': last_save_bytes,
            'transcript_bytes': transcript_size,
        }, sys.stdout)
        return

    # Extract the gap using the Claude parser
    try:
        result = claude_parser.parse_delta(
            transcript_path, last_save_bytes, transcript_size, MAX_GAP_CHARS
        )
        gap_text = result['text']
        stats = result['stats']
    except Exception as e:
        # Fallback: if parser fails, output error but don't crash
        json.dump({
            'log_path': log_path,
            'existing_notes': existing_notes,
            'gap_text': '',
            'has_gap': False,
            'error': f'Parser failed: {e}',
            'stenographer_bytes': last_save_bytes,
            'transcript_bytes': transcript_size,
        }, sys.stdout)
        return

    json.dump({
        'log_path': log_path,
        'existing_notes': existing_notes,
        'gap_text': gap_text,
        'has_gap': bool(gap_text and len(gap_text.strip()) > 200),
        'stenographer_bytes': last_save_bytes,
        'transcript_bytes': transcript_size,
        'stats': stats,
    }, sys.stdout)


if __name__ == '__main__':
    main()
