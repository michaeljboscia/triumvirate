"""
Codex JSONL transcript parser for Stenographer.

Reads Codex CLI session transcripts from JSONL files. Codex stores sessions at:
  ~/.codex/sessions/YYYY/MM/DD/rollout-<UUID>.jsonl

Event types:
  - session_meta: Session metadata (id, cwd, model, instructions)
  - turn_context: Turn metadata (model, effort, approval_policy)
  - event_msg: Events (task_started, agent_reasoning, exec_command, etc.)
  - response_item: Message payloads (developer messages, model responses, tool calls/results)

Key signal for session notes:
  - event_msg with payload.type == "agent_reasoning" — Codex's thinking
  - response_item with role == "assistant" — Codex's visible output
  - response_item with tool calls and results — what was executed
"""

import json
import os
from typing import Optional

try:
    from .claude import redact_secrets, TOOL_OUTPUT_MAX
except ImportError:
    from claude import redact_secrets, TOOL_OUTPUT_MAX


def _extract_response_item(payload: dict) -> Optional[str]:
    """Extract meaningful text from a response_item payload."""
    role = payload.get('role', '')
    msg_type = payload.get('type', '')
    blocks = payload.get('content', [])

    if not isinstance(blocks, list):
        return None

    parts = []
    for block in blocks:
        if not isinstance(block, dict):
            continue
        btype = block.get('type', '')

        if btype in ('input_text', 'output_text', 'text'):
            text = block.get('text', '').strip()
            if not text:
                continue
            # Skip system/developer instruction dumps
            if any(marker in text for marker in [
                'permissions instructions',
                'sandbox_mode',
                'approval_policy',
                'Filesystem sandboxing',
            ]):
                continue
            if len(text) > 2000:
                text = text[:2000] + '...[truncated]'
            text = redact_secrets(text)

            if role == 'assistant':
                parts.append(f"[Codex]: {text}")
            elif role == 'user':
                parts.append(f"[User]: {text}")
            # Skip developer role (system instructions)

        elif btype == 'function_call':
            name = block.get('name', 'unknown')
            args = block.get('arguments', '')
            if isinstance(args, str):
                if len(args) > 200:
                    args = args[:200] + '...'
                args = redact_secrets(args)
            parts.append(f"[Tool: {name}] {args}")

        elif btype == 'function_call_output':
            output = block.get('output', '')
            if isinstance(output, str):
                if len(output) > TOOL_OUTPUT_MAX:
                    output = output[:TOOL_OUTPUT_MAX] + '...[truncated]'
                output = redact_secrets(output)
                parts.append(f"[Result] {output}")

    return '\n'.join(parts) if parts else None


def _extract_event_msg(payload: dict) -> Optional[str]:
    """Extract meaningful info from event_msg payloads."""
    etype = payload.get('type', '')

    if etype == 'agent_reasoning':
        # This is Codex's thinking — primary signal
        summary = payload.get('summary', payload.get('reasoning', ''))
        if isinstance(summary, dict):
            summary = summary.get('summary', summary.get('text', str(summary)))
        if summary:
            if len(summary) > 3000:
                summary = summary[:3000] + '...[truncated]'
            summary = redact_secrets(summary)
            return f"[Codex Thinking]: {summary}"

    elif etype == 'exec_command':
        cmd = payload.get('command', '')
        if isinstance(cmd, list):
            cmd = ' '.join(cmd)
        if len(cmd) > 200:
            cmd = cmd[:200] + '...'
        cmd = redact_secrets(cmd)
        return f"[Codex Exec]: {cmd}"

    elif etype == 'exec_result':
        exit_code = payload.get('exit_code', 0)
        stdout = payload.get('stdout', '')
        stderr = payload.get('stderr', '')
        output = stdout or stderr
        if isinstance(output, str):
            if len(output) > TOOL_OUTPUT_MAX:
                output = output[:TOOL_OUTPUT_MAX] + '...[truncated]'
            output = redact_secrets(output)
        if exit_code != 0:
            return f"[Codex Result] (exit {exit_code}) {output}"
        elif output:
            return f"[Codex Result] {output}"

    elif etype == 'task_completed':
        return "[Codex: Task completed]"

    elif etype == 'task_started':
        return "[Codex: Task started]"

    return None


def parse_delta(
    transcript_path: str,
    start_byte: int,
    end_byte: int,
    max_chars: int = 120000,
) -> dict:
    """
    Parse a byte range of a Codex JSONL transcript.

    Returns same structure as claude parser for compatibility.
    """
    output_lines = []
    stats = {
        'lines_parsed': 0,
        'lines_skipped': 0,
        'parse_errors': 0,
        'thinking_blocks': 0,
        'tool_calls': 0,
        'user_messages': 0,
        'chars_emitted': 0,
    }
    chars_emitted = 0

    with open(transcript_path, 'rb') as f:
        f.seek(start_byte)

        # Only skip if we're mid-line (not at a clean newline boundary).
        if start_byte > 0:
            f.seek(start_byte - 1)
            prev_byte = f.read(1)
            if prev_byte != b'\n':
                f.readline()  # discard partial line

        while f.tell() < end_byte and chars_emitted < max_chars:
            raw_line = f.readline()
            if not raw_line:
                break

            line = raw_line.decode('utf-8', errors='replace').strip()
            if not line:
                continue

            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                stats['parse_errors'] += 1
                continue

            stats['lines_parsed'] += 1
            etype = event.get('type', '')
            payload = event.get('payload', {})

            if etype == 'response_item':
                text = _extract_response_item(payload)
                if text:
                    output_lines.append(text)
                    chars_emitted += len(text)
                    if payload.get('role') == 'user':
                        stats['user_messages'] += 1
                    elif any(b.get('type') == 'function_call' for b in payload.get('content', []) if isinstance(b, dict)):
                        stats['tool_calls'] += 1
                else:
                    stats['lines_skipped'] += 1

            elif etype == 'event_msg':
                text = _extract_event_msg(payload)
                if text:
                    output_lines.append(text)
                    chars_emitted += len(text)
                    if payload.get('type') == 'agent_reasoning':
                        stats['thinking_blocks'] += 1
                else:
                    stats['lines_skipped'] += 1

            elif etype in ('session_meta', 'turn_context'):
                stats['lines_skipped'] += 1

            else:
                stats['lines_skipped'] += 1

    stats['chars_emitted'] = chars_emitted
    return {
        'text': '\n'.join(output_lines),
        'stats': stats,
    }


def find_latest_session() -> Optional[str]:
    """Find the most recent Codex session file."""
    import glob
    base = os.path.expanduser('~/.codex/sessions')
    pattern = os.path.join(base, '*', '*', '*', 'rollout-*.jsonl')
    files = glob.glob(pattern)
    if files:
        files.sort(key=os.path.getmtime, reverse=True)
        return files[0]
    return None


if __name__ == '__main__':
    import sys
    path = sys.argv[1] if len(sys.argv) > 1 else find_latest_session()
    if not path:
        print("No Codex session found")
        exit(1)
    print(f"Parsing: {path}")
    file_size = os.path.getsize(path)
    result = parse_delta(path, 0, file_size)
    print(f"\n--- Stats ---")
    for k, v in result['stats'].items():
        print(f"  {k}: {v}")
    print(f"\n--- First 2000 chars ---")
    print(result['text'][:2000])
