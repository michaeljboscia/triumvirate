# TEST_PLAN — Triumvirate v2

**Version:** 2.0
**Date:** 2026-04-05
**Cross-refs:** PRD.md (FEAT-IDs), SPEC.md (REQ-IDs), IMPLEMENTATION_PLAN.md, BACKEND_STRUCTURE.md

---

## Part 1: REQ Traceability Matrix

Every REQ has a test. No orphan REQs.

| REQ-ID | Acceptance Criteria | Test Type | Pass Condition | FEAT |
|--------|-------------------|-----------|---------------|------|
| REQ-1 | N agents in browser, all talking, all live | E2E | Send message, receive responses from all 3 agents in browser within 30s | FEAT-001-004, FEAT-008, FEAT-014 |
| REQ-1 | @-mention routing works | Integration | `@claude <msg>` goes only to Claude. Verified via routing_log | FEAT-008 |
| REQ-1 | Lead agent rotation | Integration | Architecture→Claude, research→Gemini, implementation→Codex. Verified via routing_log | FEAT-008 |
| REQ-2 | Session notes, no hallucination | E2E | 5-turn conversation. Log contains all messages traceable to fabric IDs, git diff, tool calls | FEAT-011 |
| REQ-2 | File changes tracked | Integration | Create file during session. Stenographer includes it in files_modified | FEAT-011 |
| REQ-3 | Memory shared across sessions | E2E | Session 1: decision made. Session 2: agent prompt contains it | FEAT-012 |
| REQ-3 | Memory write failure is loud | Integration | Simulate write failure. Dashboard red banner. No silent skip | FEAT-012 |
| REQ-3 | Decision extraction and confirmation | Integration | Agent outputs decision. Dashboard shows prompt. Approve→persisted. Reject→not | FEAT-013 |
| REQ-4 | Single binary, no external deps | Unit | Binary boots without NATS/Temporal/Docker. No unexpected network connections | FEAT-026 |
| REQ-4 | Startup under 5 seconds | Unit | Process start to "Ready" log < 5s | FEAT-026 |
| REQ-4 | Failure modes visible | Integration | Kill agent. Dashboard shows "restarting..." within 5s | FEAT-023 |
| REQ-4 | Crash recovery | E2E | Kill -9 daemon mid-workflow. Restart. Resume from last step | FEAT-024 |
| REQ-5 | Idle agents receive digests | Integration | Claude responds. Gemini gets mechanical digest. Verified via stdin log | FEAT-018 |
| REQ-5 | Quota auto-fallback | Integration | Simulate 80%. Digests stop. Only @-mentions reach agent | FEAT-017 |
| REQ-5 | Digest scoped to task | Integration | Fleet with 2 tasks. Task A agent doesn't get task B digest | FEAT-018 |
| REQ-6 | Dashboard serves at :8080 | Unit | curl / returns HTML. curl /api/health returns JSON ok | FEAT-014 |
| REQ-6 | Tasks view shows active tasks | E2E | Start conversation. Task appears with assigned agent, real-time status | FEAT-015 |
| REQ-6 | Agents view dynamic grid | E2E | 3 agents: 2×2. 7 agents: dynamic layout | FEAT-016 |
| REQ-6 | 30-second comprehension | Manual | Unfamiliar person describes what dashboard does | FEAT-014 |
| REQ-7 | Fleet spawns N instances | E2E | Request {claude: 2, codex: 1}. Three instances. Verified via /api/agents | FEAT-001, FEAT-010 |
| REQ-7 | Worktrees created | Integration | Fleet spawns. git worktree list shows N+1. Each agent cwd is its worktree | FEAT-019 |
| REQ-7 | Shared task list with deps | Integration | Task B depends on A. Claim B before A done → stays blocked. A done → B unblocks | FEAT-020 |
| REQ-7 | Sequential merge | Integration | Merge first branch → success. Second → conflict surfaces in dashboard. Never parallel | FEAT-021 |
| REQ-7 | Contracts first | E2E | Fleet spawn → contract step before fan-out. Agents implement against interfaces | FEAT-010 |
| REQ-7 | Dynamic composition | Integration | Fleet 1: 3 claude + 2 codex. Fleet 2: 5 codex + 1 claude. Both work | FEAT-001 |
| REQ-7 | Provider abstraction | Unit | backend="api" → API crate used. Same trait interface | FEAT-005 |

---

## Part 2: Unit Tests by Module

