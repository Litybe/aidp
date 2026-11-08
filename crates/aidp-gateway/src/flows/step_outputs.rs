//! Outputs a `kind: pipeline` step exposes to the flow: the top-level keys of the
//! `result.json` of the last stage the run executed (`steps.<id>.outputs.<key>`).

use crate::models::enums::native_stage_status::NativeStageStatus;
use crate::models::native_run_view::NativeRunView;
use crate::models::native_stage_view::NativeStageView;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// File a pipeline's last stage writes with its flat outputs.
pub const RESULT_FILE: &str = "result.json";
/// Output carrying why a stage stopped as `blocked`.
pub const OUTPUT_BLOCKED_REASON: &str = "blocked_reason";
const RUN_STATE_FILE: &str = "run-state.json";
const STATES_KEY: &str = "stages";
const NAME_KEY: &str = "name";
const STAGE_DIR_KEY: &str = "stage_dir";
const SHARD_MARKER: char = '#';
const FILE_SUFFIX: &str = "_file";
/// Keys `result.json` can never replace: run identity and file outputs the facts read.
const PROTECTED: &[&str] = &["runId", "runDir", "sessionId", "pr_file", "pr_body"];

/// The last stage that ran: the latest finished, shards and plan tasks left out.
fn last_stage(view: &NativeRunView) -> Option<&NativeStageView> {
    view.stages
        .iter()
        .enumerate()
        .filter(|(_, row)| !row.name.contains(SHARD_MARKER))
        .filter(|(_, row)| {
            matches!(
                row.status,
                NativeStageStatus::Done | NativeStageStatus::Failed
            )
        })
        .max_by_key(|(index, row)| (row.finished_at.clone().unwrap_or_default(), *index))
        .map(|(_, row)| row)
}

fn stage_dir(view: &NativeRunView, name: &str) -> PathBuf {
    std::fs::read_to_string(view.run_dir.join(RUN_STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|state| {
            state[STATES_KEY]
                .as_array()?
                .iter()
                .find(|row| row[NAME_KEY].as_str() == Some(name))
                .and_then(|row| row[STAGE_DIR_KEY].as_str().map(PathBuf::from))
        })
        .unwrap_or_else(|| view.run_dir.join(name))
}

fn read_object(path: &std::path::Path) -> Option<Map<String, Value>> {
    let value: Value =
        serde_json::from_str(&aidp_core::text::read_text_lenient(path).ok()?).ok()?;
    match value {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Keys of the last stage: `result.json`, else the `signals` of its reply.
pub fn last_stage_outputs(view: &NativeRunView) -> Map<String, Value> {
    let Some(row) = last_stage(view) else {
        return Map::new();
    };
    let signals = row.signals.as_ref().and_then(Value::as_object);
    let mut keys = read_object(&stage_dir(view, &row.name).join(RESULT_FILE))
        .or_else(|| signals.cloned())
        .unwrap_or_default();
    if !keys.contains_key(OUTPUT_BLOCKED_REASON) {
        if let Some(reason) = signals.and_then(|map| map.get(OUTPUT_BLOCKED_REASON)) {
            keys.insert(OUTPUT_BLOCKED_REASON.to_string(), reason.clone());
        }
    }
    keys
}

/// Adds the last stage's keys to `outputs`. A key that is also a file output (`findings`,
/// `verify`) takes the value from `result.json`; the file path moves to `<key>_file`.
pub fn merge_stage_outputs(outputs: &mut BTreeMap<String, Value>, view: &NativeRunView) {
    for (key, value) in last_stage_outputs(view) {
        if PROTECTED.contains(&key.as_str()) {
            continue;
        }
        if let Some(path) = outputs.remove(&key) {
            outputs.insert(format!("{key}{FILE_SUFFIX}"), path);
        }
        outputs.insert(key, value);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/step_outputs_tests.rs"]
mod tests;
