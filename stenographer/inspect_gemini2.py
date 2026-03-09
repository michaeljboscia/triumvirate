#!/usr/bin/env python3
"""Deeper Gemini transcript inspection."""
import json, glob, os

gemini_files = glob.glob(os.path.expanduser('~/.gemini/tmp/*/chats/session-*.json'))
gemini_files.sort(key=os.path.getmtime, reverse=True)

for gf in gemini_files[:3]:
    print(f"\n=== {gf} ({os.path.getsize(gf)} bytes) ===")
    with open(gf) as f:
        data = json.load(f)

    msgs = data.get('messages', [])
    print(f"  Messages: {len(msgs)}")

    # Check message types and look for thoughts
    types_seen = set()
    thoughts_count = 0
    for msg in msgs:
        types_seen.add(msg.get('type', '?'))
        if 'thoughts' in msg:
            thoughts_count += 1

    print(f"  Message types: {sorted(types_seen)}")
    print(f"  Messages with thoughts: {thoughts_count}")

    # Show a message with thoughts if available
    for msg in msgs:
        if 'thoughts' in msg and msg['thoughts']:
            print(f"\n  --- Message with thoughts (type={msg.get('type')}) ---")
            print(f"  Keys: {list(msg.keys())}")
            thoughts = msg['thoughts']
            if isinstance(thoughts, list) and len(thoughts) > 0:
                print(f"  thoughts[0] keys: {list(thoughts[0].keys()) if isinstance(thoughts[0], dict) else type(thoughts[0])}")
                sample = json.dumps(thoughts[0], indent=2)
                print(f"  thoughts[0]: {sample[:800]}")
            break

    # Show structure of a typical model message
    for msg in msgs:
        if msg.get('type') == 'model':
            print(f"\n  --- Model message ---")
            print(f"  Keys: {list(msg.keys())}")
            # Show content structure
            c = msg.get('content', msg.get('parts', []))
            if isinstance(c, list) and len(c) > 0:
                print(f"  content[0] type: {type(c[0]).__name__}")
                if isinstance(c[0], dict):
                    print(f"  content[0] keys: {list(c[0].keys())}")
                    sample = json.dumps(c[0], indent=2)
                    print(f"  content[0]: {sample[:500]}")
                elif isinstance(c[0], str):
                    print(f"  content[0]: {c[0][:300]}")
            break
