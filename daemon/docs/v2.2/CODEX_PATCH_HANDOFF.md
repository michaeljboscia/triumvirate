# Codex Patch Handoff — v2.2.1 Fleet + Bugfixes

**Branch:** `main` at `8815c76`
**Date:** 2026-04-07
**Scope:** Make the fleet real + fix 2 bugs from live testing

---

## Read First

1. `daemon/docs/v2.2/SPEC.md` — fleet REQs (028–040)
2. `daemon/crates/fleet/src/orchestrator.rs` — current fleet code
3. GitHub issues #7, #10, #11

---

## Part 1: Make the Fleet Real

The fleet data layer works (worktrees, SQLite task records, status queries, merge logic, review gates). But `ShellAgentLauncher` runs `sh -lc true` — a no-op. No real agent runs in the worktree. This part replaces the stub with actual agent execution.

### FIX-A: Replace stub launcher with real agent invocation

**File:** `fleet/src/orchestrator.rs:52-71`

**Current (stub):**
```rust
async fn launch(&self, _agent: &str, project_root: &Path, worktree_path: &Path) -> Result<()> {
    let mut child = Command::new("sh").arg("-lc").arg("true")
        .current_dir(worktree_path)
        .env("TRIUMVIRATE_PROJECT_ROOT", project_root.as_os_str())
        .spawn()?;
    child.wait().await?;
    Ok(())
}
```

**Replace with:**
```rust
async fn launch(&self, agent: &str, project_root: &Path, worktree_path: &Path) -> Result<Child> {
    let task_file = worktree_path.join(".triumvirate").join("fleet-task.md");
    let prompt = format!(
        "You are a fleet agent. Read your task assignment at {} and complete the work. \
         Commit your changes when done.",
        task_file.display()
    );

    let (cmd, args) = match agent {
        "codex" => ("codex", vec!["exec".to_string(), "--message".to_string(), prompt]),
        "gemini" => ("gemini", vec!["-p".to_string(), prompt]),
        _ => anyhow::bail!("unsupported fleet agent: {agent}"),
    };

    let child = Command::new(cmd)
        .args(&args)
        .current_dir(worktree_path)
        .env("TRIUMVIRATE_PROJECT_ROOT", project_root.as_os_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    Ok(child)
}
```

