use aidp_core::clock::FixedClock;
use aidp_core::cost::PriceTable;
use aidp_core::git::RecordingGit;
use aidp_core::id::FixedIdGen;
use aidp_core::run::{init_with, reconcile_with, RunPorts, RunState};
use std::sync::atomic::Ordering;

const FIXED_RUN_ID: &str = "FIXED-0001";
const FIXED_TIMESTAMP: &str = "2026-01-01T00:00:00Z";
const RECORDED_HEAD: &str = "cafebabe000000000000000000000000000000ff";
const RECORDED_BRANCH: &str = "main";

const MINIMAL_PIPELINE_YAML: &str = r#"
version: 5
id: run-ports-golden
stages:
  - name: discover
    agent: analyzer
    outputs: [context.md]
"#;

fn workspace_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aidp-run-ports-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Replace every occurrence of the tempdir workspace path with a stable
/// placeholder in an already-pretty-printed `run-state.json` — `run_dir`,
/// `pipeline_path` and every stage's `stage_dir` are absolute paths rooted in
/// the tempdir, so a raw snapshot would never match between runs. Matches the
/// JSON-escaped form (backslashes doubled) since `pretty` is JSON text, and
/// folds the separators to `/` so the snapshot is the same on every OS.
fn normalize_run_dir(pretty: &str, workspace: &std::path::Path) -> String {
    let ws_escaped = workspace.to_string_lossy().replace('\\', "\\\\");
    pretty
        .replace(ws_escaped.as_str(), "<WORKSPACE>")
        .replace("\\\\", "/")
}

#[test]
fn init_with_fixed_ports_produces_the_golden_run_state() {
    let workspace = workspace_dir("golden");
    let pipeline_path = workspace.join("pipe.yaml");
    std::fs::write(&pipeline_path, MINIMAL_PIPELINE_YAML).unwrap();

    let clock = FixedClock::from_rfc3339(FIXED_TIMESTAMP);
    let ids = FixedIdGen(FIXED_RUN_ID.to_string());
    let git = RecordingGit::new();
    let prices = PriceTable::from_json("{\"pricing_version\":\"test\",\"tiers\":{}}");
    let ports = RunPorts {
        clock: &clock,
        ids: &ids,
        git: &git,
        prices: &prices,
    };

    let state = init_with(
        &pipeline_path,
        &workspace,
        serde_json::json!({}),
        None,
        &ports,
    )
    .unwrap();

    assert_eq!(state.run_id, FIXED_RUN_ID);
    assert_eq!(state.started_at, "2026-01-01T00:00:00+00:00");

    let run_state_raw = std::fs::read_to_string(state.run_dir.join("run-state.json")).unwrap();
    let written: serde_json::Value = serde_json::from_str(&run_state_raw).unwrap();
    let pretty = serde_json::to_string_pretty(&written).unwrap();
    let normalized = normalize_run_dir(&pretty, &workspace);

    insta::assert_snapshot!("run_ports__run_state_golden", normalized);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn init_with_recording_git_counts_head_and_branch_once_each_on_a_non_git_workspace() {
    let workspace = workspace_dir("git-counts");
    let pipeline_path = workspace.join("pipe.yaml");
    std::fs::write(&pipeline_path, MINIMAL_PIPELINE_YAML).unwrap();

    let clock = FixedClock::from_rfc3339(FIXED_TIMESTAMP);
    let ids = FixedIdGen(FIXED_RUN_ID.to_string());
    let mut git = RecordingGit::new();
    git.head_value = Some(RECORDED_HEAD.to_string());
    git.branch_value = Some(RECORDED_BRANCH.to_string());
    let prices = PriceTable::from_json("{\"pricing_version\":\"test\",\"tiers\":{}}");
    let ports = RunPorts {
        clock: &clock,
        ids: &ids,
        git: &git,
        prices: &prices,
    };

    let state = init_with(
        &pipeline_path,
        &workspace,
        serde_json::json!({}),
        None,
        &ports,
    )
    .unwrap();

    assert_eq!(state.base_sha.as_deref(), Some(RECORDED_HEAD));
    assert_eq!(state.branch.as_deref(), Some(RECORDED_BRANCH));
    assert_eq!(git.calls.head.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        git.calls.branch.load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

const AWAITING_GATE_RESULT: &str = r#"{"status":"awaiting_gate","gate":"plan","results":{}}"#;
const DONE_RESULT: &str = r#"{"status":"done","results":{}}"#;

fn reconcile_with_recording_git(label: &str, result: &str, base_sha: Option<&str>) -> RecordingGit {
    let workspace = workspace_dir(label);
    let mut state = RunState {
        run_id: FIXED_RUN_ID.to_string(),
        workspace_root: workspace.clone(),
        run_dir: workspace.join("run"),
        base_sha: base_sha.map(str::to_string),
        ..Default::default()
    };
    std::fs::create_dir_all(&state.run_dir).unwrap();
    let clock = FixedClock::from_rfc3339(FIXED_TIMESTAMP);
    let ids = FixedIdGen(FIXED_RUN_ID.to_string());
    let git = RecordingGit::new();
    let prices = PriceTable::from_json("{\"pricing_version\":\"test\",\"tiers\":{}}");
    let ports = RunPorts {
        clock: &clock,
        ids: &ids,
        git: &git,
        prices: &prices,
    };
    let result: serde_json::Value = serde_json::from_str(result).unwrap();
    reconcile_with(&mut state, &result, &ports).unwrap();
    let _ = std::fs::remove_dir_all(&workspace);
    git
}

/// An unfinished run with no base sha never reads head or log through the git port.
#[test]
fn reconcile_with_awaiting_gate_records_no_head_and_no_log_call() {
    let git = reconcile_with_recording_git("gate-git", AWAITING_GATE_RESULT, None);
    assert_eq!(git.calls.head.load(Ordering::SeqCst), 0);
    assert_eq!(git.calls.log.load(Ordering::SeqCst), 0);
}

/// A finished run reads head and log through the git port exactly once each.
#[test]
fn reconcile_with_done_records_one_head_and_one_log_call() {
    let git = reconcile_with_recording_git("done-git", DONE_RESULT, None);
    assert_eq!(git.calls.head.load(Ordering::SeqCst), 1);
    assert_eq!(git.calls.log.load(Ordering::SeqCst), 1);
}
