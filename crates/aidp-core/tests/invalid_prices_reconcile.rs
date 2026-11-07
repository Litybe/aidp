//! `reconcile` reads pricing through `PriceTable::from_env_or_embedded`,
//! which re-reads `AIDP_MODEL_PRICES` (via `EnvGuard`, never a raw
//! `std::env::set_var`) on every call. This binary points that env var at a
//! file that is not JSON and checks that `reconcile` degrades exactly as it
//! does when the price file is missing, in a process of its own so the
//! mutation cannot race another test's read of the same variable.
//! `PriceTable`'s own load-failure behavior, with no env var involved at
//! all, is exercised directly in `cost_price_table.rs`.

use std::path::PathBuf;

use aidp_core::cost::load_error;
use aidp_core::models::constants::cost::MODEL_PRICES_ENV;
use aidp_core::run::{reconcile, RunCost, RunState, StageState, StageStatus};
use aidp_testkit::{EnvGuard, TempWorkspace};

const CLAUDE_PROJECTS_DIR_ENV: &str = "AIDP_CLAUDE_PROJECTS_DIR";
const INVALID_PRICES_FIXTURE: &str = "tests/fixtures/prices/invalid-prices.json";
const RECONCILE_RESULT: &str = include_str!("fixtures/prices/reconcile-result.json");
const EMPTY_PROJECTS_DIR: &str = "empty-claude-projects";
const WORKSPACE_TAG: &str = "reconcile-invalid-prices";
const RUN_ID: &str = "r-invalid-prices";
const STALE_PRICING_VERSION: &str = "stale-2026-01-01";
const STALE_CAPTURED_AT: &str = "2026-01-01T00:00:00Z";
const PIPELINE_ID: &str = "dev";
const PIPELINE_PATH: &str = "/nonexistent.yaml";
const STARTED_AT: &str = "t";
const STAGE_NAME: &str = "code";
const UNAVAILABLE_SOURCE: &str = "unavailable";

/// Absolute path of the fixture holding a price grid that is not valid JSON.
fn invalid_prices_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(INVALID_PRICES_FIXTURE)
        .to_string_lossy()
        .into_owned()
}

/// A run with a single stage whose directory lives under `run_dir`.
fn single_stage_run(run_dir: PathBuf) -> RunState {
    let stage = StageState {
        name: STAGE_NAME.into(),
        stage_dir: run_dir.join(STAGE_NAME),
        ..Default::default()
    };
    RunState {
        run_id: RUN_ID.into(),
        pipeline_path: PathBuf::from(PIPELINE_PATH),
        pipeline_id: PIPELINE_ID.into(),
        workspace_root: run_dir.clone(),
        run_dir,
        started_at: STARTED_AT.into(),
        stages: vec![stage],
        ..Default::default()
    }
}

#[test]
fn reconcile_degrades_on_an_invalid_price_grid() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let projects_dir = workspace.child(EMPTY_PROJECTS_DIR);
    std::fs::create_dir_all(&projects_dir).expect("create the empty projects directory");
    let prices_path = invalid_prices_path();
    let projects_path = projects_dir.to_string_lossy().into_owned();
    let _env_guard = EnvGuard::with(&[
        (MODEL_PRICES_ENV, Some(prices_path.as_str())),
        (CLAUDE_PROJECTS_DIR_ENV, Some(projects_path.as_str())),
    ]);

    let mut state = single_stage_run(workspace.path().to_path_buf());
    // An earlier, successful reconcile left a run-level cost behind; this
    // pass's broken grid must clear it, not leave it sitting next to
    // `"unavailable"` usage.
    state.cost = Some(RunCost {
        total_usd: Some(1.23),
        tokens_billable: 456,
        pricing_version: Some(STALE_PRICING_VERSION.into()),
        captured_at: STALE_CAPTURED_AT.into(),
        by_model: Default::default(),
    });
    let result: serde_json::Value =
        serde_json::from_str(RECONCILE_RESULT).expect("reconcile result fixture is JSON");

    let outcome = reconcile(&mut state, &result);

    assert!(outcome.is_ok(), "reconcile must succeed: {outcome:?}");
    let stage = state
        .stages
        .iter()
        .find(|stage| stage.name == STAGE_NAME)
        .expect("the reconciled stage is still present");
    assert_eq!(stage.status, StageStatus::Success);
    let usage = stage.usage.as_ref().expect("usage attached");
    assert_eq!(usage.source.as_str(), UNAVAILABLE_SOURCE);
    assert!(state.cost.is_none(), "no cost without a valid price grid");
    let reason = load_error().expect("the invalid grid reports a load error");
    assert!(
        reason.contains(MODEL_PRICES_ENV),
        "the load error names {MODEL_PRICES_ENV}: {reason}"
    );
}
