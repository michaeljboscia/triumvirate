"""
Claude JSONL transcript parser for Stenographer.

Reads Claude Code JSONL transcripts by byte range, extracting high-signal
events into normalized text suitable for LLM summarization.

Transcript format: One JSON object per line.
Top-level event types: assistant, user, progress, system, file-history-snapshot,
                        custom-title, queue-operation

Assistant content blocks: thinking, text, tool_use
User content: string (plain text) or array (tool_result blocks)
"""

import json
import re
import os
from typing import Optional

# Regex patterns for secret redaction
SECRET_PATTERNS = [
    re.compile(r'sk-ant-api\S+'),                          # Anthropic keys
    re.compile(r'sk-[a-zA-Z0-9]{20,}'),                    # OpenAI keys
    re.compile(r'AIza[a-zA-Z0-9_-]{35}'),                  # Google API keys
    re.compile(r'ghp_[a-zA-Z0-9]{36}'),                    # GitHub PATs
    re.compile(r'gho_[a-zA-Z0-9]{36}'),                    # GitHub OAuth
    re.compile(r'github_pat_[a-zA-Z0-9_]{82}'),            # GitHub fine-grained PATs
    re.compile(r'xoxb-[a-zA-Z0-9-]+'),                     # Slack bot tokens
    re.compile(r'xoxp-[a-zA-Z0-9-]+'),                     # Slack user tokens
    re.compile(r'Bearer\s+[a-zA-Z0-9._\-]{20,}'),          # Bearer tokens
    re.compile(r'-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----'), # Private keys
    re.compile(r'password\s*[=:]\s*["\']?\S{8,}', re.I),   # Password assignments
    re.compile(r'ANTHROPIC_API_KEY\s*=\s*\S+'),             # Env var assignments
    re.compile(r'OPENAI_API_KEY\s*=\s*\S+'),
    re.compile(r'GEMINI_API_KEY\s*=\s*\S+'),
    re.compile(r'DATABASE_URL\s*=\s*\S+'),
]

# Hook context dedup — skip repeated additionalContext injections


# Max chars for tool output before truncation
TOOL_OUTPUT_MAX = 500

# Max chars for a single thinking block
THINKING_MAX = 3000


def redact_secrets(text: str) -> str:
    """Replace known secret patterns with [REDACTED]."""
    for pattern in SECRET_PATTERNS:
        text = pattern.sub('[REDACTED]', text)
    return text


def _extract_tool_use_summary(block: dict) -> str:
    """Extract salient info from a tool_use block."""
    name = block.get('name', 'Unknown')
    inp = block.get('input', {})

    if name == 'Bash':
        cmd = inp.get('command', '')
        desc = inp.get('description', '')
        # Truncate very long commands
        if len(cmd) > 200:
            cmd = cmd[:200] + '...'
        label = desc if desc else cmd
        return f"[Tool: {name}] {label}"

    elif name in ('Read', 'Glob', 'Grep'):
        path = inp.get('file_path', inp.get('pattern', inp.get('path', '')))
        return f"[Tool: {name}] {path}"

    elif name in ('Edit', 'Write'):
        path = inp.get('file_path', '')
        return f"[Tool: {name}] {path}"

    elif name == 'Agent':
        desc = inp.get('description', inp.get('prompt', ''))[:150]
        atype = inp.get('subagent_type', '')
        return f"[Tool: Agent ({atype})] {desc}"

    elif name == 'Skill':
        skill = inp.get('skill', '')
        return f"[Tool: Skill] {skill}"

    elif name == 'WebFetch':
        url = inp.get('url', '')
        return f"[Tool: WebFetch] {url}"

    elif name == 'WebSearch':
        query = inp.get('query', '')
        return f"[Tool: WebSearch] {query}"

    elif name == 'ToolSearch':
        query = inp.get('query', '')
        return f"[Tool: ToolSearch] {query}"

    elif name.startswith('mcp__'):
        # MCP tool calls — show name and truncated params
        params_summary = ', '.join(f'{k}={str(v)[:60]}' for k, v in list(inp.items())[:3])
        return f"[Tool: {name}] {params_summary}"

    else:
        params_summary = ', '.join(f'{k}={str(v)[:60]}' for k, v in list(inp.items())[:3])
        return f"[Tool: {name}] {params_summary}"


def _extract_tool_result_summary(block: dict) -> str:
    """Extract salient info from a tool_result block."""
    is_error = block.get('is_error', False)
    raw = block.get('content', '')

    # content can be string or list of {type: "text", text: "..."}
    if isinstance(raw, list):
        texts = []
        for item in raw:
            if isinstance(item, dict) and 'text' in item:
                texts.append(item['text'])
        raw = '\n'.join(texts)

    if not isinstance(raw, str):
        raw = str(raw)

    text = raw.strip()

    # Detect meaningful outcomes
    prefix = "[Error] " if is_error else "[Result] "

    # Git commit hashes
    commit_match = re.search(r'\b([0-9a-f]{7,40})\b', text)
    if commit_match and any(w in text.lower() for w in ['commit', 'push', 'merge']):
        prefix += f"(commit: {commit_match.group(1)[:10]}) "

    # Test results
    if re.search(r'(\d+\s+pass|\d+\s+fail|PASS|FAIL|tests?\s+passed)', text, re.I):
        # Keep test summary lines
        for line in text.split('\n'):
            if re.search(r'(pass|fail|error|test)', line, re.I):
                return prefix + line.strip()[:TOOL_OUTPUT_MAX]

    # Truncate long output
    if len(text) > TOOL_OUTPUT_MAX:
        # Keep first meaningful lines
        lines = text.split('\n')
        kept = []
        total = 0
        for line in lines:
            if total + len(line) > TOOL_OUTPUT_MAX:
                kept.append(f'...[{len(lines) - len(kept)} more lines]')
                break
            kept.append(line)
            total += len(line)
        text = '\n'.join(kept)

    return prefix + text


