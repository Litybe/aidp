//! What a flow run knows about its ticket workspace and pull request: the values
//! its actions' placeholders take when a step's `with` leaves them out.

use crate::models::constants::flow_facts::{
    FACT_BODY, FACT_BRANCH, FACT_PR, FACT_PR_BODY, FACT_REPO, FACT_SHA, FACT_URL, GITHUB_URL_HOST,
    OUTPUT_BRANCH, OUTPUT_PR_ACTION_FILE, OUTPUT_PR_BODY_FILE, OUTPUT_PR_FILE, PR_FACTS,
    PR_FILE_BRANCH_KEYS, PR_FILE_NESTED_KEY, PR_FILE_NUMBER_KEYS, PR_FILE_REPO_KEYS,
    PR_FILE_SHA_KEYS, PR_FILE_URL_KEYS, PULL_URL_MARKER, REPO_SEPARATOR, STEP_BRANCH,
};
use crate::models::constants::pr_status_keys::{KEY_HEAD, KEY_NUMBER, KEY_REPO, KEY_SHA, KEY_URL};
use crate::models::flow_pull_request::FlowPullRequest;
use crate::models::pr_artifact::PrArtifact;
use crate::models::pr_status_source::PrStatusSource;
use aidp_flows::{FlowState, StepState, OUTPUT_REPORT};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// The values of `output` across the run's steps, most recently finished first.
fn outputs_newest_first<'a>(state: &'a FlowState, output: &str) -> Vec<&'a Value> {
    let mut steps: Vec<&StepState> = state
        .steps
        .values()
        .filter(|step| step.outputs.contains_key(output))
        .collect();
    steps.sort_by(|a, b| b.finished_at.cmp(&a.finished_at));
    steps
        .into_iter()
        .filter_map(|step| step.outputs.get(output))
        .collect()
}

/// The first of `keys` holding a non-empty string in `value`.
fn first_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key)?.as_str())
        .find(|text| !text.trim().is_empty())
        .map(str::to_string)
}

/// The first of `keys` holding a number, as a JSON number or a string.
fn first_number(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| match value.get(*key)? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    })
}

/// The number and `owner/name` a GitHub pull request URL names.
fn from_pull_url(url: &str) -> (Option<u64>, Option<String>) {
    let Some((head, tail)) = url.split_once(PULL_URL_MARKER) else {
        return (None, None);
    };
    let number = tail
        .split(|character: char| !character.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok());
    let repository = head
        .split_once(GITHUB_URL_HOST)
        .map(|(_, repository)| repository.trim_matches(REPO_SEPARATOR).to_string())
        .filter(|repository| repository.contains(REPO_SEPARATOR));
    (number, repository)
}

/// A `pr.json` read leniently: no schema governs it, so names, nesting and number types vary.
pub fn parse_artifact(value: &Value) -> PrArtifact {
    let nested = value.get(PR_FILE_NESTED_KEY).filter(|pr| pr.is_object());
    let layers: Vec<&Value> = std::iter::once(value).chain(nested).collect();
    let text = |keys: &[&str]| layers.iter().find_map(|layer| first_text(layer, keys));
    let url = text(PR_FILE_URL_KEYS);
    let (url_number, url_repository) = url.as_deref().map(from_pull_url).unwrap_or_default();
    PrArtifact {
        number: layers
            .iter()
            .find_map(|layer| first_number(layer, PR_FILE_NUMBER_KEYS))
            .or(url_number),
        repository: text(PR_FILE_REPO_KEYS).or(url_repository),
        branch: text(PR_FILE_BRANCH_KEYS),
        commit_sha: text(PR_FILE_SHA_KEYS),
        url,
    }
}

fn read_artifact(path: &Path) -> Option<PrArtifact> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some(parse_artifact(&value))
}

/// The newest `pr.json` a pipeline step of the run points at that names a pull request.
pub fn pr_artifact(state: &FlowState) -> Option<PrArtifact> {
    let mut paths = outputs_newest_first(state, OUTPUT_PR_FILE);
    paths.extend(outputs_newest_first(state, OUTPUT_PR_ACTION_FILE));
    paths
        .into_iter()
        .filter_map(Value::as_str)
        .filter_map(|path| read_artifact(Path::new(path)))
        .find(|artifact| artifact.number.is_some())
}

