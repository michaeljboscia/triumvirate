"""
Gemini JSON transcript parser for Stenographer.

Reads Gemini CLI session transcripts from JSON files. Gemini stores sessions
as a single JSON dict with a `messages` array. Each message has:
  - id, timestamp, type ('user' or 'gemini'), content
  - thoughts[] (on 'gemini' messages) — array of {subject, description, timestamp}

The thoughts[].description is the PRIMARY signal for session notes — Gemini's
visible content is often terse while the thinking contains the real reasoning.

Cursor: message index (not byte offset) because the file is a single JSON object
that must be loaded in full.
"""

import json
import os
from typing import Optional

# Reuse redaction from claude parser
try:
    from .claude import redact_secrets, TOOL_OUTPUT_MAX
except ImportError:
    from claude import redact_secrets, TOOL_OUTPUT_MAX


def _extract_gemini_content(msg: dict) -> str:
    """Extract visible text content from a Gemini message."""
    raw = msg.get('content', '')

    # content can be string, list of parts, or list of dicts
    if isinstance(raw, str):
        return raw.strip()
    elif isinstance(raw, list):
        texts = []
        for part in raw:
            if isinstance(part, str):
                texts.append(part)
            elif isinstance(part, dict):
                # Could be {text: "..."} or {type: "text", text: "..."}
                text = part.get('text', '')
                if text:
                    texts.append(text)
        return '\n'.join(texts).strip()
    return ''


def _extract_thoughts(msg: dict) -> list:
    """Extract thought entries from a Gemini message."""
    thoughts = msg.get('thoughts', [])
    if not isinstance(thoughts, list):
        return []
    return thoughts


def parse_delta(
    transcript_path: str,
    start_index: int,
    end_index: Optional[int] = None,
    max_chars: int = 120000,
) -> dict:
    """
    Parse a range of messages from a Gemini session JSON file.

    Args:
        transcript_path: Path to session-*.json file
        start_index: First message index to process (0-based)
        end_index: Last message index (exclusive). None = all remaining.
        max_chars: Maximum characters to emit

    Returns:
        {
            "text": str,
            "stats": {
                "messages_parsed": int,
                "thoughts_extracted": int,
                "user_messages": int,
                "model_messages": int,
                "chars_emitted": int,
            },
            "total_messages": int,
        }
    """
    output_lines = []
    stats = {
        'messages_parsed': 0,
        'thoughts_extracted': 0,
        'user_messages': 0,
        'model_messages': 0,
        'chars_emitted': 0,
    }
    chars_emitted = 0

    with open(transcript_path) as f:
        data = json.load(f)

    messages = data.get('messages', [])
    total = len(messages)

    if end_index is None:
        end_index = total

    for i in range(start_index, min(end_index, total)):
        if chars_emitted >= max_chars:
            break

        msg = messages[i]
        msg_type = msg.get('type', '')
        stats['messages_parsed'] += 1

        if msg_type == 'user':
            text = _extract_gemini_content(msg)
            if text:
                # Truncate very long user messages
                if len(text) > 1000:
                    text = text[:1000] + '...[truncated]'
                text = redact_secrets(text)
                entry = f"[User]: {text}"
                output_lines.append(entry)
                chars_emitted += len(entry)
                stats['user_messages'] += 1

        elif msg_type == 'gemini':
            stats['model_messages'] += 1

            # Thoughts are the PRIMARY signal
            thoughts = _extract_thoughts(msg)
            for thought in thoughts:
                subject = thought.get('subject', '')
                description = thought.get('description', '')
                if description:
                    if len(description) > 3000:
                        description = description[:3000] + '...[truncated]'
                    description = redact_secrets(description)
                    entry = f"[Gemini Thinking ({subject})]: {description}"
                    output_lines.append(entry)
                    chars_emitted += len(entry)
                    stats['thoughts_extracted'] += 1

            # Visible content (often terse but still useful)
            visible = _extract_gemini_content(msg)
            if visible:
                if len(visible) > 2000:
                    visible = visible[:2000] + '...[truncated]'
                visible = redact_secrets(visible)
                entry = f"[Gemini]: {visible}"
                output_lines.append(entry)
                chars_emitted += len(entry)

    stats['chars_emitted'] = chars_emitted
    return {
        'text': '\n'.join(output_lines),
        'stats': stats,
        'total_messages': total,
    }


def find_latest_session(project_dir: Optional[str] = None) -> Optional[str]:
    """Find the most recent Gemini session file."""
    import glob

    search_dir = os.path.expanduser('~/.gemini/tmp')
    if project_dir:
        # Try project-specific first
        pattern = os.path.join(search_dir, project_dir, 'chats', 'session-*.json')
        files = glob.glob(pattern)
        if files:
            files.sort(key=os.path.getmtime, reverse=True)
            return files[0]

    # Fall back to most recent across all projects
    pattern = os.path.join(search_dir, '*', 'chats', 'session-*.json')
    files = glob.glob(pattern)
    if files:
        files.sort(key=os.path.getmtime, reverse=True)
        return files[0]

    return None


if __name__ == '__main__':
    import sys
    path = sys.argv[1] if len(sys.argv) > 1 else find_latest_session()
    if not path:
        print("No Gemini session found")
        exit(1)
    print(f"Parsing: {path}")
    result = parse_delta(path, 0)
    print(f"\n--- Stats ---")
    for k, v in result['stats'].items():
        print(f"  {k}: {v}")
    print(f"  total_messages: {result['total_messages']}")
    print(f"\n--- First 2000 chars ---")
    print(result['text'][:2000])
