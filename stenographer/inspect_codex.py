#!/usr/bin/env python3
"""Inspect Codex JSONL transcript structure."""
import json, os

codex_file = '/Users/mikeboscia/.codex/sessions/2026/03/01/rollout-2026-03-01T09-14-03-019ca9bf-c5e9-7962-939c-e468e1144ec9.jsonl'
if not os.path.exists(codex_file):
    print(f"File not found: {codex_file}")
    exit(1)

print(f"Size: {os.path.getsize(codex_file)} bytes")

# Read all event types
types_seen = set()
event_count = 0
sample_events = {}

with open(codex_file) as f:
    for line in f:
        try:
            obj = json.loads(line.strip())
            event_count += 1
            etype = obj.get('type', obj.get('event', 'MISSING'))
            types_seen.add(str(etype))
            if str(etype) not in sample_events:
                sample = json.dumps(obj, indent=2)
                sample_events[str(etype)] = sample[:1500]
        except:
            pass

print(f"Events: {event_count}")
print(f"Event types: {sorted(types_seen)}")

for etype, sample in sorted(sample_events.items()):
    print(f"\n=== {etype} ===")
    print(sample)