def _is_hook_injection(block: dict) -> bool:
    """Check if a user message block is a hook additionalContext injection."""
    if not isinstance(block, dict):
        return False
    text = ''
    if block.get('type') == 'text':
        text = block.get('text', '')
    elif isinstance(block.get('content'), str):
        text = block['content']

    # Common hook injection patterns
    hook_markers = [
        'hookSpecificOutput',
        'additionalContext',
        'system-reminder',
        'SESSION_LOG_SPEC',
        'IRON LAW',
        'Explanatory output style is active',
    ]
    return any(marker in text for marker in hook_markers)


def parse_delta(
    transcript_path: str,
    start_byte: int,
    end_byte: int,
    max_chars: int = 120000,
) -> dict:
    """
    Parse a byte range of a Claude JSONL transcript.

    Returns:
        {
            "text": str,          # Normalized human-readable transcript
            "stats": {
                "lines_parsed": int,
                "lines_skipped": int,
                "parse_errors": int,
                "thinking_blocks": int,
                "tool_calls": int,
                "user_messages": int,
                "chars_emitted": int,
            }
        }
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
        # Check if the byte before start_byte is a newline — if so, we're
        # at a line boundary and should NOT skip.
        if start_byte > 0:
            f.seek(start_byte - 1)
            prev_byte = f.read(1)
            if prev_byte != b'\n':
                f.readline()  # discard partial line
            # else: we're at a line boundary, don't skip

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
            event_type = event.get('type', '')

            # --- USER MESSAGES ---
            if event_type == 'user' and 'message' in event:
                msg = event['message']
                body = msg.get('content', msg.get('text', ''))

                if isinstance(body, str) and body.strip():
                    # Plain user text
                    text = body.strip()
                    if len(text) > 500:
                        text = text[:500] + '...'
                    text = redact_secrets(text)
                    entry = f"[User]: {text}"
                    output_lines.append(entry)
                    chars_emitted += len(entry)
                    stats['user_messages'] += 1

                elif isinstance(body, list):
                    for block in body:
                        if not isinstance(block, dict):
                            continue
                        btype = block.get('type', '')

                        if btype == 'tool_result':
                            summary = _extract_tool_result_summary(block)
                            summary = redact_secrets(summary)
                            output_lines.append(summary)
                            chars_emitted += len(summary)

                        elif btype == 'text':
                            if _is_hook_injection(block):
                                stats['lines_skipped'] += 1
                                continue
                            text = block.get('text', '').strip()
                            if text and len(text) > 10:
                                text = redact_secrets(text[:500])
                                entry = f"[User]: {text}"
                                output_lines.append(entry)
                                chars_emitted += len(entry)
                                stats['user_messages'] += 1

            # --- ASSISTANT MESSAGES ---
            elif event_type == 'assistant' and 'message' in event:
                msg_content = event['message'].get('content', [])
                if not isinstance(msg_content, list):
                    continue

                for block in msg_content:
                    if not isinstance(block, dict):
                        continue
                    btype = block.get('type', '')

                    if btype == 'thinking':
                        thinking_text = block.get('thinking', '')
                        if thinking_text:
                            if len(thinking_text) > THINKING_MAX:
                                thinking_text = thinking_text[:THINKING_MAX] + '...[truncated]'
                            thinking_text = redact_secrets(thinking_text)
                            entry = f"[Thinking]: {thinking_text}"
                            output_lines.append(entry)
                            chars_emitted += len(entry)
                            stats['thinking_blocks'] += 1

                    elif btype == 'redacted_thinking':
                        entry = "[Thinking]: [REDACTED BY ANTHROPIC SAFETY FILTER]"
                        output_lines.append(entry)
                        chars_emitted += len(entry)
                        stats['thinking_blocks'] += 1

                    elif btype == 'text':
                        text = block.get('text', '').strip()
                        if text:
                            if len(text) > 2000:
                                text = text[:2000] + '...[truncated]'
                            text = redact_secrets(text)
                            entry = f"[Claude]: {text}"
                            output_lines.append(entry)
                            chars_emitted += len(entry)

                    elif btype == 'tool_use':
                        summary = _extract_tool_use_summary(block)
                        summary = redact_secrets(summary)
                        output_lines.append(summary)
                        chars_emitted += len(summary)
                        stats['tool_calls'] += 1

            # --- SYSTEM / PROGRESS / OTHER ---
            elif event_type == 'system':
                # System messages occasionally have useful info
                msg = event.get('message', {})
                body = msg.get('content', '')
                if isinstance(body, str) and 'error' in body.lower():
                    entry = f"[System Error]: {body[:300]}"
                    output_lines.append(entry)
                    chars_emitted += len(entry)
                else:
                    stats['lines_skipped'] += 1

            else:
                # progress, file-history-snapshot, custom-title, queue-operation
                stats['lines_skipped'] += 1

    stats['chars_emitted'] = chars_emitted
    return {
        'text': '\n'.join(output_lines),
        'stats': stats,
    }


if __name__ == '__main__':
    # Quick self-test: parse last 50KB of a transcript
    import sys
    if len(sys.argv) < 2:
        print("Usage: python claude.py <transcript.jsonl>")
        sys.exit(1)
    transcript = sys.argv[1]
    file_size = os.path.getsize(transcript)
    start = max(0, file_size - 50000)
    result = parse_delta(transcript, start, file_size)
    print(f"--- Stats ---")
    for k, v in result['stats'].items():
        print(f"  {k}: {v}")
    print(f"\n--- First 2000 chars of output ---")
    print(result['text'][:2000])
