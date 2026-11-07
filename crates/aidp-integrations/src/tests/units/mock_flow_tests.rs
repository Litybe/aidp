use super::*;

const WORKFLOW_FILE: &str = include_str!("fixtures/mock_flow/workflow_file.json");
const SEED_URL: &str = "/__seed";
const STATE_URL: &str = "/__state";
const INJECT_URL: &str = "/__inject_failure";
const REVIEWERS_URL: &str = "/github/repos/o/r/pulls/4/requested_reviewers";
const CONTENTS_URL: &str = "/github/repos/o/r/contents/.github/workflows/d.yml?ref=main";
const MISSING_CONTENTS_URL: &str = "/github/repos/o/r/contents/x.yml";
const REVIEWERS_BODY: &str = r#"{"reviewers":["bob"],"team_reviewers":["core"]}"#;
const SINGLE_REVIEWER_BODY: &str = r#"{"reviewers":["bob"]}"#;
const FAILURE_RULE: &str = r#"{"path_contains":"requested_reviewers","status":502,"remaining":1}"#;
const EXPECTED_ENCODED_CONTENT: &str = "b246IHdvcmtmbG93X2Rpc3BhdGNo";
const TEAM_SLUG: &str = "core";

/// Sends one request through the router.
fn call(state: &Mutex<State>, method: Method, url: &str, body: &str) -> (u16, Value) {
    handle(&method, url, body, state).expect("route must answer")
}

/// Posts a seed body and asserts it was accepted.
fn seed(state: &Mutex<State>, body: &str) {
    let (code, _) = call(state, Method::Post, SEED_URL, body);
    assert_eq!(code, 200);
}

/// Requesting the same reviewers twice keeps one entry of each.
#[test]
fn requested_reviewers_post_is_idempotent() {
    let state = Mutex::new(State::default());
    call(&state, Method::Post, REVIEWERS_URL, REVIEWERS_BODY);
    let (code, _) = call(&state, Method::Post, REVIEWERS_URL, REVIEWERS_BODY);
    assert_eq!(code, 201);
    let (_, view) = call(&state, Method::Get, REVIEWERS_URL, "");
    assert_eq!(view["users"].as_array().map(Vec::len), Some(1));
    assert_eq!(view["teams"][0]["slug"], TEAM_SLUG);
}

/// Workflow file contents are served base64 encoded, and an unknown path is a 404.
#[test]
fn workflow_file_contents_are_base64() {
    let state = Mutex::new(State::default());
    seed(&state, WORKFLOW_FILE);
    let (code, body) = call(&state, Method::Get, CONTENTS_URL, "");
    assert_eq!(code, 200);
    assert_eq!(body["content"], EXPECTED_ENCODED_CONTENT);
    let (missing, _) = call(&state, Method::Get, MISSING_CONTENTS_URL, "");
    assert_eq!(missing, 404);
}

/// An injected failure hits the flow routes, is recorded, and expires after its quota.
#[test]
fn injected_failure_applies_to_new_routes_and_expires() {
    let state = Mutex::new(State::default());
    call(&state, Method::Post, INJECT_URL, FAILURE_RULE);
    let (failed, _) = call(&state, Method::Post, REVIEWERS_URL, SINGLE_REVIEWER_BODY);
    assert_eq!(failed, 502);
    let (_, dump) = call(&state, Method::Get, STATE_URL, "");
    assert_eq!(dump["gh_writes"]["requested_reviewers"], 0);
    assert_eq!(dump["requests"].as_array().map(Vec::len), Some(1));
    let (recovered, _) = call(&state, Method::Post, REVIEWERS_URL, SINGLE_REVIEWER_BODY);
    assert_eq!(recovered, 201);
}
