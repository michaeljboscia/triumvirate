//! The fleet engine on Temporal (temporal-migration design `docs/designs/triumvirate-fleet.md`).
//!
//! Off unless `TRIUMVIRATE_FLEET_ENGINE=temporal`. With it on, `triumvirate daemon` runs a
//! Temporal worker on queue `triumvirate-fleet` in namespace `triumvirate`, over mTLS with the
//! `triumvirate-worker` client cert. Today it registers only [`PingWorkflow`], the liveness probe
//! that proves the plumbing; `FleetWorkflow` lands on top of it.
//!
//! `Worker::run()` returns a future that is NOT `Send` (verified against temporalio-sdk 1.0.0,
//! not stated in the docs), so the worker cannot go on the daemon's multi-thread runtime via
//! `tokio::spawn`. It gets its own OS thread with a current-thread runtime.

pub mod worker;
pub mod fleet_workflow;

use std::{path::PathBuf, str::FromStr, time::Duration};

use temporalio_client::{
    Client, ClientOptions, ClientTlsOptions, Connection, ConnectionOptions, TlsOptions,
};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, ApplicationFailure, Runtime, Worker, WorkerOptions, WorkflowContext, WorkflowResult,
    activities::{ActivityContext, ActivityError},
};
use worker::{Blocked, RunWorkerInput, RunWorkerOutput, Start};

fn non_retryable(b: Blocked) -> ActivityError {
    ActivityError::Application(Box::new(ApplicationFailure::non_retryable(b.0)))
}

/// Whether the daemon runs the Temporal fleet engine. Default: legacy.
pub fn engine_enabled() -> bool {
    std::env::var("TRIUMVIRATE_FLEET_ENGINE").is_ok_and(|v| v.trim().eq_ignore_ascii_case("temporal"))
}

/// Where and as whom the worker connects. Every field has an env override and a production
/// default (the homebox stack in temporal-migration/deploy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    pub address: String,
    pub namespace: String,
    pub task_queue: String,
    pub cert_dir: PathBuf,
    /// The name the server's TLS certificate is issued for.
    pub tls_domain: String,
}

/// homebox's LAN address first, then its tailnet address (the `temporal-tailnet` relay). At home
/// the LAN answers; away, or while macOS Local Network privacy has not approved this binary for
/// the LAN (it blocks a launchd process with "No route to host"), the tailnet does.
pub const DEFAULT_ADDRESSES: &str = "https://192.168.2.110:7233,https://100.73.45.3:7233";

/// How long one address may take to connect before the next is tried. Away from home the LAN
/// address does not answer at all, so without a bound the fallback would wait out a TCP timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

impl WorkerConfig {
    /// `address` as an ordered list: comma-separated, tried first to last.
    pub fn addresses(&self) -> Vec<&str> {
        self.address.split(',').map(str::trim).filter(|a| !a.is_empty()).collect()
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| d.to_string());
        let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
        Ok(Self {
            address: var("TRIUMVIRATE_TEMPORAL_ADDRESS", DEFAULT_ADDRESSES),
            namespace: var("TRIUMVIRATE_TEMPORAL_NAMESPACE", "triumvirate"),
            task_queue: var("TRIUMVIRATE_TEMPORAL_TASK_QUEUE", "triumvirate-fleet"),
            cert_dir: PathBuf::from(var(
                "TRIUMVIRATE_TEMPORAL_CERT_DIR",
                &format!("{home}/.temporal/triumvirate-worker"),
            )),
            tls_domain: var("TRIUMVIRATE_TEMPORAL_TLS_DOMAIN", "temporal"),
        })
    }
}