**Key changes:**
- Returns `Child` (process handle) instead of waiting inline
- Launches actual `codex exec` or `gemini` CLI with the task prompt
- Sets `TRIUMVIRATE_PROJECT_ROOT` via subprocess env (not global — #4 already fixed this)
- Pipes stdout/stderr for future log capture

**The `AgentLauncher` trait must change** — return type becomes `Result<Child>` instead of `Result<()>`.

### FIX-B: Add completion detection

**File:** `fleet/src/orchestrator.rs` — `spawn_fleet_members`

After launching each agent, store the `Child` handle. After all agents are launched, monitor them:

```rust
// After launching all agents:
for (child, task_id, agent_name) in running_agents {
    let store = LedgerStore::open(project_root.clone())?;
    let fleet_id = fleet_id.clone();
    tokio::spawn(async move {
        let status = child.wait().await;
        match status {
            Ok(exit) if exit.success() => {
                // Update task state to "done"
                store.execute("UPDATE tasks SET state='done', completed_at=datetime('now') WHERE task_id=?", [&task_id]);
                // Emit task_completed event
                store.ingest_event(RawEvent { event_type: "task_completed", ... });
                // Request peer review
                // (call peer-review crate)
            }
            Ok(exit) => {
                // Agent exited with error
                store.execute("UPDATE tasks SET state='failed' WHERE task_id=?", [&task_id]);
                store.ingest_event(RawEvent { event_type: "task_failed", ... });
                tracing::error!(fleet_id, task_id, code = exit.code(), "fleet agent failed");
            }
            Err(e) => {
                tracing::error!(fleet_id, task_id, error = %e, "fleet agent process error");
            }
        }

        // Check if all tasks done → trigger merge phase
        let pending = store.query_val::<i64>(
            "SELECT COUNT(*) FROM tasks WHERE fleet_id=? AND state NOT IN ('done','failed')", 
            [&fleet_id]
        );
        if pending == 0 {
            // All agents done — trigger merge
            tracing::info!(fleet_id, "all fleet agents complete, starting merge phase");
            // Call merge::sequential_merge(...)
        }
    });
}
```

**This is the missing orchestration loop.** Currently `spawn_fleet_members` creates worktrees + launches agents + returns. Nobody watches the agents. This adds per-agent Tokio tasks that:
1. Wait for process exit
2. Update task state in SQLite
3. Emit Ledger events
4. Check if all agents are done
5. Trigger merge when ready

### FIX-C: Wire merge trigger

**File:** `fleet/src/orchestrator.rs` (new method) + `fleet/src/merge.rs` (already exists)

The merge logic already exists in `merge.rs` — sequential merge, conflict detection, review gates. It just needs to be CALLED when all agents complete. Add:

```rust
async fn complete_fleet(&self, fleet_id: &str, project_root: &Path) -> Result<()> {
    let store = LedgerStore::open(project_root.to_path_buf())?;
    
    // Update fleet state
    store.execute("UPDATE fleets SET state='merging' WHERE fleet_id=?", [fleet_id]);
    store.ingest_event(RawEvent { event_type: "merge_started", ... });
    
    // Run sequential merge (already implemented in merge.rs)
    let merge_result = self.merge_manager.sequential_merge(fleet_id, project_root).await;
    
    match merge_result {
        Ok(_) => {
            store.execute("UPDATE fleets SET state='done', completed_at=datetime('now') WHERE fleet_id=?", [fleet_id]);
            store.ingest_event(RawEvent { event_type: "fleet_done", ... });
        }
        Err(e) => {
            store.execute("UPDATE fleets SET state='failed', failure_reason=? WHERE fleet_id=?", [&e.to_string(), fleet_id]);
            tracing::error!(fleet_id, error = %e, "fleet merge failed");
        }
    }
    Ok(())
}
```

### Testing the real fleet

After implementing FIX-A through FIX-C, this end-to-end test should work:

```bash
# 1. Verify clean tree
git status --porcelain  # empty

# 2. Dry run
triumvirate fleet-spawn --task "Add v2.2 changelog to ROADMAP.md" --agents codex --dry-run

# 3. Execute
triumvirate fleet-spawn --task "Add v2.2 changelog to ROADMAP.md" --agents codex --wait

# 4. Expected:
#    - Worktree created
#    - codex exec runs in worktree with task prompt
#    - Codex reads fleet-task.md, edits ROADMAP.md, commits
#    - Codex exits → task state = done
#    - Peer review requested (if enabled)
#    - Merge back to main
#    - Worktree cleaned up
#    - Fleet state = done
```

---

## Part 2: Bug Fixes

### FIX-D: Background sweep tries "/" as project root (Issue #10)

**File:** `triumvirate/src/main.rs` — background spool sweep Tokio task

**Problem:** When LRU cache is empty (daemon started via launchd, no CWD), the sweep tries to resolve a default project root and lands on `/`.

**Fix:** The sweep MUST only iterate project roots already in the LRU cache. If cache is empty, do nothing — don't resolve a default.

```rust
// Before (broken):
let project_root = resolve_project_root().unwrap_or(PathBuf::from("/"));
drain_spool(&project_root);

// After (fixed):
for project_root in lru_cache.keys() {
    if let Err(e) = drain_spool(project_root) {
        tracing::warn!(project = %project_root.display(), error = %e, "spool drain failed");
    }
}
// If LRU empty: do nothing. First /ledger/wake populates it.
```

**Test:** Start daemon fresh (no prior wake calls). Verify zero WARN entries for `/`. Send `/ledger/wake` with project root. Verify sweep now includes that project.

**Commit:** `fix(daemon): sweep only LRU-cached project roots, skip when empty (fixes #10)`

### FIX-E: Background fleet spawn swallows errors (Issue #11)

**File:** `fleet/src/orchestrator.rs:122-126`

**Problem:** `let _ = orchestrator.spawn_fleet_members(...).await` discards errors silently.

**Fix:**
```rust
tokio::spawn(async move {
    if let Err(e) = orchestrator
        .spawn_fleet_members(project_root.clone(), fleet_id_bg.clone(), head_sha_bg, agents)
        .await
    {
        tracing::error!(fleet_id = %fleet_id_bg, error = %e, "fleet background spawn failed");
        if let Ok(store) = LedgerStore::open(project_root) {
            let _ = store.execute(
                "UPDATE fleets SET state='failed', failure_reason=? WHERE fleet_id=?",
                [&e.to_string(), &fleet_id_bg],
            );
            let _ = store.ingest_event(RawEvent {
                session_id: fleet_id_bg.clone(),
                event_type: "fleet_failed".to_string(),
                sequence: 1,
                timestamp: chrono::Utc::now().to_rfc3339(),
                payload_json: serde_json::json!({"error": e.to_string()}).to_string(),
            });
        }
    }
});
```

**Test:** `fleet_spawn(wait=false)` with invalid project root → fleet_status shows `failed` with reason. Daemon log has error entry.

**Commit:** `fix(fleet): log errors and update state on background spawn failure (fixes #11)`

### FIX-F: Fleet task file missing actual task description (Issue #7, enhancement)

**File:** `fleet/src/orchestrator.rs:157-162`

**Problem:** fleet-task.md has generic "Implement the assigned fleet task" instead of the actual `task_description` from the spawn request.

**Fix:** Pass `task_description` through to `spawn_fleet_members` and write it as the prose body:

```rust
fs::write(
    worktree_path.join(".triumvirate").join("fleet-task.md"),
    format!(
        "---\ntask_id: {task_id}\nfleet_id: {fleet_id}\nassigned_agent: {agent}\ndepends_on: []\n---\n\n{task_description}\n"
    ),
)?;
```

**Note:** `FleetSpawnRequest` needs a `task_description: String` field (may already exist — check).

**Commit:** `fix(fleet): include actual task description in fleet-task.md (fixes #7)`

---

## Order of Work

1. **FIX-D** — Background sweep (quick, independent) 
2. **FIX-E** — Background spawn error handling (quick, independent)
3. **FIX-F** — Task description in fleet-task.md (quick, independent)
4. **FIX-A** — Replace stub launcher with real agent CLI invocation
5. **FIX-B** — Add completion detection (depends on FIX-A)
6. **FIX-C** — Wire merge trigger (depends on FIX-B)

FIX-D, E, F are independent bug fixes. FIX-A, B, C are the sequential fleet activation.

## Rules

- Each fix gets its own commit with `fixes #N` where applicable
- Run `cargo test --workspace` after each fix
- FIX-A changes the `AgentLauncher` trait — update ALL trait impls including test mocks
- The mock launcher in tests should remain a mock (don't spawn real agents in CI)
- Add `tracing::info!` at each fleet lifecycle transition for observability
- Do NOT modify the spec or canonical docs — code only

## When Done

`cargo test --workspace && cargo clippy --workspace` all green. Then manual test:
1. Start daemon
2. `fleet_spawn(dry_run=false, wait=true, agents=["codex"])` with a real task
3. Codex runs in worktree, completes, merges back
4. `fleet_status` shows `done`
