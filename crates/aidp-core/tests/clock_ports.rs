use aidp_core::clock::FixedClock;
use aidp_core::findings::{merge_with, MergeInput};
use aidp_core::kg::{ensure_with, KnowledgeGraphSettings, KnowledgeGraphStatus, Refresh};
use aidp_core::process_runner::{
    ProcessRunner, ProcessSpec, ScriptedOutput, ScriptedProcessRunner,
};
use aidp_testkit::TempWorkspace;
use std::path::PathBuf;

/// Environment variable that overrides the graphify binary lookup.
const GRAPHIFY_BIN_ENV: &str = "AIDP_GRAPHIFY_BIN";
/// Bare program name standing in for graphify; the scripted runner never executes it.
const SCRIPTED_GRAPHIFY_BIN: &str = "graphify-scripted";

fn findings_fixture(name: &str) -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/findings"
    ))
    .join(name)
}

#[test]
fn findings_merge_with_fixed_clock_stamps_a_deterministic_timestamp() {
    let clock = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    let inputs = vec![
        MergeInput {
            label: "correctness".to_string(),
            path: findings_fixture("correctness.json"),
        },
        MergeInput {
            label: "security".to_string(),
            path: findings_fixture("security.json"),
        },
    ];

    let outcome = merge_with(&inputs, &clock).unwrap();

    assert_eq!(outcome.findings.generated_at, "2026-01-01T00:00:00+00:00");
}

#[test]
fn kg_ensure_with_scripted_runner_reports_fresh_without_graphify_on_path() {
    let ws = TempWorkspace::new("kg-ensure-with-scripted");
    std::fs::write(ws.path().join("a.rs"), "fn main() {}").unwrap();

    let cache_dir = ws.path().join(".cursor/graphify-out");
    std::fs::create_dir_all(&cache_dir).unwrap();
    std::fs::write(
        cache_dir.join("graph.json"),
        r#"{"node_count":3,"communities_count":1}"#,
    )
    .unwrap();

    let runner = ScriptedProcessRunner::new().with_output(ScriptedOutput {
        status_code: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
    });
    let clock = FixedClock::from_rfc3339("2026-02-02T00:00:00Z");
    let settings = KnowledgeGraphSettings {
        enabled: true,
        refresh: Refresh::Always,
        ..KnowledgeGraphSettings::default()
    };

    std::env::set_var(GRAPHIFY_BIN_ENV, SCRIPTED_GRAPHIFY_BIN);
    let state = ensure_with(&settings, ws.path(), None, &clock, &runner);
    std::env::remove_var(GRAPHIFY_BIN_ENV);

    assert_eq!(state.status, KnowledgeGraphStatus::Fresh);
    assert_eq!(state.built_at.as_deref(), Some("2026-02-02T00:00:00Z"));
}

#[test]
fn scripted_process_runner_output_replays_the_canned_reply() {
    let runner = ScriptedProcessRunner::new().with_output(ScriptedOutput {
        status_code: 0,
        stdout: b"ok".to_vec(),
        stderr: Vec::new(),
    });

    let out = runner.output(&ProcessSpec::new("graphify")).unwrap();

    assert!(out.status.success());
    assert_eq!(out.stdout, b"ok");
}
