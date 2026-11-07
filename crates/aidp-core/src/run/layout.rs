//! Run directory layout, feature keys, load and save.

use super::RunState;
use crate::pipeline::Pipeline;
use crate::vault::sanitize_segment;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// File the driver writes the Workflow result object to, inside the run directory.
pub const RUN_STATE_FILE: &str = "run-state.json";
pub const INPUT_TARGET: &str = "target";
pub const INPUT_JIRA_KEY: &str = "jira_key";
pub const INPUT_FEATURE: &str = "feature";
pub const INPUT_JIRA_KEYS: &str = "jira_keys";
pub const INPUT_TARGETS: &str = "targets";
pub(crate) const NO_TICKET_SEGMENT: &str = "_no-ticket";

/// Locate a run directory by id in the ticket-grouped layout, else the older flat one.
pub fn run_dir_for_in(
    vault: Option<&crate::vault::Vault>,
    workspace_root: &Path,
    run_id: &str,
) -> PathBuf {
    if let Some(v) = vault {
        if let Some(dir) = lookup_in_tree(&v.pipelines_dir(), run_id) {
            return dir;
        }
    }
    for root in workspace_root.ancestors() {
        if let Some(dir) = lookup_run_dir(root, run_id) {
            return dir;
        }
    }
    workspace_root.join(".aidp/pipelines").join(run_id)
}

/// Resolve `<root>/.aidp/pipelines/**/<run_id>/run-state.json` — flat layout first, then the
/// one-level-deep ticket buckets. `None` when this root does not hold the run.
fn lookup_run_dir(root: &Path, run_id: &str) -> Option<PathBuf> {
    lookup_in_tree(&root.join(".aidp/pipelines"), run_id)
}

/// Find `<base>/**/<run_id>/run-state.json`, bounded to the depths the layouts use.
fn lookup_in_tree(base: &Path, run_id: &str) -> Option<PathBuf> {
    const MAX_DEPTH: usize = 3;
    let direct = base.join(run_id);
    if direct.join(RUN_STATE_FILE).is_file() {
        return Some(direct);
    }
    let mut frontier = vec![base.to_path_buf()];
    for _ in 1..MAX_DEPTH {
        let mut next = Vec::new();
        for dir in &frontier {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for e in rd.flatten() {
                if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let p = e.path();
                let cand = p.join(run_id);
                if cand.join(RUN_STATE_FILE).is_file() {
                    return Some(cand);
                }
                next.push(p);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    None
}

/// The human-facing **feature** identifier for a run, derived from its inputs.
pub fn feature_key(inputs: &BTreeMap<String, serde_json::Value>) -> Option<String> {
    if let Some(seg) = inputs
        .get(INPUT_FEATURE)
        .and_then(|v| v.as_str())
        .and_then(sanitize_segment)
    {
        return Some(seg);
    }
    for list_key in [INPUT_JIRA_KEYS, INPUT_TARGETS] {
        if let Some(arr) = inputs.get(list_key).and_then(|v| v.as_array()) {
            let mut keys: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str())
                .filter_map(sanitize_segment)
                .collect();
            keys.sort();
            keys.dedup();
            match keys.len() {
                0 => continue,
                1 => return Some(keys.remove(0)),
                n => {
                    let joined = keys.join("+");
                    return Some(if joined.len() <= 64 && n <= 4 {
                        joined
                    } else {
                        format!("{}+{}more", keys[0], n - 1)
                    });
                }
            }
        }
    }
    for single_key in [INPUT_TARGET, INPUT_JIRA_KEY] {
        if let Some(seg) = inputs
            .get(single_key)
            .and_then(|v| v.as_str())
            .and_then(sanitize_segment)
        {
            return Some(seg);
        }
    }
    None
}

/// Filesystem-safe grouping segment for a run's shared root.
pub(super) fn ticket_segment(
    inputs: &BTreeMap<String, serde_json::Value>,
    pipe: Option<&Pipeline>,
) -> String {
    feature_key(inputs)
        .or_else(|| {
            pipe.and_then(|p| p.inputs.get(INPUT_FEATURE))
                .and_then(|spec| spec.default.as_ref())
                .and_then(|v| v.as_str())
                .and_then(crate::vault::sanitize_segment)
        })
        .unwrap_or_else(|| NO_TICKET_SEGMENT.to_string())
}

/// Load a run's persisted state.
pub fn load_in(
    vault: Option<&crate::vault::Vault>,
    workspace_root: &Path,
    run_id: &str,
) -> anyhow::Result<RunState> {
    use anyhow::Context;
    let run_state_path = run_dir_for_in(vault, workspace_root, run_id).join(RUN_STATE_FILE);
    let run_state_raw = std::fs::read_to_string(&run_state_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow::Error::new(crate::Error::NotFound(format!(
                "{} {}",
                crate::models::constants::messages::FAILED_TO_READ_RUN_STATE,
                run_state_path.display()
            )))
        } else {
            anyhow::Error::new(error).context(format!(
                "{} {}",
                crate::models::constants::messages::FAILED_TO_READ_RUN_STATE,
                run_state_path.display()
            ))
        }
    })?;
    let state = serde_json::from_str(&run_state_raw)
        .with_context(|| format!("failed to parse run state {}", run_state_path.display()))?;
    Ok(state)
}

/// Persist `run-state.json` only — no note generation.
pub fn save_state(state: &RunState) -> anyhow::Result<()> {
    let p = state.run_dir.join(RUN_STATE_FILE);
    std::fs::create_dir_all(&state.run_dir)?;
    std::fs::write(p, serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

/// Persist `run-state.json`, then refresh the Obsidian notes.
pub fn save(state: &RunState) -> anyhow::Result<()> {
    save_state(state)?;
    if let Err(error) = crate::obsidian::refresh(state) {
        tracing::warn!(%error, "obsidian notes not refreshed");
    }
    Ok(())
}