### proto crate (daemon/crates/proto/)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_fabric_message_serialize` | FabricMessage round-trips through serde | Create message, serialize, deserialize | Fields match |
| `test_topic_key_uniqueness` | Different topics produce different keys | AgentOutput(Claude) vs AgentOutput(Gemini) | Different strings |
| `test_agent_id_display` | AgentId Display trait | AgentId::Claude | "claude" |
| `test_payload_variants_serialize` | All Payload variants serialize to JSON | Each variant | Valid JSON with `type` field |

### Claude event parser (daemon/crates/proto/src/claude_events.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_parse_init_event` | Parse stream-json init | `{"type":"init","session_id":"uuid","model":"claude-opus-4-6"}` | ClaudeEvent::Init with correct fields |
| `test_parse_message_delta` | Parse streaming message chunk | `{"type":"message","role":"assistant","content":"Hello","delta":true}` | ClaudeEvent::Message, delta=true |
| `test_parse_message_final` | Parse final message | `{"type":"message","role":"assistant","content":"Done.","delta":false}` | delta=false |
| `test_parse_tool_use` | Parse tool invocation | `{"type":"tool_use","tool":"write_file","input":{...}}` | ClaudeEvent::ToolUse with tool name |
| `test_parse_error` | Parse error event | `{"type":"error","message":"rate limited"}` | ClaudeEvent::Error |
| `test_parse_malformed_json` | Reject invalid JSON | `{not json}` | Err with descriptive message |
| `test_parse_unknown_type` | Handle unknown event type gracefully | `{"type":"unknown_future_event"}` | Ok(ClaudeEvent::Unknown) — don't crash |
| `test_parse_missing_fields` | Handle missing required fields | `{"type":"message"}` (no content) | Err with field name |
| `test_parse_empty_line` | Handle empty JSONL lines | `""` or `"\n"` | Skip, no error |

### Gemini ACP parser (daemon/crates/proto/src/gemini_events.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_parse_jsonrpc_response` | Parse valid JSON-RPC response | `{"jsonrpc":"2.0","result":{...},"id":1}` | GeminiEvent::Response with content |
| `test_parse_jsonrpc_error` | Parse JSON-RPC error | `{"jsonrpc":"2.0","error":{"code":-32600},"id":1}` | GeminiEvent::Error |
| `test_parse_malformed_jsonrpc` | Missing jsonrpc field | `{"result":"hello"}` | Err |
| `test_parse_notification` | Parse notification (no id) | `{"jsonrpc":"2.0","method":"progress","params":{}}` | GeminiEvent::Notification |

### Codex MCP parser (daemon/crates/proto/src/codex_events.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_parse_thread_started` | Parse thread.started | `{"type":"thread.started","thread_id":"uuid"}` | CodexEvent::ThreadStarted |
| `test_parse_turn_completed` | Parse turn.completed with usage | `{"type":"turn.completed","usage":{...}}` | CodexEvent::TurnCompleted with token counts |
| `test_parse_item_agent_message` | Parse agent_message item | `{"type":"item.completed","item":{"type":"agent_message","text":"..."}}` | CodexEvent::AgentMessage |
| `test_parse_item_command` | Parse command_execution | `{"type":"item.completed","item":{"type":"command_execution","command":"cargo test"}}` | CodexEvent::Command |
| `test_parse_error_event` | Parse error | `{"type":"error","message":"..."}` | CodexEvent::Error |

### Message Fabric (daemon/crates/agentd/src/fabric/)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_publish_subscribe` | Basic pub/sub works | Publish to topic, subscriber receives | Subscriber gets exact message |
| `test_firehose_receives_all` | subscribe_all gets everything | Publish to 3 different topics | Firehose gets all 3 |
| `test_no_subscriber_no_panic` | Publish with no subscribers | Publish to empty topic | No error, no panic |
| `test_lagged_subscriber` | Slow consumer handling | Publish 300 messages (>256 capacity) | Subscriber gets RecvError::Lagged, not crash |
| `test_multiple_subscribers` | Multiple subscribers same topic | 2 subscribers, 1 publish | Both receive |
| `test_topic_isolation` | Different topics don't leak | Subscribe to topic A, publish to topic B | Subscriber A gets nothing |
| `test_concurrent_publish` | Thread safety | 10 tasks publishing simultaneously | All messages delivered, no data corruption |

