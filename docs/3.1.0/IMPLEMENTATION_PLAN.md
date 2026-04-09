# Triumvirate 3.1.0 MCP Consolidation — Implementation Plan

**Spec:** `specs/MCP_CONSOLIDATION.md`
**PRD:** `docs/3.1.0/PRD.md`
**Backend:** `docs/3.1.0/BACKEND_STRUCTURE.md`
**Target version:** `3.1.0` (single source: `daemon/Cargo.toml`)

---

## Build Overview

- **7 Waves, 26 Tasks total** (broken down below)
- Preflight (Wave -1): FIX-TEST-MOVED-VALUES ✅ **DONE** (applied 2026-04-09, verified via git blame at main.rs:6106) + T-000 version alignment (PENDING)
- Wave 0: Contracts (types, traits, interfaces)
- Wave 1: Extract MCP tool handlers from main.rs → mcp-tools modules
- Wave 2: Extract HTTP routes from main.rs → daemon-http + DaemonState to daemon-core
- Wave 3: Build aliases + update skills
- Wave 4: Front door swap + cleanup
- Wave 5: Public Release (mandatory standing template)

**Task accounting (canonical):**
| Group | Tasks | Count |
|-------|-------|-------|
| Preflight (Wave -1) | FIX-TEST-MOVED-VALUES ✅, T-000 | 2 |
| Wave 0 | T-001, T-002 | 2 |
| Wave 1 | T-003..T-007 | 5 |
| Wave 2 | T-008..T-010 | 3 |
| Wave 3 | T-011..T-015 | 5 |
| Wave 4 | T-016..T-018 | 3 |
| Wave 5 | T-019..T-024 | 6 |
| **TOTAL** | **All tasks** | **26** |

**Task execution model (two lanes):**
- **ABE-dispatched (Codex worker)**: Tasks that modify repo-internal files. Use `dispatch_codex_worktree`. Each task is audit-gated per Phase 5.3 of goatrodeo. Tasks: T-000, T-001, T-002, T-003, T-004, T-005, T-006, T-007, T-008, T-009, T-010, T-011, T-017, T-018, T-020, T-023
- **Orchestrator-executed (Claude in main session)**: Tasks that modify files OUTSIDE the repo (`~/.claude/skills/*`, `~/.claude.json`) or require user-facing GitHub operations. Claude applies these directly. Tasks: T-012, T-013, T-014, T-015, T-016, T-019, T-021, T-022, T-024
- **FIX-TEST-MOVED-VALUES**: Already complete, no dispatch needed.

**Build method:** ABE fleet dispatch (`dispatch_codex_worktree`) for ABE-lane tasks. This is the dogfood run.
**Audit gate:** Every ABE dispatch requires Phase 5.3 approval from both Gemini + Codex (fresh sessions, blind) before worker spawn.
**max_parallel:** 7 (proven in stress test)
**Test command:** `cargo test --workspace`

---

## Preflight: Version Alignment + Test Fixes

These tasks run BEFORE Wave 0. They are not part of the fleet dispatch — they are manual prerequisites that establish the baseline commit SHA for the worktree gate.

<task id="FIX-TEST-MOVED-VALUES" req="PREFLIGHT" wave="-1" depends="">
  <description>Fix 3 E0382 moved-value errors in abe_red_team_enforcement_blocks_non_compliant_worker test</description>
  <files>daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not refactor the test. Do not fix any other test issues. One line added inside the existing closure at line ~6103.</scope_out>
  <tools>cargo check --workspace, cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Before: cargo test --workspace fails with 3 E0382 errors. After: cargo test --workspace compiles and runs to completion. Full fix plan at docs/3.1.0/FIX_PLAN_test_moved_values.md.</reality_test>
  <done_when>cargo test --workspace passes. Test suite runs. Commit includes only the one-line fix inside the closure.</done_when>
  <status>DONE — applied 2026-04-09, verified via git blame at daemon/crates/triumvirate/src/main.rs:6106. Commit be545585. Test passes: `cargo test -p triumvirate abe_red_team_enforcement_blocks_non_compliant_worker` → 1 passed, 3.41s. Do NOT re-dispatch.</status>
</task>

<task id="T-000" req="PREFLIGHT" wave="-1" depends="">
  <description>Align Cargo workspace version to 3.1.0 and wire version reporting through the codebase</description>
  <files>daemon/Cargo.toml, daemon/crates/daemon-core/src/lib.rs, daemon/crates/daemon-core/src/version.rs, daemon/crates/triumvirate/src/main.rs, scripts/install-git-hooks.sh, scripts/version-drift-check.sh, ROADMAP.md</files>
  <scope_out>Do not bump individual crate-level version fields (they inherit via version.workspace = true). Do not tag git yet. Do not touch dashboard/package.json or mcp-server/package.json. Do not add a changelog file. Do NOT place hook scripts under .git/ (untrackable) — place them in scripts/ and install via scripts/install-git-hooks.sh.</scope_out>
  <tools>cargo check --workspace, cargo build --release, grep</tools>
  <verify>cargo build --release</verify>
  <reality_test>After the change, all of these must be true: (1) grep '^version = "3.1.0"' daemon/Cargo.toml returns a match; (2) cargo build --release exits 0; (3) ./daemon/target/release/triumvirate --version prints a string containing 3.1.0; (4) ./daemon/target/release/triumvirate doctor prints a string containing 3.1.0 (or add version to doctor output); (5) scripts/version-drift-check.sh exists and is executable; (6) scripts/install-git-hooks.sh exists and, when run, installs a pre-commit hook that rejects a test file declaring a mismatched version.</reality_test>
  <done_when>Cargo workspace at 3.1.0. Rust code reads VERSION from daemon_core::version via env!("CARGO_PKG_VERSION") — zero hardcoded version strings. MCP get_info() instructions include 3.1.0. CLI --version prints 3.1.0. scripts/version-drift-check.sh committed to repo (trackable). scripts/install-git-hooks.sh committed and documented. ROADMAP.md updated: "Current version: 3.1.0 (in progress)".</done_when>