/// The pull request the run opened, for the flow view.
pub fn pull_request(state: &FlowState) -> Option<FlowPullRequest> {
    let artifact = pr_artifact(state)?;
    Some(FlowPullRequest {
        number: artifact.number?,
        url: artifact.url,
        repository: artifact.repository,
    })
}

/// The live status of the ticket's pull request, `None` past `timeout`: a hung `gh`
/// must not keep the step running.
pub fn status_within(
    source: Arc<dyn PrStatusSource>,
    key: &str,
    timeout: Duration,
) -> Option<Value> {
    let (sender, receiver) = mpsc::channel();
    let key = key.to_string();
    std::thread::spawn(move || {
        let _ = sender.send(source.status(&key));
    });
    receiver.recv_timeout(timeout).ok().flatten()
}

fn insert_present(facts: &mut BTreeMap<String, Value>, name: &str, value: Option<Value>) {
    if let Some(value) = value.filter(|value| !value.is_null()) {
        facts.entry(name.to_string()).or_insert(value);
    }
}

/// The `wanted` placeholders from the branch step, the stored report, the newest PR body, this
/// run's `pr.json`, then the live status (`live`), trusted only for the run's own pull request.
pub fn collect(
    state: &FlowState,
    wanted: &[String],
    live: impl FnOnce() -> Option<Value>,
) -> BTreeMap<String, Value> {
    let wants = |name: &str| wanted.iter().any(|candidate| candidate == name);
    let mut facts = BTreeMap::new();
    let branch = state
        .steps
        .get(STEP_BRANCH)
        .and_then(|step| step.outputs.get(OUTPUT_BRANCH))
        .filter(|value| value.is_string())
        .cloned();
    insert_present(&mut facts, FACT_BRANCH, branch);
    let report = outputs_newest_first(state, OUTPUT_REPORT)
        .into_iter()
        .find(|value| value.is_string())
        .cloned();
    insert_present(&mut facts, FACT_BODY, report);
    let pr_body = outputs_newest_first(state, OUTPUT_PR_BODY_FILE)
        .into_iter()
        .find(|value| value.is_string())
        .cloned();
    insert_present(&mut facts, FACT_PR_BODY, pr_body);
    if PR_FACTS.iter().any(|name| wants(name)) {
        let artifact = pr_artifact(state).unwrap_or_default();
        insert_present(
            &mut facts,
            FACT_PR,
            artifact.number.map(|number| json!(number)),
        );
        for (name, text) in [
            (FACT_URL, artifact.url),
            (FACT_REPO, artifact.repository),
            (FACT_BRANCH, artifact.branch),
        ] {
            insert_present(&mut facts, name, text.map(Value::String));
        }
        let needs_live = wants(FACT_SHA)
            || PR_FACTS
                .iter()
                .any(|name| wants(name) && !facts.contains_key(*name));
        if needs_live {
            if let Some(status) = live() {
                let live_number = status.get(KEY_NUMBER).and_then(Value::as_u64);
                if artifact.number.is_none() || artifact.number == live_number {
                    insert_present(&mut facts, FACT_PR, status.get(KEY_NUMBER).cloned());
                    insert_present(&mut facts, FACT_URL, status.get(KEY_URL).cloned());
                    insert_present(&mut facts, FACT_REPO, status.get(KEY_REPO).cloned());
                    let head_sha = status.get(KEY_HEAD).and_then(|head| head.get(KEY_SHA));
                    insert_present(&mut facts, FACT_SHA, head_sha.cloned());
                }
            }
        }
        insert_present(&mut facts, FACT_SHA, artifact.commit_sha.map(Value::String));
    }
    facts.retain(|name, _| wants(name));
    facts
}

#[cfg(test)]
#[path = "../../tests/unit/facts_tests.rs"]
mod tests;