/// Connect a client over mTLS. Fails loudly on a missing cert file rather than falling back to
/// plain TLS: the server refuses a client without a cert, and the error should name the file.
pub async fn connect(cfg: &WorkerConfig) -> anyhow::Result<Client> {
    let read = |name: &str| {
        let p = cfg.cert_dir.join(name);
        std::fs::read(&p).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", p.display()))
    };
    let tls = TlsOptions::builder()
        .server_root_ca_cert(read("ca.cert")?)
        .domain(cfg.tls_domain.clone())
        .client_tls_options(
            ClientTlsOptions::builder()
                .client_cert(read("client.pem")?)
                .client_private_key(read("client.key")?)
                .build(),
        )
        .build();
    let host = std::env::var("HOST").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "mac".to_string());
    let mut errors = Vec::new();
    for address in cfg.addresses() {
        let conn = ConnectionOptions::new(url::Url::from_str(address)?)
            .identity(format!("triumvirate-daemon-{}@{host}", std::process::id()))
            .tls_options(tls.clone())
            .build();
        match tokio::time::timeout(CONNECT_TIMEOUT, Connection::connect(conn)).await {
            Ok(Ok(connection)) => {
                tracing::info!(address, "temporal: connected");
                return Ok(Client::new(connection, ClientOptions::new(cfg.namespace.clone()).build())?);
            }
            Ok(Err(e)) => {
                let reason = format!("{address}: {e}");
                tracing::warn!(address, error = %reason, "temporal: address failed; trying the next");
                errors.push(reason);
            }
            Err(_) => {
                let reason = format!("{address}: no answer in {} s", CONNECT_TIMEOUT.as_secs());
                tracing::warn!(address, error = %reason, "temporal: address failed; trying the next");
                errors.push(reason);
            }
        }
    }
    anyhow::bail!("no Temporal address answered: {}", errors.join("; "))
}

/// Liveness probe: a client starts it on `triumvirate-fleet` and gets back the worker's identity.
/// If this completes, the daemon's worker is polling, connected and executing activities.
#[workflow]
#[derive(Default)]
pub struct PingWorkflow;

#[workflow_methods]
impl PingWorkflow {
    #[run(name = "triumvirate-ping")]
    pub async fn run(ctx: &mut WorkflowContext<Self>, nonce: String) -> WorkflowResult<String> {
        let pong = ctx
            .execute_activity(
                FleetActivities::pong,
                nonce,
                ActivityOptions::start_to_close_timeout(Duration::from_secs(10)),
            )
            .await?;
        Ok(pong)
    }
}

/// Runs one fleet member through `run_worker` and returns its result. The vehicle for the
/// stage B checks (completion, cancel, adoption after a daemon kill) and a building block of
/// FleetWorkflow.
#[workflow]
#[derive(Default)]
pub struct RunWorkerProbeWorkflow;

#[workflow_methods]
impl RunWorkerProbeWorkflow {
    #[run(name = "triumvirate-run-worker-probe")]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: RunWorkerInput) -> WorkflowResult<RunWorkerOutput> {
        let out = ctx
            .execute_activity(FleetActivities::run_worker, input, run_worker_options())
            .await?;
        Ok(out)
    }
}

/// `maximumAttempts: 2` with every agent error non-retryable (skill rule 4): the second attempt
/// fires only when the first was lost (a dead daemon), and that attempt adopts rather than reruns.
/// The heartbeat timeout is how a dead daemon is noticed, and how cancellation reaches the activity.
pub fn run_worker_options() -> ActivityOptions {
    ActivityOptions::with_start_to_close_timeout(fleet::orchestrator::MEMBER_WALL_LIMIT)
        .heartbeat_timeout(Duration::from_secs(20))
        .retry_policy(
            temporalio_common::RetryPolicy::builder()
                .initial_interval(Duration::from_secs(1))
                .maximum_attempts(2)
                .build(),
        )
        .build()
}

pub struct FleetActivities;

#[activities]
impl FleetActivities {
    #[activity(name = "triumvirate-pong")]
    pub async fn pong(_ctx: ActivityContext, nonce: String) -> Result<String, ActivityError> {
        Ok(format!("pong {nonce} from triumvirate daemon pid {}", std::process::id()))
    }