</task>

### T-000 Implementation Details

**1. Bump Cargo workspace version**

```toml
# daemon/Cargo.toml — line 6
[workspace.package]
version = "3.1.0"   # was "0.1.0"
```

All 12 crates inherit via `version.workspace = true` — no other Cargo.toml files change.

**2. Create daemon-core/src/version.rs**

```rust
// daemon/crates/daemon-core/src/version.rs (new file)
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = env!("CARGO_PKG_NAME");
```

Export from `daemon-core/src/lib.rs`:

```rust
pub mod version;
pub use version::{VERSION, NAME};
```

**3. Wire version into MCP get_info()**

```rust
// daemon/crates/triumvirate/src/main.rs — update get_info at line 1372
impl ServerHandler for McpBridge {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(format!(
                "Triumvirate MCP bridge v{}. Use `ping` to verify connectivity.",
                daemon_core::VERSION
            ))
    }
}
```

If `rmcp::ServerInfo` exposes a `.with_server_info(name, version)` builder method, use that too. Otherwise include version in the instructions string as shown.

**4. Wire version into HTTP /health**

The existing `health` route at `main.rs:1700` returns a JSON response. Add a `version` field:

```rust
async fn health(
    State(state): State<DaemonState>,
) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: daemon_core::VERSION.to_string(),
        // ... existing fields ...
    })
}
```

Update `HealthResponse` struct to include `version: String`.

**5. Wire version into CLI --version flag**

Clap automatically picks up `CARGO_PKG_VERSION` when you use `#[command(version)]` on the CLI struct. Verify the existing `Cli` struct at main.rs:127 has this attribute. If not, add it.

**6. Create version-drift hook script (tracked, not in .git/)**

Create `scripts/version-drift-check.sh` (mode 755, committed to repo):

```bash
#!/bin/bash
# Verify staged spec/doc files declare a version matching Cargo.toml
set -e

CARGO_VERSION=$(grep -m1 '^version = ' daemon/Cargo.toml | cut -d'"' -f2)

if [ -z "$CARGO_VERSION" ]; then
    echo "version-drift-check: could not read Cargo workspace version"
    exit 1
fi

STAGED=$(git diff --cached --name-only --diff-filter=AM | grep -E '^(specs|docs)/.*\.md$' || true)

FAILED=0
for file in $STAGED; do
    if [ ! -f "$file" ]; then continue; fi
    # Look for a version declaration in the first 20 lines
    SPEC_VERSION=$(head -n 20 "$file" | grep -oE '(Version:|Target version:|version:)\s*`?[0-9]+\.[0-9]+\.[0-9]+`?' | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)
    if [ -n "$SPEC_VERSION" ] && [ "$SPEC_VERSION" != "$CARGO_VERSION" ]; then
        echo "version-drift-check: $file declares $SPEC_VERSION but Cargo.toml is at $CARGO_VERSION"
        FAILED=1
    fi
done

if [ $FAILED -eq 1 ]; then
    echo ""
    echo "Fix: bump daemon/Cargo.toml, or update the spec version header, or stage both together."
    exit 1
fi
```

Create `scripts/install-git-hooks.sh` (also committed, also mode 755):

```bash
#!/bin/bash
# Install repo hooks into the local .git/hooks directory.
# Run once after cloning. Idempotent — safe to re-run.
set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)
HOOK_SRC="$REPO_ROOT/scripts/version-drift-check.sh"
HOOK_DEST="$REPO_ROOT/.git/hooks/pre-commit"

if [ ! -f "$HOOK_SRC" ]; then
    echo "missing: $HOOK_SRC"; exit 1
fi

mkdir -p "$(dirname "$HOOK_DEST")"
ln -sf "$HOOK_SRC" "$HOOK_DEST"
chmod +x "$HOOK_SRC"
echo "installed pre-commit hook → $HOOK_DEST"
```

**Important:** `.git/` is not tracked by git. The hook script lives under `scripts/` (tracked) and is symlinked from `.git/hooks/pre-commit` by `scripts/install-git-hooks.sh`. The ABE worker's contract can include `scripts/version-drift-check.sh` and `scripts/install-git-hooks.sh` in `allowed_files`, but NOT `.git/hooks/pre-commit` — that's local state, not repo state. The orchestrator runs `bash scripts/install-git-hooks.sh` after T-000 completes to wire the hook on the developer's local checkout.

**7. Update ROADMAP.md**

```markdown
# Triumvirate Roadmap

**Last updated:** 2026-04-09
**Current version:** 3.1.0 (in progress — MCP Consolidation sprint)
**Last shipped:** 3.0.0 (ABE — Autonomous Build Enforcement)
```

Add a shipped section for 3.0.0 if it doesn't exist.