### Memory Store (daemon/crates/agentd/src/memory/)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_upsert_insert` | New memory created | upsert("key", "value", "user", "claude") | Returns true, SELECT confirms |
| `test_upsert_update` | Existing memory updated | upsert same key twice with different value | Second value persists, updated_at changes |
| `test_get_existing` | Read existing memory | Insert then get | Returns (value, type) |
| `test_get_missing` | Read nonexistent key | get("nonexistent") | Returns None |
| `test_list_all` | List all memories | Insert 5 memories | Returns 5, ordered by updated_at DESC |
| `test_list_by_type` | Filter by type | Insert 3 user + 2 feedback | list(Some("user")) returns 3 |
| `test_invalid_type_rejected` | Type constraint works | upsert with type="invalid" | SQLite CHECK constraint error |
| `test_session_lifecycle` | Start and end session | start_session, end_session | ended_at populated, summary stored |
| `test_concurrent_reads` | WAL concurrent read | Spawn 10 read tasks while writing | All reads succeed, no locks |
| `test_wal_mode_enabled` | Database in WAL mode | Open store, query PRAGMA journal_mode | Returns "wal" |

### Workflow Engine (daemon/crates/workflow/)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_create_workflow` | Workflow persists to SQLite | Create ConversationWorkflow | Row in workflows table, state=pending |
| `test_step_progression` | Steps advance correctly | Start → complete step 0 → complete step 1 | current_step increments, events logged |
| `test_step_failure_retry` | Failed step retries with backoff | Step fails, retry policy = 3 max | Retry events logged, step re-executed up to 3x |
| `test_step_failure_exhausted` | Max retries → workflow failed | Step fails 4 times (max 3) | Workflow state=failed, no more retries |
| `test_human_gate_pauses` | Workflow pauses at gate | Reach human gate step | Workflow state=paused, WebSocket signal sent |
| `test_human_gate_resumes` | Approval resumes workflow | Pause → approve signal | Workflow state=running, next step executes |
| `test_crash_recovery_resume` | Incomplete workflows resume | Create workflow, advance 2 steps, don't complete 3rd. Reopen DB | Workflow listed as resumable at step 2 |
| `test_event_log_integrity` | All events persisted | Run 5-step workflow | 10 events (started + completed per step) in workflow_events |
| `test_compensation_on_failure` | Undo steps run on failure | Steps 1,2 succeed, step 3 fails with compensation | Compensation events for steps 2,1 in reverse order |

