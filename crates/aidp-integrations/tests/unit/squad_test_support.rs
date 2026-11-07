use crate::models::squad_pr::SquadPr;
use crate::models::squad_review::SquadReview;
use crate::pr::{Mergeable, ReviewDecision, ReviewState, StatusContextState};
use crate::squad::fetch::GhRunner;
use serde_json::{json, Map, Value};
use std::sync::Mutex;

pub const LIST_ONE: &str = include_str!("fixtures/squad/list_ok_1.json");
pub const LIST_TWO: &str = include_str!("fixtures/squad/list_ok_2.json");
pub const DETAIL: &str = include_str!("fixtures/squad/detail_ok.json");
pub const TEAM_REPOS: &str = include_str!("fixtures/squad/team_repos.json");
pub const ME: &str = "alice";
pub const HEAD: &str = "head-sha";
pub const OLD_HEAD: &str = "old-sha";
pub const NOW: &str = "2026-10-03T09:00:00Z";
pub const TWO_DAYS_LATER: &str = "2026-10-05T09:00:00Z";
/// Repository holding the first fixture pull request, and the one holding the second.
pub const FIRST_PR_REPO_NAME: &str = "r00";
pub const SECOND_PR_REPO_NAME: &str = "r10";
const NAME_VARIABLE_PREFIX: &str = "n";
const ALIAS_PREFIX: &str = "r";
const FIELD_SEPARATOR: char = '=';
const PR_NODES_POINTER: &str = "/data/r0/pullRequests/nodes";
const FIRST_DETAIL_POINTER: &str = "/data/r0/pullRequest";
const SECOND_DETAIL_POINTER: &str = "/data/r1/pullRequest";
/// Text only a detail query holds: it reads one pull request by number.
pub const DETAIL_QUERY_MARKER: &str = "pullRequest(number";
pub const BAD_GATEWAY: &str = "HTTP 502: Bad Gateway";
pub const FORBIDDEN: &str = "HTTP 403: Resource not accessible";

pub type Responder = Box<dyn Fn(&[String]) -> Result<String, String> + Send + Sync>;

pub struct FakeRunner {
    pub calls: Mutex<Vec<Vec<String>>>,
    pub responder: Responder,
}

impl FakeRunner {
    pub fn new(responder: Responder) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            responder,
        }
    }

    pub fn call_count(&self) -> usize {
        self.calls().len()
    }

    pub fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl GhRunner for FakeRunner {
    fn run(&self, args: &[String]) -> Result<String, String> {
        self.calls.lock().unwrap().push(args.to_vec());
        (self.responder)(args)
    }
}

pub fn review(author: &str, state: ReviewState, commit: &str, at: &str) -> SquadReview {
    SquadReview {
        author: author.to_string(),
        state,
        commit_oid: Some(commit.to_string()),
        submitted_at: Some(at.to_string()),
    }
}

pub fn pr_by(author: &str) -> SquadPr {
    SquadPr {
        repo: "acme/svc".to_string(),
        number: 7,
        title: "Change".to_string(),
        url: "https://github.com/acme/svc/pull/7".to_string(),
        is_draft: false,
        author: author.to_string(),
        created_at: "2026-09-01T10:00:00Z".to_string(),
        updated_at: "2026-09-02T10:00:00Z".to_string(),
        base_ref: "master".to_string(),
        head_ref: "feat/change".to_string(),
        head_sha: HEAD.to_string(),
        mergeable: Mergeable::Mergeable,
        review_decision: None,
        review_requests: Vec::new(),
        my_reviews: Vec::new(),
        commits: 1,
        checks_state: Some(StatusContextState::Success),
        unresolved_threads: 0,
        comments: 0,
        labels: Vec::new(),
        recent_comments: Vec::new(),
    }
}

pub fn with_reviews(mut pr: SquadPr, reviews: Vec<SquadReview>) -> SquadPr {
    pr.my_reviews = reviews;
    pr
}