### Preflight Commit Sequence

```
1. FIX-TEST-MOVED-VALUES is ALREADY DONE (2026-04-09, commit be545585).
   Verified: cargo test -p triumvirate abe_red_team_enforcement_blocks_non_compliant_worker
             → test result: ok. 1 passed; 0 failed (3.41s)
   Skip this step. Do NOT re-dispatch.

2. T-000 — dispatch to Codex worker via ABE (audit-gated per Phase 5.3):
   Files added to the repo (via worker):
     daemon/Cargo.toml
     daemon/crates/daemon-core/src/version.rs (NEW)
     daemon/crates/daemon-core/src/lib.rs
     daemon/crates/triumvirate/src/main.rs
     scripts/version-drift-check.sh (NEW)
     scripts/install-git-hooks.sh (NEW)
     ROADMAP.md
   Commit message: "chore(3.1.0): bump workspace version, wire version reporting, add drift hook script"

3. cargo build --release                                    # verify binary
   ./daemon/target/release/triumvirate --version            # must print 3.1.0

4. bash scripts/install-git-hooks.sh                        # orchestrator runs locally
   # (the hook lives in scripts/ — .git/hooks/pre-commit is a symlink to it)

5. WAVE0_SHA=$(git rev-parse HEAD)                          # record baseline
   echo "Wave 0 baseline: $WAVE0_SHA"
```

Worktrees created in Wave 0+ branch from this SHA. The orchestrator executes step 4 (the hook install) directly — it is NOT part of any ABE task's `<files>` list because `.git/hooks/` is local state, not repo state.

---

## Wave 0: Contracts and Interfaces

<task id="T-001" req="REQ-B1,REQ-B2,REQ-B3" wave="0" depends="">
  <description>Define ObservabilityBus struct and module trait interfaces in daemon-core</description>
  <files>daemon/crates/daemon-core/src/lib.rs, daemon/crates/shared-types/src/lib.rs</files>
  <scope_out>Do not implement metrics registration. Do not move DaemonMetrics yet — just define the ObservabilityBus struct shape and the trait interfaces each mcp-tools module will receive (SessionStore, AgentExecutor, TaskTrackerHandle, LedgerStoreFactory, etc.)</scope_out>
  <tools>cargo check --workspace, cargo test --workspace</tools>
  <verify>cargo check --workspace</verify>
  <reality_test>Write a test in daemon-core that: (1) constructs an ObservabilityBus with a real DaemonMetrics and a broadcast::channel(16); (2) clones the bus into two threads via tokio::spawn; (3) each thread publishes a test ws event via bus.publish_event("test", json!({"n": N})); (4) a separate receiver attached to the channel receives BOTH messages in the correct JSON shape with matching N values; (5) metrics.agent_requests_total.inc() from one thread is visible via metrics.agent_requests_total.get() == 1 from another. This proves the bus is Clone+Send+Sync, the channel works cross-thread, and the metrics Arc is shared. A stub that returns default values cannot pass the N-value round-trip assertion.</reality_test>
  <done_when>ObservabilityBus struct defined with metrics: Arc&lt;DaemonMetrics&gt; and ws_events: broadcast::Sender&lt;String&gt; fields plus a publish_event() method. Module trait interfaces defined. Round-trip test passes. cargo check --workspace succeeds.</done_when>
</task>

<task id="T-002" req="REQ-A1,REQ-A2" wave="0" depends="">
  <description>Define alias parameter mapping types and the TS→Rust schema conversion functions</description>
  <files>daemon/crates/mcp-tools/src/aliases.rs</files>
  <scope_out>Do not register aliases with tool_router yet. Do not modify McpBridge. Types and mapping functions only.</scope_out>
  <tools>cargo check -p mcp-tools</tools>
  <verify>cargo check -p mcp-tools</verify>
  <reality_test>Call map_spawn_daemon_params with TS schema { target: "gemini", session_name: "x" } → returns Rust schema { agent: "gemini", name: "x" }. Call with { target: "codex" } → returns { agent: "codex" }. Call with { target: "claude" } → returns error (strict enum).</reality_test>
  <done_when>All 8 alias mapping functions defined and unit-tested. Parameter conversion for spawn_daemon, ask_daemon, dismiss_daemon, list_daemons, send_message, get_response, list_jobs, code_review.</done_when>
</task>

---

## Wave 1: Extract MCP Tool Handlers

All tasks in this wave extract existing code from main.rs into mcp-tools modules. ZERO behavioral change. Every function moves verbatim, only changing how it accesses shared state (from &self on McpBridge to narrowed trait interfaces).

<task id="T-003" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract inter-agent tool handlers (spawn_session, ask_session, dismiss_session, list_sessions, ask_agent, get_status, daemon_health) from main.rs to mcp-tools/src/inter_agent.rs</description>
  <files>daemon/crates/mcp-tools/src/inter_agent.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change any tool behavior. Do not modify tool schemas. Do not add new tools. Do not touch HTTP routes.</scope_out>
  <tools>cargo test --workspace, cargo check --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call spawn_session via MCP → session created. Call ask_session → response received. Call dismiss_session → session removed. All 7 tools produce identical output to pre-extraction.</reality_test>
  <done_when>7 tool handlers live in inter_agent.rs. main.rs no longer contains these functions. All existing tests pass.</done_when>
</task>

