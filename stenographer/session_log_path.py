#!/usr/bin/env python3
"""
Session Log Path — Single Source of Truth

Every component that needs to find or create a session log calls this module.
No more four implementations of the same logic with four different answers.

Used by:
    - stenographer.py (incremental saves)
    - pre-compact.sh (gap-fill at compaction, via CLI mode)
    - post-compact-recovery.sh (find log to inject, via CLI mode)
    - /session-notes skill (manual saves, via CLI mode)

Design principles:
    - ONE file per transcript (rolling appends, not versioned files)
    - File named with transcript UUID (unique, no version collision)
    - State tracked per-transcript, not per-agent
    - HOME sessions route to ~/.claude/session-logs/
    - Project sessions route to $AI_MEMORY_DIR/<repo>/
    - CLI mode for bash callers: python3 session_log_path.py --find /path/to/project

State file: ~/.triumvirate/session-log-state.json
    {
        "<transcript_uuid>": {
            "log_path": "/full/path/to/session-log.md",
            "created_at": "2026-03-09T23:00:00",
            "last_append_at": "2026-03-09T23:15:00",
            "agent": "claude",
            "repo": "claude-config",
            "feature": "hooks-system"
        }
    }
"""

import json
import os
import subprocess
import sys
from datetime import datetime
from pathlib import Path
from typing import Optional

try:
    from zoneinfo import ZoneInfo
    EASTERN = ZoneInfo('America/New_York')
except ImportError:
    EASTERN = None

STATE_FILE = Path.home() / '.triumvirate' / 'session-log-state.json'

# Maximum number of state entries before pruning old ones
MAX_STATE_ENTRIES = 50


def _now_eastern() -> datetime:
    if EASTERN:
        return datetime.now(EASTERN)
    return datetime.now()


def _now_iso() -> str:
    return _now_eastern().strftime('%Y-%m-%dT%H:%M:%S')


# ─── Taxonomy ────────────────────────────────────────────────────────────────

def _read_taxonomy(project_dir: str) -> dict:
    """Read taxonomy from .claude/taxonomy.json, walking up to git root.

    HOME directory gets bootstrap taxonomy — never inherits from ~/.claude's
    taxonomy.json (which describes the claude-config repo, not HOME sessions).
    """
    if _is_home_dir(project_dir):
        return {
            'owner': os.environ.get('USER', 'unknown'),
            'client': 'home',
            'domain': 'bootstrap',
            'repo': 'home',
            'feature': 'unsorted',
        }

    # Try project_dir first
    taxonomy_file = Path(project_dir) / '.claude' / 'taxonomy.json'
    if taxonomy_file.exists():
        try:
            with open(taxonomy_file) as f:
                return json.load(f)
        except (json.JSONDecodeError, IOError):
            pass

    # Try git root
    try:
        result = subprocess.run(
            ['git', '-C', project_dir, 'rev-parse', '--show-toplevel'],
            capture_output=True, text=True, timeout=5
        )
        if result.returncode == 0:
            git_root = result.stdout.strip()
            taxonomy_file = Path(git_root) / '.claude' / 'taxonomy.json'
            if taxonomy_file.exists():
                try:
                    with open(taxonomy_file) as f:
                        return json.load(f)
                except (json.JSONDecodeError, IOError):
                    pass
    except Exception:
        pass

    return {}


def _get_repo_name(project_dir: str) -> str:
    """Get repo name from taxonomy, git remote, or directory basename."""
    taxonomy = _read_taxonomy(project_dir)
    if taxonomy.get('repo'):
        return taxonomy['repo']

    try:
        result = subprocess.run(
            ['git', '-C', project_dir, 'remote', 'get-url', 'origin'],
            capture_output=True, text=True, timeout=5
        )
        if result.returncode == 0:
            url = result.stdout.strip()
            repo = url.split('/')[-1].replace('.git', '')
            if repo:
                return repo
    except Exception:
        pass

    return Path(project_dir).name


def _get_owner(project_dir: str) -> str:
    """Get owner from taxonomy or git config or whoami."""
    taxonomy = _read_taxonomy(project_dir)
    if taxonomy.get('owner'):
        return taxonomy['owner']

    try:
        result = subprocess.run(
            ['git', '-C', project_dir, 'config', 'user.name'],
            capture_output=True, text=True, timeout=5
        )
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip().replace(' ', '-').lower()
    except Exception:
        pass

    return os.environ.get('USER', 'unknown')


def _get_feature(project_dir: str) -> str:
    """Get feature from taxonomy or git branch."""
    taxonomy = _read_taxonomy(project_dir)
    if taxonomy.get('feature'):
        return taxonomy['feature']

    try:
        result = subprocess.run(
            ['git', '-C', project_dir, 'branch', '--show-current'],
            capture_output=True, text=True, timeout=5
        )
        if result.returncode == 0 and result.stdout.strip():
            branch = result.stdout.strip()
            if branch not in ('main', 'master'):
                return branch
    except Exception:
        pass

    return 'general'