### Fleet Coordinator (daemon/crates/agentd/src/fleet/)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_worktree_create` | Worktree created with unique branch | Create worktree for "claude-1" | `git worktree list` shows new entry |
| `test_worktree_cleanup` | Worktree removed on teardown | Create then teardown | `git worktree list` shows only main |
| `test_worktree_isolation` | Changes in one don't affect another | Write file in worktree A | File not in worktree B or main |
| `test_task_claim_atomic` | No double-claiming | Two tasks try to claim same task simultaneously | Only one succeeds, other gets "already claimed" |
| `test_task_dependency_blocking` | Blocked tasks stay blocked | Task B depends on A. Try to claim B | B state=blocked |
| `test_task_dependency_unblock` | Completing parent unblocks child | Complete task A | Task B state changes from blocked to pending |
| `test_sequential_merge_order` | Merge follows dependency graph | Tasks A→B→C completed | Merge order: A first, then B (with A's context), then C |
| `test_merge_conflict_detection` | Conflicting changes surfaced | Two worktrees modify same line of same file | Merge returns conflict with file path and diff |
| `test_merge_clean` | Non-conflicting changes merge | Two worktrees modify different files | Merge succeeds, both files present in main |

### Config (daemon/crates/agentd/src/config.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_default_config` | Defaults work with no file | No config file present | web_port=8080, db at ~/.triumvirate/memory.db, all agents enabled |
| `test_custom_config` | TOML parsing works | Valid config.toml with port=9090 | web_port=9090 |
| `test_partial_config` | Missing fields use defaults | Config with only web_port | Agents config uses defaults |
| `test_invalid_config` | Bad TOML is error, not crash | Malformed TOML | anyhow::Error, not panic |
| `test_dirs_created` | Config dir created if missing | Delete ~/.triumvirate/ | ensure_dirs() creates it |

### Quota Tracking (daemon/crates/agentd/src/quota.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_token_counting` | Tokens extracted from agent response | Claude response with usage stats | Token count added to routing_log |
| `test_quota_threshold` | 80% triggers fallback | Feed tokens until 80% of configured limit | Digest disabled for that agent |
| `test_quota_reset` | Window reset restores digests | Hit threshold, then advance time past window | Digest re-enabled |
| `test_per_instance_tracking` | Each instance tracked separately | claude-1 and claude-2 both used | Separate counts in routing_log |
| `test_per_type_aggregation` | Type-level quota aggregates instances | claude-1: 40%, claude-2: 45% | Claude type: 85% → threshold exceeded |

### Metrics (daemon/crates/agentd/src/metrics.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_metrics_endpoint_returns_prometheus` | /metrics serves Prometheus format | GET /metrics | Response contains `# HELP`, `# TYPE`, metric lines |
| `test_agent_turn_histogram` | Latency recorded per turn | Complete 3 Claude turns | `agent_turn_duration_seconds` histogram has 3 observations |
| `test_token_counters` | Input/output tokens counted | Claude response with 100 input, 50 output tokens | `agent_tokens_total{direction="input"}` = 100, output = 50 |
| `test_error_counter` | Errors counted by type | Trigger parse failure | `agent_errors_total{error_type="parse_failure"}` incremented |
| `test_active_connections_gauge` | Connection count accurate | Spawn 3 agents | `agent_active_connections` = 3. Kill one → 2 |
| `test_fabric_message_counter` | Fabric throughput tracked | Emit 100 messages | `fabric_messages_total` = 100 |
| `test_quota_gauge` | Quota percentage exported | Claude at 45% | `quota_usage_percent{agent_type="claude"}` = 45 |

### Langfuse Integration (daemon/crates/agentd/src/langfuse.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_langfuse_trace_created` | Every turn creates a trace | Complete Claude turn | HTTP POST to Langfuse /api/public/ingestion with trace data |
| `test_langfuse_generation_logged` | Token counts and cost sent | Response with 500 input, 200 output tokens | Generation record has model, tokens, cost_usd |
| `test_langfuse_session_linked` | Traces share session_id | 3 turns in same session | All 3 traces have same sessionId |
| `test_langfuse_fleet_parent_span` | Fleet tasks grouped | Fleet with 3 agents | Parent trace for fleet, child traces per agent |
| `test_langfuse_unreachable` | Doesn't block agent turns | Langfuse host unreachable | Warning logged, agent turn completes normally, no timeout |
| `test_langfuse_disabled` | Config toggle works | `[langfuse] enabled = false` | No HTTP calls to Langfuse, no errors |
| `test_langfuse_cost_calculation` | Correct dollar amounts | Claude Opus: 500 input, 200 output tokens | Cost = (500 * 5 / 1M) + (200 * 25 / 1M) = $0.0075 |

### Cost Attribution (daemon/crates/agentd/src/cost.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_per_turn_cost` | Cost calculated per turn | Claude turn with known tokens | cost_usd populated in routing_log |
| `test_per_task_cost` | Task cost = sum of turns | Task with 5 turns | Task cost = sum of 5 turn costs |
| `test_per_fleet_cost` | Fleet cost = sum of tasks | Fleet with 3 tasks | Fleet cost = sum of 3 task costs |
| `test_per_session_cost` | Session cost = everything | Full session | Session cost = sum of all routing_log cost_usd |
| `test_pricing_table_configurable` | Custom prices from config | `[pricing] claude_input_per_mtok = 10.0` | Cost uses custom rate, not default |
| `test_gemini_free_tier` | Gemini subscription = $0 | Gemini turn | cost_usd = 0.0 (subscription, not API) |

### Digest System (daemon/crates/agentd/src/digest.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_digest_template` | Template format correct | Agent response with 3 tool calls, 2 files | "Claude-1 proposed X. [3 tool calls, 2 files modified]. Gemini-1, anything to add?" |
| `test_digest_includes_event_ids` | Raw event IDs attached | Agent response | Digest payload includes fabric message IDs |
| `test_digest_no_llm_content` | No generated/summarized text | Complex agent response | Digest contains only mechanical extraction, no paraphrase |
| `test_digest_skipped_at_threshold` | No digest when quota high | Agent at 82% quota | Digest not sent, log entry shows "digest skipped: quota" |
| `test_digest_scoped_to_task` | Only same-task agents get digest | Fleet with tasks A and B | Agent on task B does NOT receive digest from task A agent |

### Routing (daemon/crates/agentd/src/routing.rs)

| Test | What It Verifies | Input | Expected |
|------|-----------------|-------|----------|
| `test_at_mention_claude` | @claude routes to claude | "@claude design the API" | Routed to claude-1, not gemini or codex |
| `test_at_mention_codex` | @codex routes to codex | "@codex implement auth.rs" | Routed to codex-1 |
| `test_default_architecture` | Architecture question → Claude | "How should we design the auth system?" | Routed to Claude (lead for architecture) |
| `test_default_research` | Research question → Gemini | "What are JWT best practices?" | Routed to Gemini |
| `test_default_implementation` | Implementation request → Codex | "Write the auth module" | Routed to Codex |
| `test_debate_command` | /debate triggers workflow | "/debate Redis vs Postgres" | DebateWorkflow started, not routed to single agent |
| `test_fleet_command` | /fleet triggers workflow | "/fleet 3 claude, 2 codex: build auth" | FleetWorkflow started with correct composition |
| `test_routing_logged` | Every route recorded | Send 5 messages | 5 rows in routing_log with correct targets |

---

## Part 3: Error Path Tests

### Agent Connector Errors

| Test | Scenario | Expected Behavior |
|------|----------|-------------------|
| `test_agent_malformed_json` | Agent stdout produces invalid JSON | Error logged, message skipped, agent not killed. Subsequent valid JSON processed normally |
| `test_agent_empty_response` | Agent produces no output for 30s | HealthStatus → Unresponsive. Dashboard shows timeout. No crash |
| `test_agent_crash` | Agent process exits unexpectedly | HealthStatus → Dead. Auto-restart with backoff (1s, 2s, 4s). Max 5 retries |
| `test_agent_crash_loop` | Agent crashes 6 times in a row | After 5 restarts, mark permanently Dead. Dashboard shows error with manual restart option |
| `test_agent_stdin_closed` | Agent closes its stdin | Detect broken pipe. Restart agent |
| `test_agent_stderr_noise` | Agent writes to stderr | Captured to log, not mixed with stdout JSON. No parser disruption |
| `test_agent_slow_response` | Agent takes 60s to respond | Health shows "busy" not "unresponsive" (output is streaming, just slow) |
| `test_agent_binary_not_found` | CLI not in PATH | Spawn fails gracefully. Health → Dead. Dashboard: "claude not found in PATH" |
| `test_agent_permission_denied` | CLI exists but not executable | Spawn fails. Clear error message including path |

### SQLite Errors

| Test | Scenario | Expected Behavior |
|------|----------|-------------------|
| `test_sqlite_disk_full` | Write fails due to disk space | anyhow error propagated. Dashboard red banner. Operation retried after space freed |
| `test_sqlite_locked` | Another process holds lock | Retry with busy_timeout (5000ms). If still locked, error to dashboard |
| `test_sqlite_corrupted` | Database file corrupted | Error on open. Dashboard shows "Database corrupted: [path]". Offer to recreate |
| `test_sqlite_missing_table` | Table doesn't exist (schema drift) | Error with table name. Suggest re-running migrations |

### WebSocket Errors

| Test | Scenario | Expected Behavior |
|------|----------|-------------------|
| `test_ws_client_disconnect` | Browser closes WebSocket | Server cleans up subscription. No resource leak. New connection works |
| `test_ws_client_reconnect` | Browser reconnects after disconnect | New WebSocket established. Missed events are NOT replayed (client fetches state via REST) |
| `test_ws_slow_client` | Browser can't keep up with events | Events buffered up to 1024. Beyond that, oldest dropped. Lagged warning sent to client |
| `test_ws_invalid_message` | Client sends malformed JSON | Error response to client. Connection stays open |
| `test_ws_multiple_clients` | 5 browser tabs open simultaneously | All receive events. No interference between clients |

### Fleet Errors

| Test | Scenario | Expected Behavior |
|------|----------|-------------------|
| `test_fleet_agent_dies_mid_task` | One fleet member crashes during work | Agent restarted in its worktree. Task resumed from last checkpoint. Other fleet members unaffected |
| `test_fleet_worktree_create_fails` | git worktree add fails (dirty state) | Fleet spawn aborted. Error shown. Cleanup any partial worktrees |
| `test_fleet_quota_exhausted_mid_run` | Agent hits quota limit during fleet task | That agent's remaining tasks redistributed to other instances of same type. If no others, tasks marked blocked |
| `test_fleet_merge_conflict_unresolved` | Human doesn't resolve conflict | Merge paused. Other non-dependent merges can proceed. Dashboard shows pending conflict |
| `test_fleet_all_agents_die` | Every agent in fleet crashes | Fleet workflow → failed state. All worktrees preserved (not cleaned up). Human can inspect and retry |

### Shutdown Errors

| Test | Scenario | Expected Behavior |
|------|----------|-------------------|
| `test_sigterm_graceful` | SIGTERM sent to daemon | All agents receive SIGTERM. Wait 5s for completion. Force kill remaining. SQLite closed cleanly |
| `test_sigint_graceful` | Ctrl+C (SIGINT) | Same as SIGTERM |
| `test_agent_ignores_sigterm` | Agent doesn't exit after SIGTERM | Wait 5s, then SIGKILL. Log warning |
| `test_orphan_prevention` | Daemon dies without cleanup | Process group (setpgid) ensures children die. Verify with `ps` after kill -9 daemon |
| `test_workflow_state_persisted_on_shutdown` | In-flight workflow during shutdown | Workflow state written to SQLite before process exits. Resumable on next boot |

---

## Part 4: Performance Benchmarks

| Benchmark | What | Target | Tool |
|-----------|------|--------|------|
| `bench_fabric_throughput` | Messages per second through broadcast channel | > 100,000 msg/s | criterion |
| `bench_sqlite_read_latency` | Memory read under concurrent load | < 1ms p99 with 10 concurrent readers | criterion |
| `bench_sqlite_write_latency` | Memory write (single writer) | < 5ms p99 | criterion |
| `bench_json_parse_claude` | Parse Claude stream-json line | < 10μs per line | criterion |
| `bench_json_parse_gemini` | Parse Gemini ACP response | < 10μs per response | criterion |
| `bench_digest_generation` | Generate mechanical digest from events | < 1ms | criterion |
| `bench_startup_time` | Daemon boot to "Ready" | < 500ms (without agent spawn) | wall clock |
| `bench_websocket_latency` | Fabric event → browser delivery | < 10ms p99 | manual measurement |
| `bench_routing_decision` | Time to decide message routing | < 100μs | criterion |

---

## Part 5: Security Tests

| Test | What It Verifies | Expected |
|------|-----------------|----------|
| `test_agent_worktree_escape` | Agent can't write outside its worktree | Attempt to write to `../../` path. Blocked by Cedar policy or path validation |
| `test_api_invalid_json` | Malformed JSON to REST API | 400 Bad Request with error message. No crash |
| `test_api_missing_fields` | Required fields missing | 422 Unprocessable Entity with field names |
| `test_localhost_only` | Server binds to 127.0.0.1 | Connection from non-localhost IP refused |
| `test_cedar_blocks_destructive` | Cedar policy prevents unapproved destructive ops | Agent tries `git push` without approval → Cedar denies |
| `test_cedar_allows_approved` | Cedar passes approved ops | Human approves via dashboard → op proceeds |
| `test_prompt_injection_via_agent` | Agent output contains control sequences | Daemon treats agent output as data, not commands. No code execution |
| `test_large_payload` | Agent sends 10MB response | Handled without OOM. Streaming processes in chunks |

---

## Part 6: Mock CLI Test Scenarios

### mock-claude Response Scripts

| Script | Behavior | Use Case |
|--------|----------|----------|
| `happy_path.json` | Init → 3 message deltas → final message | Basic conversation test |
| `with_tool_use.json` | Init → tool_use → result → message | Tool invocation flow |
| `slow_response.json` | Init → 5s delay → message | Timeout/health check testing |
| `error_response.json` | Init → error event | Error handling |
| `malformed_output.json` | Valid init → garbage bytes → valid message | Parser resilience |
| `mid_stream_disconnect.json` | Init → 2 deltas → process exit | Crash recovery |
| `large_output.json` | Init → 1000 message deltas → final | Streaming performance |
| `empty_response.json` | Init → final message with empty content | Edge case handling |

### mock-gemini Response Scripts

| Script | Behavior | Use Case |
|--------|----------|----------|
| `happy_path.json` | JSON-RPC response with content | Basic flow |
| `error_response.json` | JSON-RPC error code -32600 | Error handling |
| `notification.json` | Progress notification (no id) | Notification handling |
| `slow_response.json` | 5s delay before response | Timeout testing |
| `invalid_jsonrpc.json` | Missing jsonrpc field | Parser resilience |

### mock-codex Response Scripts

| Script | Behavior | Use Case |
|--------|----------|----------|
| `happy_path.json` | thread.started → turn.started → item.completed (agent_message) → turn.completed | Basic flow |
| `with_command.json` | ... → item.started (command) → item.completed (command, exit 0) → ... | Command execution |
| `command_fails.json` | ... → item.completed (command, exit 1) → ... | Failed command |
| `error_event.json` | thread.started → error | Error handling |
| `multi_item.json` | Multiple items in one turn (message + command + file_change) | Complex turn |

---

## Part 7: Test Infrastructure

### Mock CLI Activation

```toml
# Cargo.toml
[features]
default = []
mock = []  # Enables mock CLI binaries

# In connector code:
#[cfg(feature = "mock")]
fn cli_binary_name(&self) -> &str { "mock-claude" }

#[cfg(not(feature = "mock"))]
fn cli_binary_name(&self) -> &str { "claude" }
```

### Test Levels

| Level | What | When | Command | Gate |
|-------|------|------|---------|------|
| Unit | Individual functions, parsers, state machines | Every commit | `cargo test` | Must pass |
| Integration | Multi-component with mock CLIs | Every feature | `cargo test --features mock` | Must pass |
| Benchmark | Performance baselines | Weekly / before release | `cargo bench` | No regressions > 20% |
| E2E | Full daemon + real CLIs + browser | Before merge to main | Manual + `cargo test --features e2e` | Must pass |
| Security | Boundary validation, injection | Before release | `cargo test --features security` | Must pass |
| Manual | UX (30-second test), chaos (kill -9) | Before ship | Human | Sign-off required |

### CI Pipeline

```
cargo fmt --check                    # Formatting
cargo clippy -- -D warnings          # Linting
cargo test                           # Unit tests
cargo test --features mock           # Integration tests with mock CLIs
cargo build --release                # Release build compiles
```

---

## REQ Traceability Gate

Before `finishing-branch`, run every test. Produce:

```
═══════════════════════════════════════════
  REQ TRACEABILITY — Triumvirate v2
═══════════════════════════════════════════

  PASS: __/26 REQs verified
  FAIL: __ REQs  
  SKIP: __ REQs (manual — flagged for user)

  Unit:        __/__ pass
  Integration: __/__ pass
  Benchmarks:  __/__ within threshold
  Security:    __/__ pass

  ✅ REQ-001 — N agents in browser...
  ...
═══════════════════════════════════════════
```

Gate rule: CANNOT proceed if any REQ is FAIL. Manual tests require user sign-off.

---

## Part 8: Extensive E2E Execution Plan

This section is the executable E2E runbook. It defines exactly how to run end-to-end tests, what data to capture, and what constitutes pass/fail.

### 8.1 E2E Objectives

| Objective | Description | Primary REQs |
|-----------|-------------|--------------|
| API lifecycle integrity | Validate daemon REST lifecycle from clean boot through workflow/fleet operations | REQ-1, REQ-4, REQ-7 |
| Real-time observability | Validate `/metrics`, `/api/costs`, WebSocket stream behavior under live operations | REQ-4, REQ-5, REQ-6 |
| Persistent state correctness | Validate session/memory/workflow state continuity across restart boundaries | REQ-2, REQ-3, REQ-4 |
| Fleet safety and merge controls | Validate worktree creation, task dependency enforcement, and merge gating/approval | REQ-7, FEAT-019..022 |
| UX operability | Validate operator can understand system status/actions in one dashboard session | REQ-6 |

### 8.2 Test Environments

| Environment | Purpose | Connectors | Risk |
|-------------|---------|------------|------|
| `E2E-MOCK` | Deterministic CI and branch gating | mock binaries only | Low |
| `E2E-HYBRID` | Real daemon + selective real agents | mixed | Medium |
| `E2E-REAL` | Pre-release confidence against live CLIs | claude/gemini/codex live | High (nondeterministic) |

Required environment controls:

1. Isolated HOME per run (`HOME=<tmpdir>`) to isolate `~/.triumvirate`.
2. Unique port per run (`web_port` set in temporary config).
3. Ephemeral DB paths (`memory.db`, `workflow.db`) under temporary HOME.
4. Run IDs stamped in logs/artifacts (`E2E_RUN_ID=<timestamp>`).

### 8.3 Core E2E Scenarios

#### Scenario A: Daemon Boot + Health Surface

1. Start `triumvirate-agentd`.
2. Poll `/api/health` until success or timeout (max 10s).
3. Verify fields: `status`, `version`, `agents`.
4. Verify `/metrics` returns Prometheus text with `# HELP` and `# TYPE`.
5. Verify `/api/costs` returns `summary` and per-agent buckets.

Pass criteria:

- Health endpoint available <= 10s.
- `metrics` and `costs` endpoints both parse as expected.

#### Scenario B: Message and Routing Flow

1. `POST /api/message` with plain text.
2. `POST /api/message` with `@claude ...`.
3. `POST /api/message` with `@codex ...`.
4. Query SQLite `routing_log` and verify rows exist for routed turns.
5. Verify reason tagging (`direct_mention`, `lead_default`) is present.

Pass criteria:

- All message calls return `202 accepted`.
- Routing rows persisted with non-empty `target_agent` and `reason`.

#### Scenario C: Debate Workflow Lifecycle

1. `POST /api/debate/start`.
2. `POST /api/debate/challenge`.
3. `POST /api/debate/vote` (>=2 votes).
4. `POST /api/debate/complete`.
5. Validate workflow state/event progression in workflow store.

Pass criteria:

- Each phase returns success and references same `workflow_id`.
- Event history reflects challenge -> vote -> complete.

#### Scenario D: Fleet Task + Dependency Lifecycle

1. `POST /api/fleet/spawn` with deterministic spec (`1 codex` in mock).
2. `GET /api/fleet/tasks` and assert expected bootstrap tasks exist:
   - `contracts`, `implementation`, `merge`
3. Attempt to claim blocked dependency (`implementation`) before parent complete -> expect conflict.
4. Claim/complete `contracts`.
5. Re-attempt claim `implementation` -> expect success.
6. `GET /api/fleet/status/{fleet_id}` validate summary counters.
7. `POST /api/fleet/worktrees/teardown` cleanup.

Pass criteria:

- Dependency enforcement is strict.
- Status and tasks reflect correct transitions.
- Worktree teardown leaves no active rows for the fleet.

#### Scenario E: Governance Gate Validation

1. Call governed endpoint without approval where required (`/api/fleet/merge` with `human_approved=false`).
2. Verify `403` with policy reason.
3. Retry with `human_approved=true`.

Pass criteria:

- Unauthorized destructive operation blocked.
- Explicit approval path allowed (or conflict if repo state blocks merge for unrelated reasons).

#### Scenario F: Restart Recovery

1. Start daemon and create in-progress workflow/fleet state.
2. Kill daemon forcefully.
3. Restart daemon on same DB path.
4. Validate `/api/workflows` includes resumable workflow(s).

Pass criteria:

- Recoverable state visible post-restart.
- No DB corruption, daemon boot succeeds.

### 8.4 UI E2E Scenarios (Playwright)

Required UI flow:

1. Open dashboard, verify primary panels render:
   - Header, AgentGrid, EventFeed, Quota, Workflow, Memory, Merge, Cost.
2. Submit command-bar message.
3. Wait for event-feed growth.
4. Submit fleet command and verify fleet tasks/status panel update.
5. Validate merge resolver form behavior.
6. Validate responsive layout snapshot at:
   - desktop (1440x900)
   - tablet (1024x768)
   - mobile (390x844)

Artifact requirements:

- Screenshot per viewport.
- Trace/video on failure.
- Console log capture.

### 8.5 Command Matrix

From `/Users/mikeboscia/projects/triumvirate/daemon`:

```bash
# Build gates
cd frontend && npm install && npm run build && cd ..
cargo check
cargo test
cargo clippy -- -D warnings
cargo build
cargo build --release

# Deterministic API E2E
cargo test --test e2e_api -- --nocapture

# UI E2E (when Playwright suite is present)
cd frontend
npx playwright test
```

### 8.6 Artifact Bundle Per E2E Run

Each run must produce:

1. `artifacts/<run_id>/health.json`
2. `artifacts/<run_id>/metrics.txt`
3. `artifacts/<run_id>/costs.json`
4. `artifacts/<run_id>/daemon.log`
5. `artifacts/<run_id>/routing_log.sqlite_export.json`
6. `artifacts/<run_id>/screenshots/*` (UI runs)
7. `artifacts/<run_id>/trace.zip` (UI failures)

### 8.7 Pass/Fail Gate

Hard fail if any of the following occurs:

1. Any scenario A-F fails.
2. Any HTTP endpoint returns unexpected status schema.
3. Any daemon panic/crash in logs.
4. Any data integrity violation in SQLite checks.
5. Any clippy warning (pipeline is `-D warnings`).

Soft fail (ship-block unless waived):

1. UI visual regression in primary panels.
2. Startup > 10s median in 3-run sample.
3. Missing artifact bundle files.

### 8.8 Triage Playbook

Failure triage order:

1. `daemon.log` panic/error scan.
2. Endpoint replay (`health`, `metrics`, `costs`, workflow/fleet APIs).
3. DB integrity checks (`PRAGMA integrity_check;`, key table row counts).
4. Worktree state (`git worktree list`, stale branch cleanup).
5. Re-run failed scenario in isolation with `--nocapture`.

Severity rubric:

- `SEV-1`: data loss, crash loops, corrupted state -> immediate stop.
- `SEV-2`: workflow/fleet correctness failure -> stop merge.
- `SEV-3`: observability/UI-only inconsistency -> fix before release tag.