<task id="T-004" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract ABE tool handlers (dispatch_codex, dispatch_codex_worktree, get_task_status, get_task_output, cancel_task) from main.rs to mcp-tools/src/abe.rs</description>
  <files>daemon/crates/mcp-tools/src/abe.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change ABE behavior. Do not modify dispatch logic. Do not touch the abe/ module files (worktree_setup.rs, orchestrator.rs, etc.).</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call dispatch_codex_worktree via MCP → worktree created, Codex spawned. Call get_task_status → returns correct state. Identical behavior to pre-extraction.</reality_test>
  <done_when>5 ABE tool handlers live in abe.rs. main.rs no longer contains these functions. All tests pass.</done_when>
</task>

<task id="T-005" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract fleet tool handlers (fleet_spawn, fleet_status, fleet_task_list, fleet_claim_task, fleet_cancel) from main.rs to mcp-tools/src/fleet.rs</description>
  <files>daemon/crates/mcp-tools/src/fleet.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change fleet behavior. Do not modify fleet crate internals.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call fleet_spawn via MCP → fleet created. Call fleet_status → returns members. Identical to pre-extraction.</reality_test>
  <done_when>5 fleet tool handlers live in fleet.rs. main.rs no longer contains them. All tests pass.</done_when>
</task>

<task id="T-006" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract knowledge tool handlers (memory_*, scratchpad_*, outbox_*, fallback_*, ledger_*, lesson_*) from main.rs to mcp-tools/src/knowledge.rs</description>
  <files>daemon/crates/mcp-tools/src/knowledge.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change any tool behavior. Do not modify ledger, fallback-outbox, or daemon-core crate internals.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call ledger_query via MCP → returns events. Call scratchpad_write → persists. Call lesson_add → creates lesson. All 17 tools produce identical output.</reality_test>
  <done_when>17 knowledge tool handlers live in knowledge.rs. main.rs no longer contains them. All tests pass.</done_when>
</task>

<task id="T-007" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract review + gemini query tool handlers from main.rs to mcp-tools/src/review.rs and mcp-tools/src/gemini_query.rs</description>
  <files>daemon/crates/mcp-tools/src/review.rs, daemon/crates/mcp-tools/src/gemini_query.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change review or gemini query behavior. Do not modify peer-review crate.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call review_request via MCP → review created. Call query_gemini → Gemini response received. All 5 tools produce identical output.</reality_test>
  <done_when>3 review handlers in review.rs, 2 gemini handlers in gemini_query.rs. main.rs no longer contains them. All tests pass.</done_when>
</task>

---

## Wave 2: Extract HTTP Routes + DaemonState

<task id="T-008" req="REQ-C2" wave="2" depends="T-003,T-004,T-005,T-006,T-007">
  <description>Extract all *_route HTTP handler functions from main.rs into daemon-http crate</description>
  <files>daemon/crates/daemon-http/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change HTTP route behavior. Do not modify Axum router setup (that stays in main.rs startup). Do not move WebSocket or dashboard routes in this task (T-009 handles those).</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>curl localhost:8080/health → 200. curl localhost:8080/ledger/health → valid JSON. curl POST /ask-agent → agent response. All 19 API routes produce identical output.</reality_test>
  <done_when>19 HTTP route handler functions live in daemon-http. main.rs no longer contains *_route functions (except ws, dashboard, metrics — see T-009). All tests pass.</done_when>
</task>

<task id="T-009" req="REQ-C2,REQ-B3" wave="2" depends="T-001,T-008">
  <description>Extract WebSocket handler, dashboard routes, metrics route, and DaemonState construction into daemon-http and daemon-core</description>
  <files>daemon/crates/daemon-http/src/lib.rs, daemon/crates/daemon-core/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change WebSocket or metrics behavior. DaemonMetrics struct definition moves to daemon-core (or shared location). Axum Router construction stays in main.rs.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>WebSocket connects to ws://localhost:8080/ws → receives bootstrap events. curl /metrics → Prometheus text format with all 12 metrics. Dashboard at / serves HTML.</reality_test>
  <done_when>ws_route, dashboard routes, metrics_route, DaemonMetrics, DaemonState, encode_ws_event, publish_ws_event all extracted. main.rs contains only startup wiring. All tests pass.</done_when>
</task>

<task id="T-010" req="REQ-C3,REQ-C4" wave="2" depends="T-008,T-009">
  <description>Verify main.rs is under 300 lines and contains only startup wiring</description>
  <files>daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not add new functionality. This is a verification + cleanup task only. Remove dead imports, unused helper functions, stale comments.</scope_out>
  <tools>wc -l daemon/crates/triumvirate/src/main.rs, cargo test --workspace</tools>
  <verify>cargo test --workspace && test $(wc -l < daemon/crates/triumvirate/src/main.rs) -lt 300</verify>
  <reality_test>wc -l main.rs reports < 300. grep for any async fn that isn't main/run_daemon/run_doctor/run_status — should find zero tool handlers or route handlers. cargo test --workspace passes.</reality_test>
  <done_when>main.rs is under 300 lines. Contains only: CLI parsing, config, tracing init, DaemonState build, McpBridge build, server spawns, shutdown. No tool handlers. No route handlers.</done_when>
</task>

---

## Wave 3: Aliases + Skill Updates

