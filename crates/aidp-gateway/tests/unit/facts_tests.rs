use super::{collect, parse_artifact, pull_request};
use crate::models::constants::flow_facts::{
    FACT_BODY, FACT_BRANCH, FACT_PR, FACT_PR_BODY, FACT_REPO, FACT_SHA, FACT_URL, OUTPUT_BRANCH,
    OUTPUT_PR_BODY_FILE, OUTPUT_PR_FILE, STEP_BRANCH,
};
use crate::models::constants::pr_status_keys::{KEY_HEAD, KEY_NUMBER, KEY_REPO, KEY_SHA, KEY_URL};
use aidp_flows::{FlowDef, FlowState, StepState, StepStatus, OUTPUT_REPORT};
use aidp_testkit::TempWorkspace;
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::BTreeMap;

const PR_NEWER: &str = include_str!("../fixtures/flow_facts/pr_newer.json");
const PR_OLDER: &str = include_str!("../fixtures/flow_facts/pr_older.json");
const PR_CREATED: &str = include_str!("../fixtures/flow_facts/pr_created.json");
const KEY: &str = "LOC-7";
const BRANCH: &str = "feat/LOC-7";
const REPORT: &str = "C:/vault/flow/reports/1.md";
const LIVE_SHA: &str = "ccccccc";
const NEWER_SHA: &str = "bbbbbbb";
const NEWER_NUMBER: u64 = 42;
const OLD_TICKET_NUMBER: u64 = 7;
const NEWER_URL: &str = "https://github.com/acme/app/pull/42";
const REPO: &str = "acme/app";
const T1: &str = "2026-10-08T10:00:00Z";
const T2: &str = "2026-10-08T11:00:00Z";
const STEP_DEV: &str = "dev";
const STEP_LOCAL_FIX: &str = "local_fix";
const STEP_VERDICT: &str = "verdict";
const OLDER_PR_FILE: &str = "dev-pr.json";
const NEWER_PR_FILE: &str = "fix-pr.json";
const STRING_NUMBER: &str = "42";
const PR_NUMBER_KEY: &str = "pr_number";
const NESTED_PR_KEY: &str = "pr";
const URL_KEY: &str = "url";
const STEP_PR_CREATE: &str = "publish@dev/pr_create";
const PR_BODY_FILE: &str = "pr-body.md";
const CREATED_PR_FILE: &str = "pr.json";
const CREATED_NUMBER: u64 = 7;

fn step(finished_at: &str, outputs: BTreeMap<String, Value>) -> StepState {
    StepState {
        status: StepStatus::Done,
        finished_at: Some(finished_at.to_string()),
        outputs,
        ..StepState::default()
    }
}

/// A run whose `dev` then `local_fix` wrote a `pr.json`, with a branch step and a stored report.
fn state(workspace: &TempWorkspace) -> FlowState {
    let older = workspace.child(OLDER_PR_FILE);
    let newer = workspace.child(NEWER_PR_FILE);
    std::fs::write(&older, PR_OLDER).unwrap();
    std::fs::write(&newer, PR_NEWER).unwrap();
    let mut state = FlowState::new(KEY, FlowDef::default());
    state.steps.insert(
        STEP_BRANCH.to_string(),
        step(
            T1,
            BTreeMap::from([(OUTPUT_BRANCH.to_string(), json!(BRANCH))]),
        ),
    );
    state.steps.insert(
        STEP_DEV.to_string(),
        step(
            T1,
            BTreeMap::from([(OUTPUT_PR_FILE.to_string(), json!(older))]),
        ),
    );
    state.steps.insert(
        STEP_LOCAL_FIX.to_string(),
        step(
            T2,
            BTreeMap::from([(OUTPUT_PR_FILE.to_string(), json!(newer))]),
        ),
    );
    state.steps.insert(
        STEP_VERDICT.to_string(),
        step(
            T2,
            BTreeMap::from([(OUTPUT_REPORT.to_string(), json!(REPORT))]),
        ),
    );
    state
}

