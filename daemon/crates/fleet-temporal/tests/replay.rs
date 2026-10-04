//! Replay gate for FleetWorkflow (PLAN Phase 3: replay tests gate every change to a deployed
//! workflow definition).
//!
//! The fixture is the real history of a two-stub fleet run against the homebox server
//! (temporal-migration scripts/exit-checks/fleet-stage-c.sh, 2026-10-04). Replaying it runs the
//! CURRENT workflow code over that history: a change that reorders, adds or removes a command
//! (an activity, a timer) fails here instead of breaking fleets in flight after a deploy.
//!
//! The negative control registers a deliberately different workflow under the same type name and
//! must FAIL replay. Without it, a replay test that could never fail would look like a gate.

use std::time::Duration;

use fleet_temporal::fleet_workflow::{FleetInput, FleetResult, FleetWorkflow};
use temporalio_client::WorkflowHistory;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, WorkflowContext, WorkflowResult,
    workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions},
};

fn fixture() -> WorkflowHistory {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fleet-two-stubs-history.json"))
        .expect("fixture");
    WorkflowHistory::from_json(&bytes).expect("history json")
}

#[tokio::test]
async fn fleet_workflow_replays_its_recorded_history() {
    let opts = WorkflowReplayerOptions::new()
        .register_workflow::<FleetWorkflow>()
        .expect("register")
        .build();
    let replayer = WorkflowReplayer::new(opts).expect("replayer");
    replayer
        .replay_workflow(fixture())
        .await
        .expect("FleetWorkflow must replay its own recorded history deterministically");
}

/// A changed workflow under the same type name: it skips prepare and runs a different activity
/// first. Replay MUST report it incompatible.
#[workflow]
#[derive(Default)]
pub struct ChangedFleetWorkflow;

#[workflow_methods]
impl ChangedFleetWorkflow {
    #[run(name = "triumvirate-fleet")]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: FleetInput) -> WorkflowResult<FleetResult> {
        let _ = ctx
            .execute_activity(
                fleet_temporal::fleet_workflow::FleetLedgerActivities::finalize_fleet,
                input,
                ActivityOptions::start_to_close_timeout(Duration::from_secs(10)),
            )
            .await?;
        Ok(FleetResult { ledger_state: "x".to_string(), members: Vec::new() })
    }
}

#[tokio::test]
async fn a_changed_workflow_fails_replay() {
    let opts = WorkflowReplayerOptions::new()
        .register_workflow::<ChangedFleetWorkflow>()
        .expect("register")
        .build();
    let replayer = WorkflowReplayer::new(opts).expect("replayer");
    let err = replayer
        .replay_workflow(fixture())
        .await
        .expect_err("a workflow that schedules a different activity must fail replay");
    // It must fail for the RIGHT reason: incompatibility with the history, not a setup error.
    let text = format!("{err:?}").to_lowercase();
    eprintln!("replay error: {text}");
    assert!(text.contains("nondetermin"), "expected a nondeterminism failure, got: {text}");
}

/// Capture a fixture (live server; run explicitly):
/// `FIXTURE_WORKFLOW_ID=fleet-... cargo test -p fleet-temporal --test replay capture -- --ignored`
/// Writes the history through the Rust client's own `to_json`, the format `from_json` reads.
/// The CLI's `workflow show -o json` is protobuf canonical JSON and does not load.
#[tokio::test]
#[ignore = "talks to the live server; run explicitly to refresh the fixture"]
async fn capture() {
    let id = std::env::var("FIXTURE_WORKFLOW_ID").expect("FIXTURE_WORKFLOW_ID");
    let cfg = fleet_temporal::WorkerConfig::from_env().expect("cfg");
    let client = fleet_temporal::connect(&cfg).await.expect("connect");
    let json = client
        .get_workflow_handle::<FleetWorkflow>(id)
        .fetch_history(Default::default())
        .to_json()
        .await
        .expect("history");
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fleet-two-stubs-history.json"), json)
        .expect("write fixture");
}