<task id="T-011" req="REQ-A1,REQ-A2,REQ-A3" wave="3" depends="T-002,T-003">
  <description>Register all 8 alias tools in the MCP tool_router with parameter mapping and logging</description>
  <files>daemon/crates/mcp-tools/src/aliases.rs, daemon/crates/mcp-tools/src/lib.rs</files>
  <scope_out>Do not modify canonical tool handlers. Do not change ~/.claude.json yet. Aliases are ADDITIONAL tools, not replacements.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call spawn_daemon via MCP → creates session (same as spawn_session). Call ask_daemon → gets response. Call send_message → synchronously calls ask_session, returns response (not job_id). Daemon log shows tracing::info with tool_alias field for each call.</reality_test>
  <done_when>All 8 aliases registered and callable via MCP. Parameter mapping works for all schema differences. Alias usage logged. get_response returns deprecation notice.</done_when>
</task>

<task id="T-012" req="REQ-J2" wave="3" depends="T-011">
  <description>Update send-to-codex skill to use mcp__triumvirate__ask_session</description>
  <files>~/.claude/skills/send-to-codex/SKILL.md</files>
  <scope_out>Do not change skill behavior or purpose. Only update tool references and remove send_message/get_response two-step pattern.</scope_out>
  <tools>cat ~/.claude/skills/send-to-codex/SKILL.md</tools>
  <verify>grep -c "mcp__triumvirate__ask_session" ~/.claude/skills/send-to-codex/SKILL.md</verify>
  <reality_test>Invoke /send-to-codex with a question → Codex responds via ask_session. No job_id in the flow. Response is direct.</reality_test>
  <done_when>Skill references mcp__triumvirate__ask_session. No references to mcp__inter-agent. No send_message/get_response pattern.</done_when>
</task>

<task id="T-013" req="REQ-J3" wave="3" depends="T-011">
  <description>Update send-to-gemini skill to use mcp__triumvirate__ask_session</description>
  <files>~/.claude/skills/send-to-gemini/SKILL.md</files>
  <scope_out>Same as T-012.</scope_out>
  <tools>cat ~/.claude/skills/send-to-gemini/SKILL.md</tools>
  <verify>grep -c "mcp__triumvirate__ask_session" ~/.claude/skills/send-to-gemini/SKILL.md</verify>
  <reality_test>Invoke /send-to-gemini with a question → Gemini responds via ask_session. Direct response.</reality_test>
  <done_when>Skill references mcp__triumvirate__ask_session. No inter-agent references.</done_when>
</task>

<task id="T-014" req="REQ-J4" wave="3" depends="T-011">
  <description>Update send-to-siblings skill to use mcp__triumvirate__ask_session for both agents</description>
  <files>~/.claude/skills/send-to-siblings/SKILL.md</files>
  <scope_out>Same as T-012.</scope_out>
  <tools>cat ~/.claude/skills/send-to-siblings/SKILL.md</tools>
  <verify>grep -c "mcp__triumvirate__ask_session" ~/.claude/skills/send-to-siblings/SKILL.md</verify>
  <reality_test>Invoke /send-to-siblings → both Gemini and Codex respond via ask_session. Both direct responses.</reality_test>
  <done_when>Skill references mcp__triumvirate__ask_session for both agents. No inter-agent references.</done_when>
</task>

<task id="T-015" req="REQ-A1" wave="3" depends="T-011">
  <description>Update inter-agent-protocol, goatrodeo, design-goatrodeo, and crystallize skills to use mcp__triumvirate__* tool names</description>
  <files>~/.claude/skills/inter-agent-protocol/SKILL.md, ~/.claude/skills/goatrodeo.md, ~/.claude/skills/design-goatrodeo.md, ~/.claude/skills/crystallize/factory/phase-2-diagnose.md</files>
  <scope_out>Do not change skill logic or purpose. Only update MCP tool name references from mcp__inter-agent__* to mcp__triumvirate__*.</scope_out>
  <tools>grep -r "mcp__inter-agent" ~/.claude/skills/</tools>
  <verify>grep -rc "mcp__inter-agent" ~/.claude/skills/ | grep -v ":0$" | wc -l should be 0</verify>
  <reality_test>grep -r "mcp__inter-agent" ~/.claude/skills/ returns zero matches. All skills reference mcp__triumvirate__* only.</reality_test>
  <done_when>Zero references to mcp__inter-agent in any skill file. All updated to mcp__triumvirate__.</done_when>
</task>

---

## Wave 4: Front Door Swap + Cleanup

<task id="T-016" req="REQ-F1,REQ-F2,REQ-F3,REQ-F4" wave="4" depends="T-011,T-015">
  <description>Verify all tools (canonical + aliases) work through the Rust daemon, then remove inter-agent entry from ~/.claude.json</description>
  <files>~/.claude.json</files>
  <scope_out>Do not modify the Rust daemon. This is a configuration change only. Keep a backup of ~/.claude.json before modification.</scope_out>
  <tools>cp ~/.claude.json ~/.claude.json.bak.v3.0 && cargo test --workspace</tools>
  <verify>grep -c "inter-agent" ~/.claude.json should be 0 (after removal). All MCP tools callable via mcp__triumvirate__*.</verify>
  <reality_test>After removing inter-agent entry: call spawn_daemon (alias) → works. Call spawn_session (canonical) → works. Call dispatch_codex_worktree → works. Call every alias and canonical tool name — all respond. No "tool not found" errors. Node process for inter-agent is not running.</reality_test>
  <done_when>~/.claude.json has no inter-agent entry. All 40+ tools accessible via triumvirate. No Node.js MCP process running.</done_when>