# ─── Session Log Directory ───────────────────────────────────────────────────

def _is_home_dir(project_dir: str) -> bool:
    """Check if project_dir is the user's HOME directory."""
    home = str(Path.home())
    return os.path.realpath(project_dir) in (home, home + '/')


def get_session_log_dir(project_dir: str) -> Path:
    """
    Determine the correct directory for session logs.

    HOME → ~/.claude/session-logs/
    Projects → $AI_MEMORY_DIR/<repo>/ (default: ~/.ai-memory/<repo>/)
    """
    if _is_home_dir(project_dir):
        log_dir = Path.home() / '.claude' / 'session-logs'
    else:
        ai_memory = os.environ.get('AI_MEMORY_DIR', str(Path.home() / '.ai-memory'))
        repo = _get_repo_name(project_dir)
        log_dir = Path(ai_memory) / repo

    log_dir.mkdir(parents=True, exist_ok=True)
    return log_dir


# ─── State Management ────────────────────────────────────────────────────────

def _load_state() -> dict:
    if not STATE_FILE.exists():
        return {}
    try:
        with open(STATE_FILE) as f:
            return json.load(f)
    except (json.JSONDecodeError, IOError):
        return {}


def _save_state(state: dict):
    STATE_FILE.parent.mkdir(parents=True, exist_ok=True)

    # Prune old entries if too many
    if len(state) > MAX_STATE_ENTRIES:
        # Sort by last_append_at, keep newest
        sorted_keys = sorted(
            state.keys(),
            key=lambda k: state[k].get('last_append_at', ''),
            reverse=True
        )
        state = {k: state[k] for k in sorted_keys[:MAX_STATE_ENTRIES]}

    tmp = str(STATE_FILE) + '.tmp'
    with open(tmp, 'w') as f:
        json.dump(state, f, indent=2)
    os.rename(tmp, str(STATE_FILE))


def _transcript_uuid(transcript_path: str) -> str:
    """Extract UUID from transcript path (filename without .jsonl)."""
    return Path(transcript_path).stem


# ─── Core: Find or Create Session Log ────────────────────────────────────────

def find_session_log(project_dir: str, transcript_path: str = None,
                     agent: str = 'claude') -> Optional[str]:
    """
    Find the current session log for a project.

    Resolution order:
    1. State file (if transcript_path provided and has a known log)
    2. Latest rolling session log in the correct directory
    3. None (caller should use get_or_create_session_log instead)
    """
    # Check state first (fastest, most accurate)
    if transcript_path:
        uuid = _transcript_uuid(transcript_path)
        state = _load_state()
        entry = state.get(uuid)
        if entry and os.path.exists(entry.get('log_path', '')):
            return entry['log_path']

    # Look for existing rolling logs in the correct directory
    log_dir = get_session_log_dir(project_dir)
    repo = _get_repo_name(project_dir)

    # Rolling logs use transcript UUID in filename — find any
    # Also check legacy _v*.md format for migration
    candidates = []

    # New format: <taxonomy>_<date>_<uuid>_<agent>.md
    for f in log_dir.glob(f'*_{agent}.md'):
        candidates.append(f)

    # Legacy format: *--*_v*_<agent>.md
    for f in log_dir.glob(f'*--*_v*_{agent}.md'):
        if f not in candidates:
            candidates.append(f)

    if candidates:
        # Return most recently modified
        candidates.sort(key=lambda p: p.stat().st_mtime, reverse=True)
        return str(candidates[0])

    return None


def get_or_create_session_log(project_dir: str, transcript_path: str,
                               agent: str = 'claude') -> str:
    """
    Get the session log for this transcript, creating if needed.

    This is THE function everything calls. It guarantees:
    - Same transcript → same log file (always)
    - Different transcript → different log file (always)
    - Rolling appends within a session (never versioned files)
    """
    uuid = _transcript_uuid(transcript_path)
    state = _load_state()

    # Already tracked?
    entry = state.get(uuid)
    if entry and os.path.exists(entry.get('log_path', '')):
        return entry['log_path']

    # Create new session log
    log_dir = get_session_log_dir(project_dir)
    repo = _get_repo_name(project_dir)
    owner = _get_owner(project_dir)
    feature = _get_feature(project_dir)
    taxonomy = _read_taxonomy(project_dir)
    client = taxonomy.get('client', 'unknown')
    domain = taxonomy.get('domain', 'unknown')

    date_str = _now_eastern().strftime('%Y%m%d')
    short_uuid = uuid[:12]

    # Filename: <owner>--<client>_<domain>_<repo>_<feature>_<date>_<uuid>_<agent>.md
    filename = f"{owner}--{client}_{domain}_{repo}_{feature}_{date_str}_{short_uuid}_{agent}.md"
    log_path = log_dir / filename

    # Write header
    now = _now_eastern()
    header = f"""# Session Log: {owner}/{client}/{domain}/{repo}

**Feature:** {feature}
**Session Started:** {now.strftime('%Y-%m-%d %H:%M:%S %Z')}
**Transcript:** `{uuid}`
**Agent:** {agent}
**Generated by:** Session Notes v2 (Stenographer + Gap-Fill)

---

## TAXONOMY

| Level | Value |
|-------|-------|
| Owner | {owner} |
| Client | {client} |
| Domain | {domain} |
| Repo | {repo} |
| Feature | {feature} |
| Transcript | `{uuid}` |

---
"""
    with open(log_path, 'w') as f:
        f.write(header)

    # Track in state
    state[uuid] = {
        'log_path': str(log_path),
        'created_at': _now_iso(),
        'last_append_at': _now_iso(),
        'agent': agent,
        'repo': repo,
        'feature': feature,
    }
    _save_state(state)

    return str(log_path)