    /// One fleet member's agent run. See `worker` for the fresh, adopt and finished paths.
    #[activity(name = "triumvirate-run-worker")]
    pub async fn run_worker(ctx: ActivityContext, input: RunWorkerInput) -> Result<RunWorkerOutput, ActivityError> {
        let root = PathBuf::from(&input.project_root);
        let (mut child, token, adopted) = match worker::decide(&root, &input.fleet_id, &input.task_id) {
            Ok(Start::Finished(done)) => {
                tracing::info!(fleet_id = %input.fleet_id, task_id = %input.task_id, "run_worker: already finished; returning its record");
                return Ok(worker::output(&input, done, true));
            }
            Ok(Start::Adopt(t)) => {
                tracing::warn!(fleet_id = %input.fleet_id, task_id = %input.task_id, pid = t.pid, "run_worker: adopting a running worker");
                (None, t, true)
            }
            Ok(Start::Fresh) => {
                let (c, t) = worker::launch(&input).await.map_err(non_retryable)?;
                let adopted = c.is_none();
                (c, t, adopted)
            }
            Err(b) => return Err(non_retryable(b)),
        };
        // Output growth is the progress signal (design section 4a). On adoption the clock starts
        // now: the silence before this attempt is unknown, not evidence.
        let stall = worker::stall_window(&input.agent);
        let mut last_bytes = worker::output_bytes(&root, &input);
        let mut last_growth = std::time::Instant::now();
        loop {
            if let Some(done) = worker::finished(&root, &input, &token) {
                if let Some(c) = child.as_mut() {
                    let _ = c.wait().await;
                }
                return Ok(worker::output(&input, done, adopted));
            }
            let ended = match child.as_mut() {
                Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                None => token.verify() != fleet::worker_token::Verification::Live,
            };
            if ended {
                if let Some(done) = worker::finished(&root, &input, &token) {
                    return Ok(worker::output(&input, done, adopted));
                }
                // Killed from outside without a record. Retryable: the next attempt finds the
                // token Gone and starts fresh, which is safe because nothing of ours runs.
                return Err(ApplicationFailure::new(format!(
                    "worker for {} ended without a completion record (killed?)",
                    input.task_id
                ))
                .into());
            }
            tokio::select! {
                _ = ctx.cancelled() => {
                    let stopped = stop_worker(&token, child.as_mut()).await;
                    match stopped {
                        Ok(Ok(outcome)) => {
                            tracing::warn!(fleet_id = %input.fleet_id, task_id = %input.task_id, ?outcome, "run_worker: cancelled; worker group stopped");
                            return Err(ActivityError::cancelled());
                        }
                        // A worker that could not be proven stopped is NOT reported cancelled
                        // (Codex, review of 3871850): the workflow's abort then refuses to mark
                        // the fleet cancelled either.
                        Ok(Err(e)) => {
                            return Err(non_retryable(Blocked(format!("cancel could not stop the worker for {}: {e}", input.task_id))));
                        }
                        Err(e) => {
                            return Err(non_retryable(Blocked(format!("cancel task failed for {}: {e}", input.task_id))));
                        }
                    }
                }
                _ = tokio::time::sleep(worker::POLL) => {
                    let bytes = worker::output_bytes(&root, &input);
                    if bytes != last_bytes {
                        last_bytes = bytes;
                        last_growth = std::time::Instant::now();
                    }
                    if let Some(window) = stall
                        && last_growth.elapsed() > window
                        // The record may have landed during the sleep: a finished run is not a stall.
                        && worker::finished(&root, &input, &token).is_none()
                    {
                        let silent = last_growth.elapsed().as_secs();
                        let stopped = stop_worker(&token, child.as_mut()).await;
                        // The agent may have finished between the check above and the stop (Codex,
                        // review of PR 57). A done record with no signal is the agent's own exit,
                        // not our stop: that run completed and is judged like any other.
                        if let Some(done) = worker::finished(&root, &input, &token)
                            && done.signal.is_none()
                        {
                            tracing::info!(task_id = %input.task_id, "run_worker: finished as the stall stop began; recording its own exit");
                            return Ok(worker::output(&input, done, adopted));
                        }
                        tracing::warn!(fleet_id = %input.fleet_id, task_id = %input.task_id, agent = %input.agent, silent, ?stopped, "run_worker: stalled; worker group stopped");
                        // Non-retryable: a retry would rerun the same task from scratch, and the
                        // caller decides what a stall means (design 4a).
                        return Err(non_retryable(Blocked(match stopped {
                            Ok(Ok(_)) => format!("stalled after {silent} s without output growth (window {} s); worker stopped", window.as_secs()),
                            Ok(Err(e)) => format!("stalled after {silent} s without output growth, and the worker could not be stopped: {e}"),
                            Err(e) => format!("stalled after {silent} s without output growth, and the stop task failed: {e}"),
                        })));
                    }
                    if let Err(e) = ctx.record_heartbeat(input.task_id.clone()).await {
                        tracing::warn!(task_id = %input.task_id, error = %e, "run_worker: heartbeat failed");
                    }
                }
            }
        }
    }
}

