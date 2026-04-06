#!/usr/bin/env bash
set -euo pipefail

printf '{"type":"init","session_id":"mock-gemini-session","model":"gemini-mock"}\n'
sleep 0.05
printf '{"type":"tool_use","tool_name":"read_file","tool_id":"t1","parameters":{"file_path":"README.md"}}\n'
sleep 0.05
printf '{"type":"tool_result","tool_id":"t1","status":"success","output":""}\n'
sleep 0.05
printf '{"type":"message","role":"assistant","content":"done","delta":true}\n'
printf '{"type":"result","status":"success","stats":{"input_tokens":100,"output_tokens":20,"cached":10,"total_tokens":130}}\n'