</task>

<task id="T-017" req="REQ-X1" wave="4" depends="T-016">
  <description>Archive the TS MCP server to archive/mcp-server-ts/</description>
  <files>mcp-server/, archive/mcp-server-ts/</files>
  <scope_out>Do not delete — archive. Do not modify the archived code. Preserve for reference.</scope_out>
  <tools>git mv mcp-server archive/mcp-server-ts</tools>
  <verify>test -d archive/mcp-server-ts/src && ! test -d mcp-server</verify>
  <reality_test>archive/mcp-server-ts/src/server.ts exists. mcp-server/ directory does not exist. git status shows rename.</reality_test>
  <done_when>TS MCP server archived. Original directory gone. Git tracks the move.</done_when>
</task>

<task id="T-018" req="REQ-X2,REQ-X3" wave="4" depends="T-016,T-017">
  <description>Internal verification: full test suite, all tools, clean state — end of internal work, entry gate for Wave 5</description>
  <files>daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>No code changes. Verification only. Does NOT touch public release artifacts — that is Wave 5.</scope_out>
  <tools>cargo test --workspace, wc -l daemon/crates/triumvirate/src/main.rs</tools>
  <verify>cargo test --workspace passes. wc -l main.rs < 300. No Node.js MCP processes running. All skills work.</verify>
  <reality_test>Run cargo test --workspace → all pass (including existing 156+ tests). Run wc -l main.rs → under 300. Invoke /goatrodeo → twins spawn via Rust daemon. Invoke /send-to-codex → Codex responds. curl /metrics → all metrics present. WebSocket → events flow.</reality_test>
  <done_when>Internal work complete. All 3.1.0 functionality works locally. Ready for Wave 5 public release.</done_when>
</task>

---

## Wave 5: Public Release

Wave 5 makes the sprint actually available to users. Every sprint ends here — this is not optional. Skipping Wave 5 means the work shipped internally but not publicly, which is the same as not shipping.

**Rule: Nothing in Wave 5 starts until Wave 4 is complete and T-018 passes.**