/// Stop a worker's process group, reaping our own shim child WHILE it stops.
///
/// Reaping only afterwards left the shim a zombie for the whole wait: on macOS a group whose only
/// member is an unreaped zombie answers kill(-pgid, 0) with EPERM, which `group_exists` reads as
/// alive, so terminate waited out the grace, escalated, got EPERM again and reported a stopped
/// worker as unstoppable (found by the stall exit check, 2026-10-04).
async fn stop_worker(
    token: &fleet::worker_token::WorkerToken,
    child: Option<&mut tokio::process::Child>,
) -> Result<Result<fleet::worker_token::TerminateOutcome, String>, tokio::task::JoinError> {
    let t = token.clone();
    let grace = fleet::orchestrator::fleet_kill_grace();
    let term = tokio::task::spawn_blocking(move || t.terminate(grace));
    let reap = async {
        if let Some(c) = child {
            let _ = tokio::time::timeout(grace + Duration::from_secs(10), c.wait()).await;
        }
    };
    let (stopped, ()) = tokio::join!(term, reap);
    stopped
}

fn worker_options(cfg: &WorkerConfig) -> anyhow::Result<WorkerOptions> {
    Ok(WorkerOptions::new(cfg.task_queue.clone())
        .register_workflow::<PingWorkflow>()?
        .register_workflow::<RunWorkerProbeWorkflow>()?
        .register_workflow::<fleet_workflow::FleetWorkflow>()?
        .register_activities(FleetActivities)
        .register_activities(fleet_workflow::FleetLedgerActivities)
        .build())
}

/// Run the worker until it fails. One attempt: the caller owns retry.
async fn run_worker_once(cfg: &WorkerConfig) -> anyhow::Result<()> {
    let runtime = Runtime::from_current_tokio(Default::default())?;
    let client = connect(cfg).await?;
    let mut worker = Worker::new(&runtime, client, worker_options(cfg)?)?;
    tracing::info!(
        namespace = %cfg.namespace,
        task_queue = %cfg.task_queue,
        addresses = %cfg.address,
        "temporal fleet worker polling"
    );
    worker.run().await?;
    Ok(())
}

/// Start the worker on its own thread and keep it running: a connect or poll failure is logged
/// and retried with backoff (5 s doubling to 60 s), never silently abandoned. The no-pollers
/// absence alert on this queue is what catches a worker that cannot get back up.
pub fn spawn_worker_thread(cfg: WorkerConfig) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("temporal-fleet-worker".to_string())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(error = %e, "temporal fleet worker: cannot build its runtime; engine is DOWN");
                    return;
                }
            };
            rt.block_on(async move {
                let mut backoff = Duration::from_secs(5);
                loop {
                    let ran = std::time::Instant::now();
                    match run_worker_once(&cfg).await {
                        Ok(()) => tracing::warn!("temporal fleet worker stopped; restarting"),
                        Err(e) => tracing::error!(error = %e, retry_in_s = backoff.as_secs(), "temporal fleet worker failed"),
                    }
                    // A run that was healthy for a while earns a fast reconnect again; only a run
                    // that keeps failing at once backs off toward the cap (Codex, review).
                    if ran.elapsed() > Duration::from_secs(120) {
                        backoff = Duration::from_secs(5);
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            });
        })
}

/// Start a fleet on the Temporal engine (the MCP `fleet_spawn` path when the flag is on). The
/// caller has already generated the fleet ID, recorded the restart index and marked the fleet
/// Temporal-owned. With `wait`, returns the final ledger state; otherwise "spawning".
pub async fn start_fleet(
    cfg: &WorkerConfig,
    input: fleet_workflow::FleetInput,
    wait: bool,
) -> anyhow::Result<String> {
    use temporalio_client::{WorkflowGetResultOptions, WorkflowStartOptions};
    let client = connect(cfg).await?;
    let handle = client
        .start_workflow(
            fleet_workflow::FleetWorkflow::run,
            input.clone(),
            WorkflowStartOptions::new(cfg.task_queue.clone(), input.fleet_id.clone()).build(),
        )
        .await?;
    if !wait {
        return Ok("spawning".to_string());
    }
    let result = handle.get_result(WorkflowGetResultOptions::default()).await?;
    Ok(result.ledger_state)
}

