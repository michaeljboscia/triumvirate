#!/usr/bin/env bash
set -euo pipefail

printf '{"type":"thread.started","thread_id":"mock-codex-thread"}\n'
sleep 0.05
printf '{"type":"turn.started"}\n'
sleep 0.05
printf '{"type":"item.started","item":{"id":"i1","type":"command_execution","command":"echo hi","status":"in_progress"}}\n'
sleep 0.05
printf '{"type":"item.completed","item":{"id":"i1","type":"command_execution","command":"echo hi","exit_code":0,"status":"completed"}}\n'
printf '{"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"done"}}\n'
printf '{"type":"turn.completed","usage":{"input_tokens":200,"cached_input_tokens":50,"output_tokens":30}}\n'