def update_append_time(transcript_path: str):
    """Mark that we just appended to this transcript's log."""
    uuid = _transcript_uuid(transcript_path)
    state = _load_state()
    if uuid in state:
        state[uuid]['last_append_at'] = _now_iso()
        _save_state(state)


def find_latest_for_recovery(project_dir: str, agent: str = 'claude') -> Optional[str]:
    """
    Find the most recent session log for post-compaction recovery.

    Unlike find_session_log, this doesn't need a transcript_path.
    Used by post-compact-recovery.sh when it needs to find what
    pre-compact.sh (or stenographer) just wrote.

    Resolution:
    1. Most recent entry in state file for this repo
    2. Most recent file in the correct log directory
    """
    repo = _get_repo_name(project_dir)

    # Check state for this repo's most recent log
    state = _load_state()
    repo_entries = [
        (uuid, entry) for uuid, entry in state.items()
        if entry.get('repo') == repo and entry.get('agent') == agent
    ]
    if repo_entries:
        # Sort by last_append_at, newest first
        repo_entries.sort(key=lambda x: x[1].get('last_append_at', ''), reverse=True)
        latest = repo_entries[0][1]
        if os.path.exists(latest.get('log_path', '')):
            return latest['log_path']

    # Fallback: glob the directory
    log_dir = get_session_log_dir(project_dir)
    candidates = sorted(
        log_dir.glob(f'*_{agent}.md'),
        key=lambda p: p.stat().st_mtime,
        reverse=True
    )
    if candidates:
        return str(candidates[0])

    # Legacy fallback
    candidates = sorted(
        log_dir.glob(f'*--*_v*_{agent}.md'),
        key=lambda p: p.stat().st_mtime,
        reverse=True
    )
    if candidates:
        return str(candidates[0])

    return None


# ─── CLI Mode (for bash callers) ─────────────────────────────────────────────

def main():
    """
    CLI interface for bash hooks.

    Usage:
        # Find current log (returns path or empty):
        python3 session_log_path.py --find /path/to/project

        # Find or create log for a transcript:
        python3 session_log_path.py --get /path/to/project --transcript /path/to/uuid.jsonl

        # Find latest for recovery (no transcript needed):
        python3 session_log_path.py --recover /path/to/project

        # Get the log directory:
        python3 session_log_path.py --dir /path/to/project

        # All commands accept --agent (default: claude)
    """
    import argparse

    parser = argparse.ArgumentParser(description='Session Log Path — Single Source of Truth')
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument('--find', metavar='PROJECT_DIR',
                       help='Find current session log (returns path or empty)')
    group.add_argument('--get', metavar='PROJECT_DIR',
                       help='Find or create session log (requires --transcript)')
    group.add_argument('--recover', metavar='PROJECT_DIR',
                       help='Find latest log for post-compaction recovery')
    group.add_argument('--dir', metavar='PROJECT_DIR',
                       help='Print the session log directory path')

    parser.add_argument('--transcript', help='Path to transcript file')
    parser.add_argument('--agent', default='claude', help='Agent name (default: claude)')

    args = parser.parse_args()

    # Source .env for AI_MEMORY_DIR
    env_file = Path.home() / '.claude' / '.env'
    if env_file.exists():
        try:
            with open(env_file) as f:
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

    if args.find:
        result = find_session_log(args.find, args.transcript, args.agent)
        if result:
            print(result)

    elif args.get:
        if not args.transcript:
            print("ERROR: --get requires --transcript", file=sys.stderr)
            sys.exit(1)
        result = get_or_create_session_log(args.get, args.transcript, args.agent)
        print(result)

    elif args.recover:
        result = find_latest_for_recovery(args.recover, args.agent)
        if result:
            print(result)

    elif args.dir:
        result = get_session_log_dir(args.dir)
        print(result)


if __name__ == '__main__':
    main()
