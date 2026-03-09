#!/usr/bin/env python3
"""Inspect Claude JSONL transcript structure for parser development."""
import json
import sys
import os

TRANSCRIPT = '/Users/mikeboscia/.claude/projects/-Users-mikeboscia/7cd83b80-ca38-4b6c-979c-0c86105f0f35.jsonl'

def inspect():
    print("=== TOOL RESULT STRUCTURE ===")
    found_result = False
    found_user_str = 0
    found_user_arr = 0

    with open(TRANSCRIPT) as f:
        for line in f:
            try:
                obj = json.loads(line.strip())
            except:
                continue

            # Find tool_result blocks
            if not found_result and obj.get('type') == 'user' and 'message' in obj:
                msg = obj['message']
                body = msg.get('content', msg.get('text', ''))
                if isinstance(body, list):
                    for block in body:
                        if isinstance(block, dict) and block.get('type') == 'tool_result':
                            # Truncate for display
                            display = {}
                            for k, v in block.items():
                                if k == 'content' and isinstance(v, list):
                                    trunc_list = []
                                    for c in v:
                                        if isinstance(c, dict) and 'text' in c:
                                            tc = dict(c)
                                            tc['text'] = tc['text'][:300] + '...[TRUNC]' if len(tc['text']) > 300 else tc['text']
                                            trunc_list.append(tc)
                                        else:
                                            trunc_list.append(c)
                                    display[k] = trunc_list
                                elif k == 'content' and isinstance(v, str) and len(v) > 300:
                                    display[k] = v[:300] + '...[TRUNC]'
                                else:
                                    display[k] = v
                            print(json.dumps(display, indent=2)[:2000])
                            found_result = True
                            break

            # Find user message formats
            if obj.get('type') == 'user' and 'message' in obj:
                body = obj['message'].get('content', obj['message'].get('text', ''))
                if isinstance(body, str) and len(body) > 5 and found_user_str < 3:
                    print(f"\n=== USER MSG (string) #{found_user_str+1} ===")
                    print(body[:200])
                    found_user_str += 1
                elif isinstance(body, list) and found_user_arr < 2:
                    types = [b.get('type','?') for b in body if isinstance(b, dict)]
                    print(f"\n=== USER MSG (array) #{found_user_arr+1} ===")
                    print(f"Block types: {types}")
                    found_user_arr += 1

    # Check for text blocks in assistant messages
    print("\n=== ASSISTANT TEXT BLOCK EXAMPLE ===")
    with open(TRANSCRIPT) as f:
        for line in f:
            try:
                obj = json.loads(line.strip())
            except:
                continue
            if obj.get('type') == 'assistant' and 'message' in obj:
                for block in obj['message'].get('content', []):
                    if isinstance(block, dict) and block.get('type') == 'text' and len(block.get('text', '')) > 50:
                        print(json.dumps(block, indent=2)[:1000])
                        return

if __name__ == '__main__':
    inspect()
