#!/usr/bin/env python3
"""Inspect Gemini transcript structure."""
import json, glob, os

# Find most recent Gemini session file
gemini_dirs = glob.glob(os.path.expanduser('~/.gemini/tmp/*/chats/session-*.json'))
if not gemini_dirs:
    print("No Gemini session files found")
    exit(0)

# Sort by mtime
gemini_dirs.sort(key=os.path.getmtime, reverse=True)
latest = gemini_dirs[0]
print(f"Latest Gemini session: {latest}")
print(f"Size: {os.path.getsize(latest)} bytes")

with open(latest) as f:
    data = json.load(f)

print(f"Type: {type(data).__name__}")
if isinstance(data, list):
    print(f"Length: {len(data)} messages")
    if len(data) > 0:
        print(f"\n--- First message keys ---")
        print(json.dumps(list(data[0].keys()), indent=2))
        print(f"\n--- First message (truncated) ---")
        first = json.dumps(data[0], indent=2)
        print(first[:2000])
        # Find a message with thoughts
        for i, msg in enumerate(data):
            if 'thoughts' in msg and msg['thoughts']:
                print(f"\n--- Message {i} has thoughts ---")
                thought_sample = json.dumps(msg['thoughts'][0], indent=2) if msg['thoughts'] else 'empty'
                print(f"Thought sample: {thought_sample[:1000]}")
                break
elif isinstance(data, dict):
    print(f"Keys: {list(data.keys())}")
    print(json.dumps(data, indent=2)[:2000])
