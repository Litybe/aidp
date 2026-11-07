//! `reconcile_never_fails_on_ledger_error` lives in its own integration-test
//! binary, in its own OS process, deliberately isolated from every other
//! `aidp-core` test.
//!
//! Why: `reconcile` reads pricing through `PriceTable::from_env_or_embedded`,
//! which re-reads `AIDP_MODEL_PRICES` on every call rather than caching it —
//! so this test overrides that env var to an unreadable path (via
//! `EnvGuard`, never a raw `std::env::set_var`) to assert `reconcile`
//! degrades gracefully (`source: "unavailable"`) instead of panicking or
//! erroring. The env var itself is still process-wide, so mutating it from a
//! test that shares a process with others racing on the same variable would
//! still be observable to them for the duration of the change; giving this
//! test its own `tests/*.rs` binary keeps that mutation confined to a
//! process nothing else runs in. `PriceTable`'s own load-failure behavior,
//! with no env var involved at all, is exercised directly in
//! `cost_price_table.rs`.
use std::path::PathBuf;

use aidp_core::cost::load_error;
use aidp_core::models::constants::cost::MODEL_PRICES_ENV;
use aidp_core::run::{reconcile, RunState, StageState};
use aidp_testkit::{EnvGuard, TempWorkspace};

const MISSING_PRICES_PATH: &str = "C:/definitely/does/not/exist/model-prices.json";

#[test]
fn reconcile_never_fails_on_ledger_error() {
    let _env_guard = EnvGuard::set(MODEL_PRICES_ENV, MISSING_PRICES_PATH);

    let workspace = TempWorkspace::new("reconcile-ledgererr");
    let run_dir = workspace.path().to_path_buf();

    let mk = |name: &str| StageState {
        name: name.into(),
        stage_dir: PathBuf::from(format!("/ws/{name}")),
        ..Default::default()
    };
    let mut st = RunState {
        run_id: "r-ledger-err".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        stages: vec![mk("code")],
        ..Default::default()
    };
    let result = serde_json::json!({
        "status": "done",
        "results": {
            "code": { "status": "completed", "metrics": { "tokens_used": 100 } }
        }
    });
    let outcome = reconcile(&mut st, &result);
    assert!(outcome.is_ok(), "reconcile must succeed: {:?}", outcome);
    let code = st.stages.iter().find(|s| s.name == "code").unwrap();
    let usage = code.usage.as_ref().expect("usage attached");
    assert_eq!(usage.source.as_str(), "unavailable");
    assert!(st.cost.is_none());

    let reason = load_error().expect("a set but missing override must report a load error");
    assert!(reason.contains(MODEL_PRICES_ENV), "{reason}");
    let resolved_path = std::path::absolute(MISSING_PRICES_PATH)
        .unwrap_or_else(|_| PathBuf::from(MISSING_PRICES_PATH));
    assert!(
        reason.contains(&resolved_path.display().to_string()),
        "{reason}"
    );
}