/// Cancel a Temporal-engine fleet. Its workflow's cleanup stops the verified workers and marks the
/// ledger cancelled; this call only requests it.
pub async fn cancel_fleet(cfg: &WorkerConfig, fleet_id: &str) -> anyhow::Result<()> {
    let client = connect(cfg).await?;
    client
        .get_workflow_handle::<fleet_workflow::FleetWorkflow>(fleet_id.to_string())
        .cancel(Default::default())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test owns every env var it touches, so nothing races.
    #[test]
    fn config_defaults_and_overrides() {
        let keys = [
            "TRIUMVIRATE_FLEET_ENGINE",
            "TRIUMVIRATE_TEMPORAL_ADDRESS",
            "TRIUMVIRATE_TEMPORAL_NAMESPACE",
            "TRIUMVIRATE_TEMPORAL_TASK_QUEUE",
            "TRIUMVIRATE_TEMPORAL_CERT_DIR",
        ];
        for k in keys {
            unsafe { std::env::remove_var(k) };
        }
        // RED IF: the engine turns on by default. Installing the binary must change nothing.
        assert!(!engine_enabled());
        let d = WorkerConfig::from_env().expect("defaults");
        assert_eq!(d.address, DEFAULT_ADDRESSES);
        assert_eq!(d.namespace, "triumvirate");
        assert_eq!(d.task_queue, "triumvirate-fleet");
        assert!(d.cert_dir.ends_with(".temporal/triumvirate-worker"));
        assert_eq!(d.tls_domain, "temporal");

        unsafe {
            std::env::set_var("TRIUMVIRATE_FLEET_ENGINE", " Temporal ");
            std::env::set_var("TRIUMVIRATE_TEMPORAL_TASK_QUEUE", "triumvirate-fleet-exitcheck");
        }
        assert!(engine_enabled());
        assert_eq!(WorkerConfig::from_env().expect("cfg").task_queue, "triumvirate-fleet-exitcheck");
        unsafe { std::env::set_var("TRIUMVIRATE_FLEET_ENGINE", "legacy") };
        assert!(!engine_enabled());
        for k in keys {
            unsafe { std::env::remove_var(k) };
        }
    }

    /// A missing cert fails loudly and names the file; it never falls back to plain TLS.
    #[tokio::test]
    async fn a_missing_cert_is_a_loud_error_naming_the_file() {
        let cfg = WorkerConfig {
            address: "https://127.0.0.1:1".to_string(),
            namespace: "triumvirate".to_string(),
            task_queue: "q".to_string(),
            cert_dir: PathBuf::from("/nonexistent/triumvirate-worker"),
            tls_domain: "temporal".to_string(),
        };
        let Err(err) = connect(&cfg).await else { panic!("connect must fail without certs") };
        let err = err.to_string();
        assert!(err.contains("/nonexistent/triumvirate-worker/ca.cert"), "{err}");
    }
}

#[cfg(test)]
mod address_tests {
    use super::*;

    /// RED IF the LAN stops being tried first, or the tailnet fallback drops out of the default.
    #[test]
    fn the_default_tries_the_lan_then_the_tailnet() {
        let cfg = WorkerConfig {
            address: DEFAULT_ADDRESSES.to_string(),
            namespace: String::new(),
            task_queue: String::new(),
            cert_dir: PathBuf::new(),
            tls_domain: String::new(),
        };
        assert_eq!(cfg.addresses(), vec!["https://192.168.2.110:7233", "https://100.73.45.3:7233"]);
        let one = WorkerConfig { address: " https://h:1 , ,https://g:2".to_string(), ..cfg };
        assert_eq!(one.addresses(), vec!["https://h:1", "https://g:2"]);
    }
}

