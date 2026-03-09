#!/usr/bin/env python3
"""Set up state for stenographer live test — process last 100KB of transcript."""
import json, os

state_file = os.path.expanduser('~/.triumvirate/stenographer-state.json')
transcript = '/Users/mikeboscia/.claude/projects/-Users-mikeboscia/7cd83b80-ca38-4b6c-979c-0c86105f0f35.jsonl'
fsize = os.path.getsize(transcript)
start_from = max(0, fsize - 100000)

state = {
    'sessions': {
        'claude': {
            'active_transcript': transcript,
            'last_save_bytes': start_from,
            'last_message_index': 0,
            'last_save_time': 0,
            'saves_count': 0,
            'session_log_path': None,
        }
    }
}

with open(state_file, 'w') as f:
    json.dump(state, f, indent=2)

print(f'State set: start_from={start_from}, file_size={fsize}, delta={fsize - start_from} bytes')
