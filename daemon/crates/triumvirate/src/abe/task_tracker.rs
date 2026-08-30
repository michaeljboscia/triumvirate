use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use daemon_core::{EventSequencer, encode_ws_event, metrics::DaemonMetrics};
use shared_types::{
    AgentStreamEvent, CancelTaskResponse, GetTaskOutputResponse, GetTaskStatusResponse,
    TaskStatus, WorkerLifecycleType,
};
use tokio::{process::Child, sync::{Mutex, broadcast}};
use tracing::instrument;

#[derive(Debug)]
struct TaskOutput {
    commit_sha: String,
    modified_files: Vec<String>,
    stdout: String,
    validation_log: Option<String>,
    test_output: Option<String>,
}

#[derive(Debug)]
struct TaskRecord {
    wave: u32,
    status: TaskStatus,
    started_at: Instant,
    commit_sha: Option<String>,
    exit_code: Option<i32>,
    error_message: Option<String>,
    output: Option<TaskOutput>,
    child: Option<Arc<Mutex<Child>>>,
    worktree_path: Option<PathBuf>,
    /// FEAT-014 (REQ-010) T-004: Pantheon lineage captured at register time
    /// from `daemon_core::current_pantheon_session()`. Stored here (rather
    /// than read on-demand) because `mark_completed`/`mark_failed` execute
    /// inside a `tokio::spawn`ed monitor task where the task-local is not
    /// visible. `None` for legacy (non-Pantheon) callers.
    parent_session_id: Option<String>,
    /// FEAT-014 (REQ-010) T-004: Root of the dispatch chain. For direct
    /// Pantheon dispatches this equals `parent_session_id`. For chained
    /// dispatches (v4.0+) it identifies the original Pantheon session at
    /// the top of the chain.
    root_session_id: Option<String>,
    /// Which dispatch tool created this task ("dispatch_codex" | "dispatch_codex_worktree").
    /// `None` for any future non-codex ABE task, which then emits NO tv_codex_dispatch — the
    /// arbiter must not mislabel work it did not dispatch as a codex run.
    dispatch_surface: Option<&'static str>,
    /// Repo the dispatch ran against, for the PostHog `tv_repo` slice. Stored so the tracker
    /// (which is the arbiter and therefore the emitter) can report it at terminal time.
    dispatch_repo: Option<String>,
    /// OS pid of the worker, captured at register time. `cancel()` signals the worker through
    /// THIS pid rather than the `Mutex<Child>`, because the monitor holds that mutex for the
    /// entire `wait()`; locking it in cancel would block until the worker exited on its own,
    /// which defeated cancellation entirely. `None` for tasks with no live child (setup-failed).
    pid: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TaskTracker {
    inner: Arc<Mutex<HashMap<String, TaskRecord>>>,
    metrics: Option<Arc<DaemonMetrics>>,
    ws_events: Option<broadcast::Sender<String>>,
    /// FEAT-014 (REQ-010): shared sequencer for WorkerLifecycle events.
    /// None means WorkerLifecycle emission is disabled (legacy constructor).
    sequencer: Option<Arc<EventSequencer>>,
}

impl Default for TaskTracker {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            metrics: None,
            ws_events: None,
            sequencer: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    Transitioned,
    AlreadyTerminal,
    NotFound,
}

fn is_terminal(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed
            | TaskStatus::Stuck
            | TaskStatus::Failed
            | TaskStatus::Timeout
            | TaskStatus::SetupFailed
            | TaskStatus::Cancelled
    )
}

impl TaskTracker {
    pub fn with_metrics(metrics: Arc<DaemonMetrics>) -> Self {
        Self::with_observability(metrics, None)
    }