pub fn draft_of(author: &str) -> SquadPr {
    let mut pr = pr_by(author);
    pr.is_draft = true;
    pr
}

pub fn ready_to_merge_pr() -> SquadPr {
    let mut pr = pr_by(ME);
    pr.review_decision = Some(ReviewDecision::Approved);
    pr
}

pub fn requested_pr() -> SquadPr {
    let mut pr = pr_by("bob");
    pr.review_requests = vec![ME.to_string()];
    pr
}

pub fn red_checks_pr() -> SquadPr {
    let mut pr = pr_by(ME);
    pr.checks_state = Some(StatusContextState::Failure);
    pr
}

pub fn conflicting_pr() -> SquadPr {
    let mut pr = pr_by(ME);
    pr.mergeable = Mergeable::Conflicting;
    pr
}

pub fn changes_requested_on_mine() -> SquadPr {
    let mut pr = pr_by(ME);
    pr.review_decision = Some(ReviewDecision::ChangesRequested);
    pr
}

pub fn open_discussion_pr() -> SquadPr {
    let mut pr = with_reviews(
        pr_by("bob"),
        vec![review(ME, ReviewState::Commented, HEAD, "t1")],
    );
    pr.unresolved_threads = 2;
    pr
}

pub fn repo_names(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("acme/r{index:02}"))
        .collect()
}

/// True for the arguments of a detail query.
pub fn is_detail(args: &[String]) -> bool {
    args.iter().any(|arg| arg.contains(DETAIL_QUERY_MARKER))
}

/// The repository names a query aliases, by alias index: `n3=r07` gives `(3, "r07")`.
fn aliased_names(args: &[String]) -> Vec<(usize, String)> {
    args.iter()
        .filter_map(|arg| {
            let (key, value) = arg.split_once(FIELD_SEPARATOR)?;
            let index = key.strip_prefix(NAME_VARIABLE_PREFIX)?.parse().ok()?;
            Some((index, value.to_string()))
        })
        .collect()
}

fn fixture_at(fixture: &str, pointer: &str) -> Value {
    let value: Value = serde_json::from_str(fixture).unwrap();
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

/// What one aliased repository answers: its pull requests in a listing, or the pull
/// request asked for in a detail query.
fn answer_for(name: &str, detail: bool) -> Value {
    match (name, detail) {
        (FIRST_PR_REPO_NAME, false) => {
            json!({ "pullRequests": { "nodes": fixture_at(LIST_ONE, PR_NODES_POINTER) } })
        }
        (SECOND_PR_REPO_NAME, false) => {
            json!({ "pullRequests": { "nodes": fixture_at(LIST_TWO, PR_NODES_POINTER) } })
        }
        (FIRST_PR_REPO_NAME, true) => {
            json!({ "pullRequest": fixture_at(DETAIL, FIRST_DETAIL_POINTER) })
        }
        (SECOND_PR_REPO_NAME, true) => {
            json!({ "pullRequest": fixture_at(DETAIL, SECOND_DETAIL_POINTER) })
        }
        (_, false) => json!({ "pullRequests": { "nodes": [] } }),
        (_, true) => json!({ "pullRequest": null }),
    }
}

/// Answers every listing and detail query by repository name, whatever the batch sizes:
/// `r00` holds pull request 11, `r10` pull request 12, every other repository none.
pub fn graphql_responder() -> Responder {
    Box::new(|args| {
        let detail = is_detail(args);
        let data: Map<String, Value> = aliased_names(args)
            .into_iter()
            .map(|(index, name)| (format!("{ALIAS_PREFIX}{index}"), answer_for(&name, detail)))
            .collect();
        Ok(json!({ "data": data }).to_string())
    })
}

pub fn team_runner() -> FakeRunner {
    FakeRunner::new(Box::new(|_| {
        let names: Vec<String> = serde_json::from_str(TEAM_REPOS).unwrap();
        Ok(names.join("\n"))
    }))
}