<task id="T-019" req="REQ-X1,REQ-X2,REQ-X3" wave="5" depends="T-018">
  <description>Repo hygiene sweep — audit the public-facing state of the repo and bring it into alignment with reality</description>
  <files>README.md, ROADMAP.md, CONTRIBUTING.md, NOTICE.md, .gitignore, archive/</files>
  <scope_out>Do not change code. Do not rewrite README from scratch — edit in place. Do not delete archive/ contents. Do not publish anything yet.</scope_out>
  <tools>grep, find, git status, git ls-files</tools>
  <verify>All audit checklist items below pass. git status clean.</verify>
  <reality_test>
    Audit checklist (every item must pass):
    1. README.md mentions version 3.1.0 or has no hardcoded version
    2. README.md describes the CURRENT architecture (Rust daemon, fleet, ABE) — no stale TS MCP server mentions as the primary
    3. README.md has a "Status" section that is accurate (what works, what doesn't)
    4. README.md install instructions actually work (tested in T-023)
    5. ROADMAP.md marks 3.0.0 (ABE) as shipped, 3.1.0 as in-progress → shipped
    6. ROADMAP.md's "Current version" matches daemon/Cargo.toml
    7. CONTRIBUTING.md reflects the current dev workflow (cargo build, cargo test, where to file issues)
    8. NOTICE.md is current (copyright year, license attributions)
    9. No .env, credentials.json, or secret files in git ls-files output
    10. .gitignore excludes .triumvirate/, target/, *.db, .env, dead-drop/
    11. archive/ contents are explicitly acknowledged in .gitignore or README as historical reference
    12. Old v2.x docs that reference the TS MCP server are either updated or archived
    13. No references to "inter-agent" as the primary MCP server anywhere in top-level docs
    14. Links in README.md resolve (no 404s to old paths)
    15. License file (LICENSE) exists and is MIT
  </reality_test>
  <done_when>All 15 audit items pass. Commit: "docs(3.1.0): repo hygiene sweep — align public docs with Rust daemon reality". git status clean after commit.</done_when>
</task>

<task id="T-020" req="REQ-X3" wave="5" depends="T-019">
  <description>Build release binaries for all supported platforms with reproducible checksums</description>
  <files>scripts/build-release.sh (new or existing), daemon/target/release-dist/</files>
  <scope_out>Do not publish yet — T-022 handles GitHub release. Do not sign binaries (requires manual step). Do not build Windows if it has never been tested — mark it as "next sprint" if missing.</scope_out>
  <tools>cargo build --release, cargo zigbuild (for cross-compilation), sha256sum, tar, zip</tools>
  <verify>All expected binaries exist in daemon/target/release-dist/ with matching .sha256 files.</verify>
  <reality_test>
    For each target platform:
    1. Binary exists at daemon/target/release-dist/triumvirate-3.1.0-{target}.{tar.gz|zip}
    2. SHA256 file exists alongside
    3. Binary executes: `./triumvirate --version` prints "3.1.0"
    4. Binary executes: `./triumvirate --help` prints help text with all subcommands
    5. Binary size is within 10% of prior release (sanity check for bloat)
    
    Target platforms (minimum):
    - darwin-arm64 (Apple Silicon) — native build
    - darwin-x64 (Intel Mac) — cross-compile via rustup target
    - linux-x64 — cross-compile or docker
    - linux-arm64 — cross-compile or docker
    
    Windows is OUT OF SCOPE unless the install.sh already supports it.
  </reality_test>
  <done_when>4 binaries built. Checksums generated. All --version and --help checks pass. Release artifacts in daemon/target/release-dist/ ready for upload.</done_when>
</task>

<task id="T-021" req="REQ-X3" wave="5" depends="T-018,T-019">
  <description>Draft CHANGELOG.md entry and release notes for 3.1.0</description>
  <files>CHANGELOG.md, docs/3.1.0/RELEASE_NOTES.md</files>
  <scope_out>Do not describe internal refactoring details that don't affect users. Focus on user-visible changes. Do not invent features that weren't built.</scope_out>
  <tools>git log, cat docs/3.1.0/*.md</tools>
  <verify>CHANGELOG.md has a 3.1.0 section at the top. RELEASE_NOTES.md exists with migration steps.</verify>
  <reality_test>
    CHANGELOG.md 3.1.0 section must include:
    1. Release date (today)
    2. Summary (2-3 sentences, user-facing)
    3. What Changed (MCP consolidation, TS server removed, version alignment)
    4. Migration Notes (required: update ~/.claude.json, old skills work via aliases)
    5. Breaking Changes (if any — explicit list)
    6. Under The Hood (brief mention of refactor for developers)
    
    RELEASE_NOTES.md must include:
    1. Install command (curl install.sh or direct binary download)
    2. Upgrade steps for existing users
    3. Verification command (triumvirate --version)
    4. Rollback instructions (restore ~/.claude.json backup)
    5. Known issues (if any)
    6. Link to full CHANGELOG
    
    Both documents must be written in plain english. No jargon without explanation. A user who has never seen this project should understand what they get.
  </reality_test>
  <done_when>CHANGELOG.md entry + RELEASE_NOTES.md reviewed and committed. Both readable by non-contributors. Migration path explicit.</done_when>
</task>

<task id="T-022" req="REQ-X3" wave="5" depends="T-020,T-021">
  <description>Publish GitHub release 3.1.0 with binaries, checksums, and release notes</description>
  <files>GitHub release (external, via gh CLI)</files>
  <scope_out>Do not push to main if not already pushed. Do not force-push. Do not close issues automatically — T-024 handles issue cleanup.</scope_out>
  <tools>gh release create, gh release upload, git tag, git push --tags</tools>
  <verify>gh release view 3.1.0 succeeds. All binaries visible on the GitHub release page.</verify>
  <reality_test>
    1. git tag 3.1.0 exists on the final Wave 4 commit
    2. git push --tags succeeded (tag visible on remote)
    3. gh release view 3.1.0 shows the release with:
       - Title: "Triumvirate 3.1.0 — MCP Consolidation"
       - Body: contents of RELEASE_NOTES.md
       - 4 binary assets (one per platform)
       - 4 SHA256 assets
    4. Downloading a binary via https://github.com/michaeljboscia/triumvirate/releases/download/3.1.0/triumvirate-3.1.0-darwin-arm64.tar.gz works
    5. Extracted binary runs and prints 3.1.0
  </reality_test>
  <done_when>GitHub release 3.1.0 published. Binaries downloadable. Tag pushed. Release visible at https://github.com/michaeljboscia/triumvirate/releases/tag/3.1.0.</done_when>
</task>

<task id="T-023" req="REQ-X3" wave="5" depends="T-022">
  <description>End-to-end install verification on a clean environment — prove other people can actually use this</description>
  <files>scripts/smoke-install.sh (new or update install.sh), docs/3.1.0/INSTALL_VERIFIED.md</files>
  <scope_out>Do not test in your existing environment (defeats the purpose). Use a fresh directory, Docker container, or a VM. Do not modify the release after publishing unless a critical bug is found.</scope_out>
  <tools>docker run, mktemp -d, curl, bash install.sh</tools>
  <verify>Fresh environment install succeeds and produces a working daemon.</verify>
  <reality_test>
    Clean-room test (run in a fresh Docker container or mktemp directory with zero Triumvirate state):
    1. curl -fsSL https://github.com/michaeljboscia/triumvirate/releases/download/3.1.0/install.sh | bash
       OR
       Download binary tarball, extract, move to PATH
    2. triumvirate --version prints 3.1.0
    3. triumvirate doctor reports ready state
    4. Configure ~/.claude.json with the triumvirate MCP entry (copy from release notes)
    5. Start daemon: triumvirate daemon &
    6. Daemon starts, HTTP :8080/health returns 200 with version 3.1.0
    7. MCP tools list includes all expected tools
    8. Kill daemon: triumvirate stop (or kill $PID)
    9. Cleanup: rm -rf the fresh directory
    
    Write the steps that worked to docs/3.1.0/INSTALL_VERIFIED.md as the canonical install guide.
  </reality_test>
  <done_when>Fresh environment install worked without modification. INSTALL_VERIFIED.md written with exact reproducible steps. If the install failed, fix it and re-run before marking done.</done_when>
</task>

<task id="T-024" req="REQ-X3" wave="5" depends="T-022">
  <description>Close resolved GitHub issues and update issue state for the sprint</description>
  <files>GitHub issues (external, via gh CLI)</files>
  <scope_out>Do not close issues that weren't actually resolved. Do not close issues that touch on v3.2 (observability) or v3.3 (token economics) scope. Do not delete issues.</scope_out>
  <tools>gh issue list, gh issue close, gh issue comment</tools>
  <verify>Relevant issues closed with a reference to release 3.1.0.</verify>
  <reality_test>
    For each issue labeled "v3.1" or "mcp-consolidation" in the repo:
    1. If resolved by this sprint: close with comment "Resolved in 3.1.0 — see https://github.com/michaeljboscia/triumvirate/releases/tag/3.1.0"
    2. If NOT resolved: leave open, relabel for next sprint
    3. Issue #13 (Rust rewrite) → close if fully addressed, or relabel/narrow if only partially
    
    Issues #19, #20, #21, #23 stay OPEN — they are v3.2 / v3.3 scope.
    
    After cleanup: gh issue list --label v3.1 should show no open issues, OR only issues explicitly flagged as "moved to v3.2".
  </reality_test>
  <done_when>Relevant issues closed. Next-sprint issues relabeled. Issue tracker reflects the post-3.1.0 state.</done_when>
</task>

---

## Standing Sprint Checklist (applies to every future sprint)

Wave 5 is a **template**. Every sprint from 3.1.0 forward MUST include a "Public Release" wave with these 6 tasks (or equivalents). If a sprint ships without completing Wave 5:
- The sprint is NOT done
- The version is NOT tagged
- Other people CANNOT use the work
- The retrospective must document why Wave 5 was skipped

The goal: **Every sprint produces a downloadable, installable, verifiable public artifact.** Not "it works on my machine" — "it works on a stranger's machine."

---

## Execution Contract

### Backlog Freeze
This document contains 24 tasks across 6 waves (including Preflight as Wave -1 and Public Release as Wave 5). This is the COMPLETE backlog.
- Do NOT accept new tasks until all tasks are complete (backlog_status: 0).
- If new requirements arrive mid-execution, respond: `blocked_on: scope-change — [describe new requirement]` and STOP.
- Only the human can add, remove, or reorder tasks in this backlog.

### Execution Order
- Wave order is strict: complete ALL tasks in Wave N before starting Wave N+1.
- Within a wave: tasks are parallel-safe (no dependencies on each other). Execute concurrently or in any order.
- Within a sequential group: strict FIFO. Do not start T(N+1) before T(N) is committed and reported.

### Definition of Done (Per Task)
A task is DONE when ALL of these are true:
1. Code is written (not stubbed — see reality test)
2. `<verify>` passes (compilation/type check)
3. `<reality_test>` passes (behavioral check that a stub cannot fake)
4. `<done_when>` condition is met (semantic completion check)
5. FULL test suite passes (`cargo test --workspace`) — not just this task's tests
6. Git commit is created with message referencing task ID

A task that passes its own tests but breaks other tests is NOT done. Fix the regression first.

### Commit Report Format
After each task commit, respond with EXACTLY this format and nothing else:
```
task: T-{ID}
commit: {hash}
changed: {1-5 bullets, one per file or logical change}
tests: cargo test --workspace → {pass count}/{total count} passed
remaining: {N} tasks in current wave, {M} total
```
No interim progress updates. No explanations between tasks. No summaries until backlog_status: 0.

### Collateral Fix Protocol
If completing a task REQUIRES touching files outside that task's `<files>` list:
1. Label the commit: `collateral-fix: T-{ID} — {one-line justification}`
2. List extra files in the commit report under a `collateral:` field
3. Re-run full test suite after the collateral fix

If you WANT to touch adjacent code but don't NEED to, don't. Scope discipline > local improvement.

### Blocked Protocol
If blocked on any task, respond with EXACTLY:
```
blocked_on: {single concrete blocker}
task: T-{ID}
evidence: {command + output summary, max 5 lines}
proposed_fix: {single action you would take}
```
Then STOP. Do not proceed to the next task. Do not attempt workarounds without reporting.

### Context-Switch Refusal
If you receive instructions not in this backlog during execution:
- Respond: "Outside current execution contract. Backlog has {N} remaining tasks. Complete backlog first, or explicitly cancel it."
- Do NOT start the new work.

### Self-Validation (MANDATORY)
After each task commit, run the validation script:
```
~/.claude/scripts/validate-task.sh T-{ID} "cargo test --workspace" {files from <files> list}
```
- If BLOCKED (exit 1): fix the failure before proceeding. Do NOT skip to next task.
- If WARN (exit 2): proceed, but include warnings in commit report.
- If PASS (exit 0): proceed to next task.

### End-of-Execution Report
When all tasks are complete (through Wave 5), respond with:
```
backlog_status: 0 remaining
completed_tasks: [FIX-TEST-MOVED-VALUES, T-000, T-001..T-024]
total_commits: {N}
collateral_fixes: {N} ({list if any})
validation: {N}/{N} tasks passed validate-task.sh
test_suite: cargo test --workspace → {pass/fail with counts}
public_release:
  tag: 3.1.0
  github_release: https://github.com/michaeljboscia/triumvirate/releases/tag/3.1.0
  binaries: [darwin-arm64, darwin-x64, linux-x64, linux-arm64]
  install_verified: {true|false}
  issues_closed: {N}
main_rs_lines: {N}   # must be < 300
version_reported: {string from triumvirate --version}
```