    pub fn with_observability(
        metrics: Arc<DaemonMetrics>,
        ws_events: Option<broadcast::Sender<String>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            metrics: Some(metrics),
            ws_events,
            sequencer: None,
        }
    }

    /// FEAT-014 (REQ-010): Full observability with WorkerLifecycle event
    /// emission enabled. Shares a sequencer with the streaming executor so
    /// WorkerLifecycle events have monotonic seq numbers aligned with the
    /// rest of the event stream.
    pub fn with_pantheon_observability(
        metrics: Arc<DaemonMetrics>,
        ws_events: broadcast::Sender<String>,
        sequencer: Arc<EventSequencer>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            metrics: Some(metrics),
            ws_events: Some(ws_events),
            sequencer: Some(sequencer),
        }
    }

    fn inc_dispatch_status(&self, status: &'static str) {
        if let Some(metrics) = &self.metrics {
            metrics
                .abe_task_dispatch_total
                .with_label_values(&[status])
                .inc();
        }
    }

    fn emit_task_state(
        &self,
        task_id: &str,
        wave: u32,
        status: &'static str,
        duration_ms: u128,
        commit_sha: Option<&str>,
    ) {
        let Some(ws_events) = &self.ws_events else {
            return;
        };
        let payload = serde_json::json!({
            "task_id": task_id,
            "wave": wave,
            "status": status,
            "duration_ms": duration_ms,
            "commit_sha": commit_sha,
        });
        let _ = ws_events.send(encode_ws_event("abe_task_state", payload));
    }

    /// FEAT-014 (REQ-010): Emit a WorkerLifecycle event on the shared WebSocket
    /// channel. Used by Pantheon's sidebar to populate the worker hierarchy
    /// in real-time. Lineage fields (parent_session_id, root_session_id) are
    /// populated by T-004's dispatch path when available; NULL for legacy
    /// callers without Pantheon context.
    ///
    /// No-op if either ws_events or sequencer is absent (legacy constructor).
    fn emit_worker_lifecycle(
        &self,
        lifecycle: WorkerLifecycleType,
        task_id: &str,
        parent_session_id: Option<String>,
        root_session_id: Option<String>,
        commit_sha: Option<String>,
        error_message: Option<String>,
        elapsed_ms: Option<u64>,
    ) {
        let (Some(ws_events), Some(sequencer)) = (&self.ws_events, &self.sequencer) else {
            return;
        };
        let event = AgentStreamEvent::WorkerLifecycle {
            lifecycle,
            // Reports the agent ABE is actually configured to dispatch. This was hardcoded to
            // "codex", so a lifecycle event would have LIED about which agent ran once ABE could
            // dispatch anything else.
            agent: mcp_tools::abe::abe_worker_agent(),
            session_name: format!("{}-worker-{task_id}", mcp_tools::abe::abe_worker_agent()),
            task_id: Some(task_id.to_string()),
            parent_session_id,
            root_session_id,
            commit_sha,
            error_message,
            elapsed_ms,
            seq: sequencer.next(),
        };
        let payload = match serde_json::to_value(&event) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(error = %err, "failed to serialize WorkerLifecycle event");
                return;
            }
        };
        let _ = ws_events.send(encode_ws_event("agent_stream", payload));
    }

    #[instrument(skip_all, fields(task_id = %task_id, wave))]
    #[allow(clippy::too_many_arguments)]
    pub async fn register(
        &self,
        task_id: String,
        wave: u32,
        child: Arc<Mutex<Child>>,
        worktree_path: Option<PathBuf>,
        parent_session_id: Option<String>,
        root_session_id: Option<String>,
        dispatch_surface: Option<&'static str>,
        dispatch_repo: Option<String>,
        dispatch_started_at: Instant,
    ) {
        // Capture the pid BEFORE the child is stored and before the monitor task starts waiting
        // on it: cancel() signals via this pid so it never has to contend for the Child mutex the
        // monitor holds across wait(). Safe to lock here because register is awaited before the
        // monitor is spawned, so there is no contention yet.
        let pid = child.lock().await.id();
        let mut guard = self.inner.lock().await;
        // Use the caller's dispatch clock, not now(): the dispatch (cwd resolution, spawn,
        // worktree setup) began before register, and the reported duration must cover it.
        let started_at = dispatch_started_at;
        guard.insert(
            task_id.clone(),
            TaskRecord {
                wave,
                status: TaskStatus::Working,
                started_at,
                commit_sha: None,
                exit_code: None,
                error_message: None,
                output: None,
                child: Some(child),
                worktree_path,
                parent_session_id: parent_session_id.clone(),
                root_session_id: root_session_id.clone(),
                dispatch_surface,
                dispatch_repo,
                pid,
            },
        );
        self.emit_task_state(&task_id, wave, "dispatched", 0, None);
        self.emit_task_state(&task_id, wave, "running", 0, None);
        // FEAT-014 (REQ-010) T-004: WorkerLifecycle::Spawned with Pantheon lineage.
        self.emit_worker_lifecycle(
            WorkerLifecycleType::Spawned,
            &task_id,
            parent_session_id,
            root_session_id,
            None,
            None,
            None,
        );
    }

    /// Emit the `tv_codex_dispatch` terminal event. The tracker is the ARBITER of a dispatch's
    /// terminal status (only one caller wins each transition), so it is the only place that can
    /// author a truthful, exactly-once outcome event — a racing observer knows only whether IT
    /// won, never whether someone else won first (the rule all three siblings converged on).
    ///
    /// MUST be called AFTER the inner lock is released: `record_codex_dispatch` is sync and
    /// `capture()` detaches the POST via its own `tokio::spawn`, so nothing network-y is awaited
    /// here, but holding the tracker mutex across any spawn is a latency trap. Every call site
    /// below builds a snapshot under the lock, `drop(guard)`, then calls this.
    ///
    /// `surface: None` means a non-codex ABE task: emit nothing rather than mislabel it.
    ///
    /// KNOWN, ACCEPTED weakness (DeepSeek): if the calling mark_* panics in the narrow window
    /// between `drop(guard)` and this call, the record is already terminal (immutable) but no
    /// event fires, and the supervisor's mark_failed then sees AlreadyTerminal and also emits
    /// nothing — a terminal task with no telemetry. This is a best-effort observability loss,
    /// not a state-correctness bug, and closing it (emit-under-lock, or a two-phase pending_emit
    /// flag) is not worth the cost for fire-and-forget analytics. The state is always right; the
    /// event is best-effort, exactly like every other capture() in this module.
    fn emit_terminal(
        surface: Option<&'static str>,
        repo: Option<&str>,
        outcome: &'static str,
        duration_ms: u64,
        exit_code: Option<i32>,
        files_changed: Option<usize>,
    ) {
        let Some(surface) = surface else { return };
        mcp_bridge::posthog::record_codex_dispatch(
            surface,
            outcome,
            duration_ms,
            repo,
            files_changed,
            exit_code,
        );
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "completed"))]
    pub async fn mark_completed(
        &self,
        task_id: &str,
        commit_sha: String,
        modified_files: Vec<String>,
        stdout: String,
        validation_log: Option<String>,
        test_output: Option<String>,
    ) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::Completed;
        task.commit_sha = Some(commit_sha.clone());
        let duration_ms = task.started_at.elapsed().as_millis();
        task.output = Some(TaskOutput {
            commit_sha,
            modified_files,
            stdout,
            validation_log,
            test_output,
        });
        task.child = None;
        self.inc_dispatch_status("completed");
        tracing::info!(
            task_id = %task_id,
            commit_sha = task.commit_sha.as_deref().unwrap_or_default(),
            duration_ms = task.started_at.elapsed().as_millis() as u64,
            "abe_task_completed"
        );
        self.emit_task_state(
            task_id,
            task.wave,
            "completed",
            duration_ms,
            task.commit_sha.as_deref(),
        );
        // FEAT-014 (REQ-010) T-004: WorkerLifecycle::Completed with lineage
        // re-read from TaskRecord (task-local from register is gone — this
        // executes in the spawned monitor task).
        self.emit_worker_lifecycle(
            WorkerLifecycleType::Completed,
            task_id,
            task.parent_session_id.clone(),
            task.root_session_id.clone(),
            task.commit_sha.clone(),
            None,
            Some(duration_ms as u64),
        );
        let surface = task.dispatch_surface;
        let repo = task.dispatch_repo.clone();
        let files = task.output.as_ref().map(|o| o.modified_files.len());
        let exit_code = task.exit_code;
        drop(guard);
        Self::emit_terminal(surface, repo.as_deref(), "completed", duration_ms as u64, exit_code, files);
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "failed"))]
    pub async fn mark_failed(
        &self,
        task_id: &str,
        exit_code: Option<i32>,
        error_message: String,
    ) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::Failed;
        let duration_ms = task.started_at.elapsed().as_millis();
        task.exit_code = exit_code;
        task.error_message = Some(error_message.clone());
        task.child = None;
        self.inc_dispatch_status("failed");
        self.emit_task_state(task_id, task.wave, "failed", duration_ms, task.commit_sha.as_deref());
        // FEAT-014 (REQ-010) T-004: WorkerLifecycle::Failed with lineage from TaskRecord.
        self.emit_worker_lifecycle(
            WorkerLifecycleType::Failed,
            task_id,
            task.parent_session_id.clone(),
            task.root_session_id.clone(),
            None,
            Some(error_message),
            Some(duration_ms as u64),
        );
        // exit 0 with a Failed status is the silent no-op: codex exited clean and committed
        // nothing. Derived structurally from exit_code, exactly as the old read-back did.
        let outcome = if exit_code == Some(0) { "no_commit" } else { "failed" };
        let surface = task.dispatch_surface;
        let repo = task.dispatch_repo.clone();
        drop(guard);
        Self::emit_terminal(surface, repo.as_deref(), outcome, duration_ms as u64, exit_code, None);
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "timeout"))]
    pub async fn mark_timeout(&self, task_id: &str) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::Timeout;
        let duration_ms = task.started_at.elapsed().as_millis();
        task.error_message = Some("task timed out".to_string());
        task.child = None;
        self.inc_dispatch_status("timeout");
        self.emit_task_state(task_id, task.wave, "timeout", duration_ms, task.commit_sha.as_deref());
        let surface = task.dispatch_surface;
        let repo = task.dispatch_repo.clone();
        drop(guard);
        Self::emit_terminal(surface, repo.as_deref(), "timeout", duration_ms as u64, None, None);
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "stuck"))]
    pub async fn mark_stuck(&self, task_id: &str, error_message: String) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        // Guard against watchdog race: if the sentinel file exists, the worker
        // already completed — don't mark it STUCK. The sentinel watcher will
        // handle the Completed transition.
        if let Some(wt) = &task.worktree_path {
            let sentinel = wt.join(".triumvirate").join("TASK_COMPLETE.json");
            if sentinel.exists() {
                tracing::info!(
                    task_id = %task_id,
                    "watchdog tried to mark STUCK but sentinel exists — skipping"
                );
                return TransitionOutcome::AlreadyTerminal;
            }
        }
        task.status = TaskStatus::Stuck;
        task.error_message = Some(error_message);
        task.child = None;
        let duration_ms = task.started_at.elapsed().as_millis() as u64;
        let surface = task.dispatch_surface;
        let repo = task.dispatch_repo.clone();
        drop(guard);
        Self::emit_terminal(surface, repo.as_deref(), "stuck", duration_ms, None, None);
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "setup_failed"))]
    pub async fn mark_setup_failed(&self, task_id: &str, error_message: String) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::SetupFailed;
        task.error_message = Some(error_message);
        task.child = None;
        let duration_ms = task.started_at.elapsed().as_millis() as u64;
        let surface = task.dispatch_surface;
        let repo = task.dispatch_repo.clone();
        drop(guard);
        Self::emit_terminal(surface, repo.as_deref(), "setup_failed", duration_ms, None, None);
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn get_status(&self, task_id: &str) -> Option<GetTaskStatusResponse> {
        let guard = self.inner.lock().await;
        let task = guard.get(task_id)?;
        Some(GetTaskStatusResponse {
            task_id: task_id.to_string(),
            status: task.status.clone(),
            elapsed_sec: Some(task.started_at.elapsed().as_secs()),
            commit_sha: task.commit_sha.clone(),
            exit_code: task.exit_code,
            error_message: task.error_message.clone(),
        })
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn get_output(&self, task_id: &str) -> Option<GetTaskOutputResponse> {
        let guard = self.inner.lock().await;
        let task = guard.get(task_id)?;
        let output = task.output.as_ref()?;
        Some(GetTaskOutputResponse {
            task_id: task_id.to_string(),
            commit_sha: output.commit_sha.clone(),
            modified_files: output.modified_files.clone(),
            stdout: output.stdout.clone(),
            validation_log: output.validation_log.clone(),
            test_output: output.test_output.clone(),
        })
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "cancelled"))]
    pub async fn cancel(&self, task_id: &str) -> Option<CancelTaskResponse> {
        // Phase 1: CLAIM the cancellation atomically under the records lock. Writing Cancelled
        // here — while the task is still Working — is what makes cancel win the race against the
        // monitor. When the monitor's wait() later reaps the worker we are about to signal, its
        // mark_failed/mark_timeout see an already-terminal task and no-op (they guard on
        // is_terminal), so the outcome cannot flip from "cancelled" to "failed".
        //
        // We must NOT touch the `Mutex<Child>` at all: the monitor holds it for the entire
        // wait(), so locking it here would block until the worker exited on its own — the exact
        // bug this replaces (cancel was serialized behind the worker's full runtime, then reported
        // the monitor's "no commit -> failed" as "already-failed"). We signal by the pid captured
        // at register time instead, and hand reaping to the monitor.
        let (pid, worktree_path, worktree_display, wave, duration_ms, surface, repo) = {
            let mut guard = self.inner.lock().await;
            let task = guard.get_mut(task_id)?;
            if is_terminal(&task.status) {
                // Genuinely already finished (completed/failed/timed out) before we arrived.
                return Some(CancelTaskResponse {
                    task_id: task_id.to_string(),
                    status: format!("already-{}", status_label(&task.status)),
                    worktree_path: task.worktree_path.as_ref().map(|p| p.display().to_string()),
                });
            }
            task.status = TaskStatus::Cancelled;
            // Drop the tracker's Child handle; the monitor still holds its own clone and will reap
            // the exit our signal triggers.
            task.child = None;
            (
                task.pid,
                task.worktree_path.clone(),
                task.worktree_path.as_ref().map(|p| p.display().to_string()),
                task.wave,
                task.started_at.elapsed().as_millis(),
                task.dispatch_surface,
                task.dispatch_repo.clone(),
            )
        };

        // Phase 2: signal the worker by pid (never blocks — no Child mutex involved). SIGTERM asks
        // it to stop; the monitor reaps the exit. A detached escalation force-kills only if the
        // worker is still the SAME live process after a grace, guarded by kill(pid, 0) to avoid
        // signalling a recycled pid. If SIGTERM is ignored past that, the monitor's own timeout is
        // the final backstop.
        if let Some(pid) = pid {
            let pid_i = pid as i32;
            let _ = unsafe { libc::kill(pid_i, libc::SIGTERM) };
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if unsafe { libc::kill(pid_i, 0) } == 0 {
                    let _ = unsafe { libc::kill(pid_i, libc::SIGKILL) };
                }
            });
        }
        if let Some(worktree_path) = worktree_path.as_ref() {
            cleanup_git_locks(worktree_path);
        }

        self.inc_dispatch_status("cancelled");
        self.emit_task_state(task_id, wave, "cancelled", duration_ms, None);
        Self::emit_terminal(surface, repo.as_deref(), "cancelled", duration_ms as u64, None, None);

        Some(CancelTaskResponse {
            task_id: task_id.to_string(),
            status: "cancelled".to_string(),
            worktree_path: worktree_display,
        })
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn exists(&self, task_id: &str) -> bool {
        let guard = self.inner.lock().await;
        guard.contains_key(task_id)
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn worktree_path_for(&self, task_id: &str) -> Option<PathBuf> {
        let guard = self.inner.lock().await;
        guard.get(task_id).and_then(|task| task.worktree_path.clone())
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "setup_failed"))]
    pub async fn register_setup_failed(&self, task_id: String, error_message: String) {
        let mut guard = self.inner.lock().await;
        guard.insert(
            task_id.clone(),
            TaskRecord {
                wave: 0,
                status: TaskStatus::SetupFailed,
                started_at: Instant::now(),
                commit_sha: None,
                exit_code: None,
                error_message: Some(error_message),
                output: None,
                child: None,
                worktree_path: None,
                parent_session_id: None,
                root_session_id: None,
                // This path inserts a terminal SetupFailed record directly (no register), so it
                // never emits tv_codex_dispatch via the tracker. The dispatch_codex_worktree
                // call site reports setup_failed directly instead.
                dispatch_surface: None,
                dispatch_repo: None,
                pid: None,
            },
        );
        self.emit_task_state(&task_id, 0, "failed", 0, None);
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn elapsed_for(&self, task_id: &str) -> Option<Duration> {
        let guard = self.inner.lock().await;
        guard.get(task_id).map(|r| r.started_at.elapsed())
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn status_for(&self, task_id: &str) -> Option<TaskStatus> {
        let guard = self.inner.lock().await;
        guard.get(task_id).map(|r| r.status.clone())
    }

    /// FEAT-012 (REQ-017) T-007.5: Snapshot every tracked worker as a
    /// `WorkerInfo` row suitable for `/api/workers` aggregation.
    ///
    /// Returns one entry per task currently in the inner map. Each entry
    /// uses the task_id as both `session_id` and `task_id` (ABE workers do
    /// not have a separate session identifier — each dispatch is its own
    /// session). The `agent` is hardcoded to `"codex"` because the daemon
    /// only spawns Codex workers via ABE today; if Claude/Gemini ABE workers
    /// land in a future task, this should branch on `TaskRecord` metadata.
    ///
    /// `started_at` is rendered as RFC 3339 by reconstructing a SystemTime
    /// from the elapsed Instant — this is approximate (within milliseconds)
    /// because Instant has no anchor to wall-clock time, but it's accurate
    /// enough for client-side hierarchical display and matches what the
    /// existing watch CLI expects.
    ///
    /// Status mapping: TaskStatus → string follows the BACKEND_STRUCTURE.md
    /// contract: working/completed/failed/timeout/stuck/setup_failed/cancelled.
    ///
    /// This method is the authoritative read path for the `/api/workers`
    /// endpoint added in T-008. It is intentionally allocation-heavy
    /// (clones every record) — the caller is a once-per-HTTP-request reader,
    /// not a hot loop.
    pub async fn snapshot_workers(&self) -> Vec<shared_types::WorkerInfo> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let guard = self.inner.lock().await;
        let now_sys = SystemTime::now();
        let now_inst = Instant::now();
        guard
            .iter()
            .map(|(task_id, rec)| {
                let elapsed = now_inst.saturating_duration_since(rec.started_at);
                let started_sys = now_sys.checked_sub(elapsed).unwrap_or(now_sys);
                let started_at = format_rfc3339(started_sys.duration_since(UNIX_EPOCH).ok());
                shared_types::WorkerInfo {
                    session_id: task_id.clone(),
                    agent: "codex".to_string(),
                    name: format!("codex-worker-{task_id}"),
                    status: status_label(&rec.status).to_string(),
                    task_id: Some(task_id.clone()),
                    parent_session_id: rec.parent_session_id.clone(),
                    root_session_id: rec.root_session_id.clone(),
                    pantheon_session_id: None, // ABE workers inherit pantheon lineage via parent
                    cwd: rec.worktree_path.as_ref().map(|p| p.display().to_string()),
                    started_at,
                    elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
                }
            })
            .collect()
    }
}

