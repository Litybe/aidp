use super::*;
use crate::models::constants::doctor::{BUG_HISTORY_FILE, TEST_INFRA_FILE};
use crate::models::enums::DoctorTrigger;
use aidp_testkit::TempWorkspace;

const KEY: &str = "GH-9";
const RUN_ID: &str = "run-9";
const NOISE_LINES: usize = 500;

fn candidate() -> DoctorCandidate {
    DoctorCandidate {
        id: format!("{KEY}/implement@0"),
        key: KEY.to_string(),
        ticket: KEY.to_string(),
        trigger: DoctorTrigger::FlowFailed,
        step: Some("implement".to_string()),
        iteration: Some(0),
        run_id: Some(RUN_ID.to_string()),
        project_id: None,
        detail: String::new(),
        live: false,
        activity_ms: 0,
    }
}

#[test]
fn the_log_tail_keeps_the_lines_of_the_incident_and_the_warnings() {
    let root = TempWorkspace::new("doctor-log-tail");
    let log = root.child("daemon.err.log");
    let mut text = String::new();
    for index in 0..NOISE_LINES {
        text.push_str(&format!("INFO noise {index}\n"));
    }
    text.push_str(&format!("INFO flow {KEY} step implement failed\n"));
    text.push_str("WARN unity heartbeat stale\n");
    text.push_str("INFO other ticket\n");
    std::fs::write(&log, text).unwrap();
    let tail = filtered_log_tail(&log, &[KEY.to_string()]);
    assert_eq!(
        tail,
        format!("INFO flow {KEY} step implement failed\nWARN unity heartbeat stale")
    );
    assert_eq!(filtered_log_tail(&root.child("missing.log"), &[]), "");
}

#[test]
fn the_bundle_copies_the_flow_files_and_indexes_them() {
    let root = TempWorkspace::new("doctor-bundle");
    let flow_dir = root.child("flow");
    std::fs::create_dir_all(&flow_dir).unwrap();
    std::fs::write(flow_dir.join("state.json"), "{}").unwrap();
    std::fs::write(flow_dir.join("journal.jsonl"), "{}\n").unwrap();
    let bundle = root.child("bundle");
    let candidate = candidate();
    let files = write_bundle(
        &bundle,
        &EvidenceSource {
            candidate: &candidate,
            flow_dir: Some(flow_dir),
            run: None,
            state_dir: root.path(),
            pr_status: Some(json!({ "number": 4 })),
            unity_status: None,
            convergence: vec![
                (BUG_HISTORY_FILE, json!([{ "bug": "GH-10" }])),
                (TEST_INFRA_FILE, json!([])),
            ],
        },
    )
    .unwrap();
    let evidence = bundle.join(EVIDENCE_DIR);
    assert!(evidence.join("flow-state.json").is_file());
    assert!(evidence.join("flow-journal.jsonl").is_file());
    assert!(!evidence.join("flow-runtime.json").exists());
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(evidence.join(EVIDENCE_INDEX_FILE)).unwrap())
            .unwrap();
    assert_eq!(index[INCIDENT_KEY]["key"], KEY);
    assert_eq!(index[FILES_KEY].as_array().unwrap().len(), files.len() - 1);
    let pr: Value =
        serde_json::from_str(&std::fs::read_to_string(evidence.join(PR_FILE)).unwrap()).unwrap();
    assert_eq!(pr["number"], 4);
    let history: Value =
        serde_json::from_str(&std::fs::read_to_string(evidence.join(BUG_HISTORY_FILE)).unwrap())
            .unwrap();
    assert_eq!(history[0]["bug"], "GH-10");
    assert!(evidence.join(TEST_INFRA_FILE).is_file());
    assert!(index[FILES_KEY]
        .as_array()
        .unwrap()
        .iter()
        .any(|file| file.as_str().unwrap().ends_with(BUG_HISTORY_FILE)));
}
