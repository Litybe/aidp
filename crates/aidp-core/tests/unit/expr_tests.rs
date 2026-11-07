use super::*;
use crate::run::RunState;

use std::collections::BTreeMap;
use std::path::PathBuf;

fn pipe_from(yaml: &str) -> Pipeline {
    serde_yaml::from_str(yaml).unwrap()
}

fn state_with(run_dir: PathBuf, inputs: serde_json::Value) -> RunState {
    let inputs_map: BTreeMap<String, serde_json::Value> = match inputs {
        serde_json::Value::Object(m) => m.into_iter().collect(),
        _ => BTreeMap::new(),
    };
    RunState {
        run_id: "test-run".into(),
        pipeline_path: run_dir.join("effective-pipeline.yaml"),
        pipeline_id: "t".into(),
        workspace_root: run_dir.clone(),
        run_dir,
        started_at: "now".into(),
        inputs: inputs_map,
        ..Default::default()
    }
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aidp-expr-collect-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

const UNPARSEABLE_WHEN: &str = r#"
version: 5
id: t
stages:
  - name: gatekeeper
    when: "{{ inputs.count > 'not-a-number' }}"
"#;

const PARSEABLE_WHEN: &str = r#"
version: 5
id: t
inputs:
  track: { kind: enum, values: [quick, standard], default: standard }
stages:
  - name: gatekeeper
    when: "{{ inputs.track == 'standard' }}"
"#;

#[test]
fn unparseable_when_collects_one_warning_naming_the_stage() {
    let pipe = pipe_from(UNPARSEABLE_WHEN);
    let st = state_with(tmp("unparseable"), serde_json::json!({}));
    let stage = pipe.stages.first().unwrap();
    let mut warnings = Vec::new();
    let ran = eval_when_collect(stage, &st, &pipe, &mut warnings);

    assert!(ran, "an unparseable when: must still fail open (run)");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("gatekeeper"));
}

#[test]
fn parseable_when_collects_no_warning() {
    let pipe = pipe_from(PARSEABLE_WHEN);
    let st = state_with(tmp("parseable"), serde_json::json!({"track": "standard"}));
    let stage = pipe.stages.first().unwrap();
    let mut warnings = Vec::new();
    let ran = eval_when_collect(stage, &st, &pipe, &mut warnings);

    assert!(ran);
    assert!(warnings.is_empty());
}