/// Map TaskStatus to the canonical lowercase string used by /api/workers.
fn status_label(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Working => "working",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Timeout => "timeout",
        TaskStatus::Stuck => "stuck",
        TaskStatus::SetupFailed => "setup_failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

/// Render an optional UNIX duration as an RFC 3339 timestamp string.
/// On overflow or None, returns the epoch (1970-01-01T00:00:00Z) — this
/// only happens for clocks set before 1970 and is harmless for the UI.
fn format_rfc3339(unix: Option<std::time::Duration>) -> String {
    let secs = unix.map(|d| d.as_secs() as i64).unwrap_or(0);
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

fn cleanup_git_locks(worktree_path: &Path) {
    let git_dir = resolve_git_dir(worktree_path);
    let _ = fs::remove_file(git_dir.join("index.lock"));
    let _ = fs::remove_file(worktree_path.join(".git").join("index.lock"));
    let _ = fs::remove_file(worktree_path.join(".git/index.lock"));
}

fn resolve_git_dir(worktree_path: &Path) -> PathBuf {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_file() {
        let content = fs::read_to_string(&dot_git).unwrap_or_default();
        if let Some(gitdir) = content.lines().find_map(|line| line.strip_prefix("gitdir:")) {
            let raw = gitdir.trim();
            let parsed = PathBuf::from(raw);
            if parsed.is_absolute() {
                return parsed;
            }
            return worktree_path.join(parsed);
        }
    }
    dot_git
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use daemon_core::{EventSequencer, metrics::DaemonMetrics};
    use shared_types::{AgentStreamEvent, TaskStatus, WorkerLifecycleType};
    use tokio::sync::broadcast;

    use super::{TaskTracker, TransitionOutcome};
    use tokio::{process::Command, sync::Mutex};

    async fn register_working_task(tracker: &TaskTracker, task_id: &str) {
        let child = Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .expect("spawn child");
        tracker
            .register(
                task_id.to_string(),
                1,
                Arc::new(Mutex::new(child)),
                None,
                None,
                None,
                None,
                None,
                std::time::Instant::now(),
            )
            .await;
    }

    #[tokio::test]
    async fn register_creates_working_task() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-01").await;

        let status = tracker.get_status("U-TT-01").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Working);
    }

    #[tokio::test]
    async fn mark_completed_transitions_to_completed() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-02").await;

        let outcome = tracker
            .mark_completed(
                "U-TT-02",
                "abc123".to_string(),
                vec!["src/lib.rs".to_string()],
                "done".to_string(),
                Some("validation ok".to_string()),
                Some("tests ok".to_string()),
            )
            .await;
        assert_eq!(outcome, TransitionOutcome::Transitioned);

        let status = tracker.get_status("U-TT-02").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Completed);
        assert_eq!(status.commit_sha.as_deref(), Some("abc123"));
    }

    #[tokio::test]
    async fn mark_failed_transitions_to_failed() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-03").await;

        let outcome = tracker
            .mark_failed("U-TT-03", Some(1), "boom".to_string())
            .await;
        assert_eq!(outcome, TransitionOutcome::Transitioned);

        let status = tracker.get_status("U-TT-03").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Failed);
        assert_eq!(status.exit_code, Some(1));
        assert_eq!(status.error_message.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn mark_timeout_transitions_to_timeout() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-04").await;

        let outcome = tracker.mark_timeout("U-TT-04").await;
        assert_eq!(outcome, TransitionOutcome::Transitioned);

        let status = tracker.get_status("U-TT-04").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Timeout);
    }

    #[tokio::test]
    async fn mark_stuck_transitions_to_stuck() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-05").await;

        let outcome = tracker
            .mark_stuck("U-TT-05", "stuck waiting for signal".to_string())
            .await;
        assert_eq!(outcome, TransitionOutcome::Transitioned);

        let status = tracker.get_status("U-TT-05").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Stuck);
        assert_eq!(
            status.error_message.as_deref(),
            Some("stuck waiting for signal")
        );
    }

    #[tokio::test]
    async fn double_transition_returns_already_terminal() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-06").await;

        let first = tracker
            .mark_completed(
                "U-TT-06",
                "def456".to_string(),
                Vec::new(),
                String::new(),
                None,
                None,
            )
            .await;
        assert_eq!(first, TransitionOutcome::Transitioned);

        let second = tracker
            .mark_failed("U-TT-06", Some(2), "should fail transition".to_string())
            .await;
        assert_eq!(second, TransitionOutcome::AlreadyTerminal);
    }

    #[tokio::test]
    async fn unknown_task_returns_not_found() {
        let tracker = TaskTracker::default();

        let outcome = tracker
            .mark_completed(
                "U-TT-07-missing",
                "nope".to_string(),
                Vec::new(),
                String::new(),
                None,
                None,
            )
            .await;
        assert_eq!(outcome, TransitionOutcome::NotFound);
    }

    #[tokio::test]
    async fn cancel_transitions_task_to_cancelled() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-08").await;

        let response = tracker.cancel("U-TT-08").await.expect("cancel response");
        assert_eq!(response.status, "cancelled");

        let status = tracker.get_status("U-TT-08").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_cleans_git_index_lock() {
        let tracker = TaskTracker::default();
        let tmp = tempfile::tempdir().expect("tmp");
        let wt = tmp.path().join("worktree");
        std::fs::create_dir_all(wt.join(".git")).expect("mkdir");
        let lock = wt.join(".git").join("index.lock");
        std::fs::write(&lock, "lock").expect("write lock");

        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 5")
            .spawn()
            .expect("spawn");
        tracker
            .register(
                "T-LOCK".to_string(),
                0,
                Arc::new(Mutex::new(child)),
                Some(wt.clone()),
                None,
                None,
                None,
                None,
                std::time::Instant::now(),
            )
            .await;

        let _ = tracker.cancel("T-LOCK").await;
        assert!(!lock.exists(), "expected index.lock to be removed on cancel");
    }

    /// FEAT-014 (REQ-010) reality test for T-007:
    /// Verify that register/mark_completed/mark_failed emit WorkerLifecycle
    /// events via the broadcast channel when Pantheon observability is enabled.
    /// A stub implementation that only emits abe_task_state would fail this test.
    #[tokio::test]
    async fn pantheon_observability_emits_worker_lifecycle_events() {
        let metrics = Arc::new(DaemonMetrics::new().expect("metrics"));
        let (ws_tx, mut ws_rx) = broadcast::channel(64);
        let sequencer = Arc::new(EventSequencer::new());
        let tracker = TaskTracker::with_pantheon_observability(
            metrics,
            ws_tx,
            sequencer,
        );

        // Register a task → should emit Spawned
        let child = Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .expect("spawn");
        tracker
            .register(
                "T-LIFECYCLE-01".to_string(),
                1,
                Arc::new(Mutex::new(child)),
                None,
                None,
                None,
                None,
                None,
                std::time::Instant::now(),
            )
            .await;

        // Drain events and find the WorkerLifecycle ones
        let mut lifecycle_events: Vec<AgentStreamEvent> = Vec::new();
        while let Ok(msg) = ws_rx.try_recv() {
            // msg is an encoded ws_event wrapper. We parse the outer envelope
            // and look for "agent_stream" type events, then extract the payload.
            let outer: serde_json::Value =
                serde_json::from_str(&msg).expect("valid json");
            if outer.get("type").and_then(|v| v.as_str()) == Some("agent_stream") {
                let payload = outer.get("payload").expect("payload field");
                if let Ok(event) = serde_json::from_value::<AgentStreamEvent>(payload.clone()) {
                    if matches!(event, AgentStreamEvent::WorkerLifecycle { .. }) {
                        lifecycle_events.push(event);
                    }
                }
            }
        }

        // Register emits exactly one WorkerLifecycle::Spawned
        assert_eq!(lifecycle_events.len(), 1, "expected 1 Spawned event");
        match &lifecycle_events[0] {
            AgentStreamEvent::WorkerLifecycle {
                lifecycle,
                task_id,
                session_name,
                agent,
                seq,
                ..
            } => {
                assert!(matches!(lifecycle, WorkerLifecycleType::Spawned));
                assert_eq!(task_id.as_deref(), Some("T-LIFECYCLE-01"));
                assert_eq!(session_name, "codex-worker-T-LIFECYCLE-01");
                assert_eq!(agent, "codex");
                assert!(*seq > 0, "seq should be monotonic");
            }
            other => panic!("expected WorkerLifecycle, got {other:?}"),
        }

        // Mark completed → should emit Completed
        let _outcome = tracker
            .mark_completed(
                "T-LIFECYCLE-01",
                "abc123".to_string(),
                vec!["src/auth.rs".to_string()],
                "done".to_string(),
                None,
                None,
            )
            .await;

        // Drain again
        let mut completed_events: Vec<AgentStreamEvent> = Vec::new();
        while let Ok(msg) = ws_rx.try_recv() {
            let outer: serde_json::Value = serde_json::from_str(&msg).expect("valid json");
            if outer.get("type").and_then(|v| v.as_str()) == Some("agent_stream") {
                let payload = outer.get("payload").expect("payload field");
                if let Ok(event) = serde_json::from_value::<AgentStreamEvent>(payload.clone()) {
                    if matches!(event, AgentStreamEvent::WorkerLifecycle { .. }) {
                        completed_events.push(event);
                    }
                }
            }
        }

        assert_eq!(completed_events.len(), 1, "expected 1 Completed event");
        match &completed_events[0] {
            AgentStreamEvent::WorkerLifecycle {
                lifecycle,
                task_id,
                commit_sha,
                elapsed_ms,
                ..
            } => {
                assert!(matches!(lifecycle, WorkerLifecycleType::Completed));
                assert_eq!(task_id.as_deref(), Some("T-LIFECYCLE-01"));
                assert_eq!(commit_sha.as_deref(), Some("abc123"));
                assert!(elapsed_ms.is_some(), "elapsed_ms should be populated");
            }
            other => panic!("expected WorkerLifecycle::Completed, got {other:?}"),
        }
    }

    /// FEAT-014 (REQ-010) T-004 reality test:
    /// Verify that Pantheon lineage (parent_session_id / root_session_id)
    /// supplied to register() is stamped onto ALL three WorkerLifecycle
    /// events — Spawned at register time, Completed/Failed from the spawned
    /// monitor task (which cannot read task-locals). A stub that only
    /// stamped lineage on Spawned would pass the Spawned assertion but fail
    /// on Completed/Failed.
    #[tokio::test]
    async fn pantheon_lineage_propagates_through_all_worker_lifecycle_events() {
        let metrics = Arc::new(DaemonMetrics::new().expect("metrics"));
        let (ws_tx, mut ws_rx) = broadcast::channel(64);
        let sequencer = Arc::new(EventSequencer::new());
        let tracker = TaskTracker::with_pantheon_observability(metrics, ws_tx, sequencer);

        let parent_id = "pantheon-session-xyz".to_string();
        let root_id = "pantheon-root-xyz".to_string();

        // --- Path 1: register → mark_completed → expect Spawned + Completed ---
        let child = Command::new("sh").arg("-c").arg("true").spawn().expect("spawn");
        tracker
            .register(
                "T-LIN-OK".to_string(),
                1,
                Arc::new(Mutex::new(child)),
                None,
                Some(parent_id.clone()),
                Some(root_id.clone()),
                None,
                None,
                std::time::Instant::now(),
            )
            .await;
        let _ = tracker
            .mark_completed(
                "T-LIN-OK",
                "deadbeef".to_string(),
                vec!["x.rs".to_string()],
                String::new(),
                None,
                None,
            )
            .await;

        // --- Path 2: register → mark_failed → expect Spawned + Failed ---
        let child2 = Command::new("sh").arg("-c").arg("true").spawn().expect("spawn");
        tracker
            .register(
                "T-LIN-ERR".to_string(),
                1,
                Arc::new(Mutex::new(child2)),
                None,
                Some(parent_id.clone()),
                Some(root_id.clone()),
                None,
                None,
                std::time::Instant::now(),
            )
            .await;
        let _ = tracker
            .mark_failed("T-LIN-ERR", Some(1), "boom".to_string())
            .await;

        // Drain and collect WorkerLifecycle events by task_id → lifecycle
        let mut collected: Vec<AgentStreamEvent> = Vec::new();
        while let Ok(msg) = ws_rx.try_recv() {
            let outer: serde_json::Value = serde_json::from_str(&msg).expect("json");
            if outer.get("type").and_then(|v| v.as_str()) != Some("agent_stream") {
                continue;
            }
            let payload = outer.get("payload").expect("payload");
            if let Ok(ev) = serde_json::from_value::<AgentStreamEvent>(payload.clone()) {
                if matches!(ev, AgentStreamEvent::WorkerLifecycle { .. }) {
                    collected.push(ev);
                }
            }
        }

        // Must see 4 events total: Spawned+Completed for OK, Spawned+Failed for ERR.
        assert_eq!(
            collected.len(),
            4,
            "expected 4 WorkerLifecycle events, got {}: {collected:?}",
            collected.len()
        );

        // Every single one MUST carry the Pantheon lineage we provided.
        for ev in &collected {
            match ev {
                AgentStreamEvent::WorkerLifecycle {
                    parent_session_id,
                    root_session_id,
                    task_id,
                    ..
                } => {
                    assert_eq!(
                        parent_session_id.as_deref(),
                        Some(parent_id.as_str()),
                        "parent_session_id missing on {task_id:?}"
                    );
                    assert_eq!(
                        root_session_id.as_deref(),
                        Some(root_id.as_str()),
                        "root_session_id missing on {task_id:?}"
                    );
                }
                _ => unreachable!(),
            }
        }

        // And the distribution must be one Spawned + one terminal per task_id.
        let mut ok_spawned = 0;
        let mut ok_completed = 0;
        let mut err_spawned = 0;
        let mut err_failed = 0;
        for ev in &collected {
            if let AgentStreamEvent::WorkerLifecycle {
                lifecycle, task_id, ..
            } = ev
            {
                match (task_id.as_deref(), lifecycle) {
                    (Some("T-LIN-OK"), WorkerLifecycleType::Spawned) => ok_spawned += 1,
                    (Some("T-LIN-OK"), WorkerLifecycleType::Completed) => ok_completed += 1,
                    (Some("T-LIN-ERR"), WorkerLifecycleType::Spawned) => err_spawned += 1,
                    (Some("T-LIN-ERR"), WorkerLifecycleType::Failed) => err_failed += 1,
                    other => panic!("unexpected event combo: {other:?}"),
                }
            }
        }
        assert_eq!(ok_spawned, 1);
        assert_eq!(ok_completed, 1);
        assert_eq!(err_spawned, 1);
        assert_eq!(err_failed, 1);
    }

    /// T-004 regression: register() with both lineage fields set to None
    /// emits events with `parent_session_id = None` / `root_session_id = None`.
    /// This is the legacy-caller path (non-Pantheon MCP clients).
    #[tokio::test]
    async fn pantheon_lineage_none_when_caller_did_not_supply_it() {
        let metrics = Arc::new(DaemonMetrics::new().expect("metrics"));
        let (ws_tx, mut ws_rx) = broadcast::channel(64);
        let sequencer = Arc::new(EventSequencer::new());
        let tracker = TaskTracker::with_pantheon_observability(metrics, ws_tx, sequencer);

        let child = Command::new("sh").arg("-c").arg("true").spawn().expect("spawn");
        tracker
            .register(
                "T-LIN-NONE".to_string(),
                1,
                Arc::new(Mutex::new(child)),
                None,
                None,
                None,
                None,
                None,
                std::time::Instant::now(),
            )
            .await;
        let _ = tracker
            .mark_completed(
                "T-LIN-NONE",
                "abc".to_string(),
                vec![],
                String::new(),
                None,
                None,
            )
            .await;

        while let Ok(msg) = ws_rx.try_recv() {
            let outer: serde_json::Value = serde_json::from_str(&msg).expect("json");
            if outer.get("type").and_then(|v| v.as_str()) != Some("agent_stream") {
                continue;
            }
            let payload = outer.get("payload").expect("payload");
            if let Ok(AgentStreamEvent::WorkerLifecycle {
                parent_session_id,
                root_session_id,
                ..
            }) = serde_json::from_value::<AgentStreamEvent>(payload.clone())
            {
                assert!(parent_session_id.is_none(), "expected no parent");
                assert!(root_session_id.is_none(), "expected no root");
            }
        }
    }

    /// FEAT-012 (REQ-017) T-007.5 reality test:
    /// snapshot_workers must enumerate every registered task and surface
    /// the lineage fields. A stub that returns Vec::new() fails the count;
    /// a stub that hardcodes Vec::with_capacity(2).push(default) fails the
    /// per-task assertions.
    #[tokio::test]
    async fn snapshot_workers_enumerates_all_tracked_tasks_with_lineage() {
        let tracker = TaskTracker::default();

        // Register two tasks with distinct lineage.
        let child_a = Command::new("sh").arg("-c").arg("true").spawn().expect("spawn");
        tracker
            .register(
                "T-SNAP-A".to_string(),
                1,
                Arc::new(Mutex::new(child_a)),
                None,
                Some("pantheon-parent-A".to_string()),
                Some("pantheon-root-A".to_string()),
                None,
                None,
                std::time::Instant::now(),
            )
            .await;

        let child_b = Command::new("sh").arg("-c").arg("true").spawn().expect("spawn");
        tracker
            .register(
                "T-SNAP-B".to_string(),
                2,
                Arc::new(Mutex::new(child_b)),
                None,
                None, // legacy non-Pantheon caller
                None,
                None,
                None,
                std::time::Instant::now(),
            )
            .await;

        let snapshot = tracker.snapshot_workers().await;
        assert_eq!(snapshot.len(), 2, "expected 2 worker entries");

        // Build a lookup so we can assert per-task without depending on order.
        let by_id: std::collections::HashMap<&str, &shared_types::WorkerInfo> = snapshot
            .iter()
            .map(|w| (w.task_id.as_deref().unwrap_or(""), w))
            .collect();

        let a = by_id.get("T-SNAP-A").expect("T-SNAP-A in snapshot");
        assert_eq!(a.session_id, "T-SNAP-A");
        assert_eq!(a.task_id.as_deref(), Some("T-SNAP-A"));
        assert_eq!(a.agent, "codex");
        assert_eq!(a.name, "codex-worker-T-SNAP-A");
        assert_eq!(a.status, "working");
        assert_eq!(a.parent_session_id.as_deref(), Some("pantheon-parent-A"));
        assert_eq!(a.root_session_id.as_deref(), Some("pantheon-root-A"));
        assert!(a.started_at.starts_with("20"), "RFC 3339 timestamp expected");

        let b = by_id.get("T-SNAP-B").expect("T-SNAP-B in snapshot");
        assert_eq!(b.session_id, "T-SNAP-B");
        assert_eq!(b.parent_session_id, None);
        assert_eq!(b.root_session_id, None);
    }

    /// FEAT-012 (REQ-017) T-007.5 reality test:
    /// snapshot_workers reflects the post-completion status correctly,
    /// not just the initial Working state. A stub that always returns
    /// status="working" fails this.
    #[tokio::test]
    async fn snapshot_workers_reflects_post_completion_status() {
        let tracker = TaskTracker::default();
        let child = Command::new("sh").arg("-c").arg("true").spawn().expect("spawn");
        tracker
            .register(
                "T-SNAP-DONE".to_string(),
                0,
                Arc::new(Mutex::new(child)),
                None,
                None,
                None,
                None,
                None,
                std::time::Instant::now(),
            )
            .await;
        let _ = tracker
            .mark_completed(
                "T-SNAP-DONE",
                "abc123".to_string(),
                vec![],
                String::new(),
                None,
                None,
            )
            .await;
        let snap = tracker.snapshot_workers().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].status, "completed");
    }

    /// FEAT-012 (REQ-017) T-007.5 reality test: empty tracker returns empty snapshot.
    #[tokio::test]
    async fn snapshot_workers_empty_tracker_returns_empty_vec() {
        let tracker = TaskTracker::default();
        let snap = tracker.snapshot_workers().await;
        assert!(snap.is_empty());
    }

    /// Verify that the legacy constructor (without sequencer) does NOT emit
    /// WorkerLifecycle events. This is the backwards-compat guarantee.
    #[tokio::test]
    async fn legacy_observability_does_not_emit_worker_lifecycle() {
        let metrics = Arc::new(DaemonMetrics::new().expect("metrics"));
        let (ws_tx, mut ws_rx) = broadcast::channel(64);
        let tracker = TaskTracker::with_observability(metrics, Some(ws_tx));

        let child = Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .expect("spawn");
        tracker
            .register(
                "T-LEGACY-01".to_string(),
                1,
                Arc::new(Mutex::new(child)),
                None,
                None,
                None,
                None,
                None,
                std::time::Instant::now(),
            )
            .await;

        // Should still emit abe_task_state events, but no WorkerLifecycle
        let mut lifecycle_count = 0;
        while let Ok(msg) = ws_rx.try_recv() {
            let outer: serde_json::Value = serde_json::from_str(&msg).expect("valid json");
            if outer.get("type").and_then(|v| v.as_str()) == Some("agent_stream") {
                let payload = outer.get("payload").expect("payload field");
                if let Ok(AgentStreamEvent::WorkerLifecycle { .. }) =
                    serde_json::from_value::<AgentStreamEvent>(payload.clone())
                {
                    lifecycle_count += 1;
                }
            }
        }
        assert_eq!(lifecycle_count, 0, "legacy constructor should not emit WorkerLifecycle");
    }
}