fn wanted(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn live_status(number: u64) -> Value {
    json!({ KEY_NUMBER: number, KEY_URL: NEWER_URL, KEY_REPO: REPO, KEY_HEAD: { KEY_SHA: LIVE_SHA } })
}

#[test]
fn the_branch_and_report_come_from_the_run_without_reading_the_pr() {
    let workspace = TempWorkspace::new("facts_branch");
    let read = Cell::new(false);
    let facts = collect(
        &state(&workspace),
        &wanted(&[FACT_BRANCH, FACT_BODY]),
        || {
            read.set(true);
            None
        },
    );
    assert_eq!(facts[FACT_BRANCH], json!(BRANCH));
    assert_eq!(facts[FACT_BODY], json!(REPORT));
    assert!(!read.get(), "no pull request fact was wanted");
}

#[test]
fn the_run_pr_json_names_the_pull_request_and_the_live_head_sha_wins() {
    let workspace = TempWorkspace::new("facts_same_pr");
    let facts = collect(
        &state(&workspace),
        &wanted(&[FACT_PR, FACT_URL, FACT_REPO, FACT_SHA]),
        || Some(live_status(NEWER_NUMBER)),
    );
    assert_eq!(facts[FACT_PR], json!(NEWER_NUMBER));
    assert_eq!(facts[FACT_URL], json!(NEWER_URL));
    assert_eq!(facts[FACT_REPO], json!(REPO));
    assert_eq!(facts[FACT_SHA], json!(LIVE_SHA));
}

#[test]
fn a_live_status_of_another_pull_request_is_ignored() {
    let workspace = TempWorkspace::new("facts_other_pr");
    let facts = collect(&state(&workspace), &wanted(&[FACT_PR, FACT_SHA]), || {
        Some(live_status(OLD_TICKET_NUMBER))
    });
    assert_eq!(facts[FACT_PR], json!(NEWER_NUMBER));
    assert_eq!(facts[FACT_SHA], json!(NEWER_SHA));
}

#[test]
fn without_a_pr_json_the_live_status_answers() {
    let facts = collect(
        &FlowState::new(KEY, FlowDef::default()),
        &wanted(&[FACT_PR, FACT_SHA]),
        || Some(live_status(OLD_TICKET_NUMBER)),
    );
    assert_eq!(facts[FACT_PR], json!(OLD_TICKET_NUMBER));
    assert_eq!(facts[FACT_SHA], json!(LIVE_SHA));
    assert_eq!(facts.len(), 2, "only the wanted facts are returned");
}

#[test]
fn an_unknown_fact_stays_absent() {
    let facts = collect(
        &FlowState::new(KEY, FlowDef::default()),
        &wanted(&[FACT_PR, FACT_BRANCH]),
        || None,
    );
    assert!(facts.is_empty());
}

#[test]
fn a_pr_json_is_read_leniently() {
    let from_string = parse_artifact(&json!({ PR_NUMBER_KEY: STRING_NUMBER }));
    assert_eq!(from_string.number, Some(NEWER_NUMBER));
    let nested = parse_artifact(&json!({ NESTED_PR_KEY: { URL_KEY: NEWER_URL } }));
    assert_eq!(nested.number, Some(NEWER_NUMBER));
    assert_eq!(nested.repository.as_deref(), Some(REPO));
    assert_eq!(nested.url.as_deref(), Some(NEWER_URL));
}

#[test]
fn the_view_pull_request_is_the_newest_pr_json() {
    let workspace = TempWorkspace::new("facts_view");
    let pr = pull_request(&state(&workspace)).unwrap();
    assert_eq!(pr.number, NEWER_NUMBER);
    assert_eq!(pr.repository.as_deref(), Some(REPO));
    assert!(pull_request(&FlowState::new(KEY, FlowDef::default())).is_none());
}

/// A run whose `dev` wrote a PR body, then whose publish block opened the PR and wrote `pr.json`.
fn published_state(workspace: &TempWorkspace) -> FlowState {
    let body = workspace.child(PR_BODY_FILE);
    let created = workspace.child(CREATED_PR_FILE);
    std::fs::write(&created, PR_CREATED).unwrap();
    let mut state = FlowState::new(KEY, FlowDef::default());
    state.steps.insert(
        STEP_DEV.to_string(),
        step(
            T1,
            BTreeMap::from([(OUTPUT_PR_BODY_FILE.to_string(), json!(body))]),
        ),
    );
    state.steps.insert(
        STEP_PR_CREATE.to_string(),
        step(
            T2,
            BTreeMap::from([(OUTPUT_PR_FILE.to_string(), json!(created))]),
        ),
    );
    state
}

/// The PR body a pipeline wrote fills `{pr_body}`; the PR the host opened fills `{pr}` and the view.
#[test]
fn the_host_opened_pull_request_is_the_flow_pull_request() {
    let workspace = TempWorkspace::new("facts_published");
    let state = published_state(&workspace);
    let facts = collect(&state, &wanted(&[FACT_PR_BODY, FACT_PR]), || None);
    assert_eq!(facts[FACT_PR_BODY], json!(workspace.child(PR_BODY_FILE)));
    assert_eq!(facts[FACT_PR], json!(CREATED_NUMBER));
    let pr = pull_request(&state).unwrap();
    assert_eq!(pr.number, CREATED_NUMBER);
    assert_eq!(pr.repository.as_deref(), Some(REPO));
}
