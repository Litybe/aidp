//! Handler behaviour against a fake `gh`, and the read-only gate on the five action routes.

use super::*;
use crate::bus::Bus;
use crate::http::{serve_on, HttpConfig};
use crate::prs::PrStatusCache;
use aidp_core::clock::SystemClock;
use aidp_testkit::{ProcShim, TempWorkspace};
use aidp_tickets::model::{Autopilot, IssueType, Pr, Priority, Source};
use serde_json::json;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;

const GRAPHQL_OPEN: &str = include_str!(
    "../../../aidp-integrations/src/tests/units/fixtures/pr/graphql_open_pending.json"
);
const GH: &str = "gh";
const REPO: &str = "acme/widgets";
const SERVICE: &str = "widgets";
const PR_NUMBER: u64 = 42;
const TICKET_KEY: &str = "LOC-77";
const HEAD_SHA: &str = "abc123";
const PR_URL: &str = "https://github.com/acme/widgets/pull/42";
const TODO_DIR: &str = "TODO";
const CONFIG_FILE: &str = "config.yaml";
const WRITE_BACK_CONFIG: &str = "jira:\n  write_back:\n    transitions: true\n  status_map: { pr-ready: \"In Review\", done: \"Done\" }\n";
const CHECKS_PATH: &str =
    "/repository/pullRequest/commits/nodes/0/commit/statusCheckRollup/contexts/nodes";
const THREADS_PATH: &str = "/repository/pullRequest/reviewThreads/nodes";
const SQUASH_KEY: &str = "allow_squash_merge";
const THREAD_ONE: &str = "thread-1";
const THREAD_TWO: &str = "thread-2";
const PR_MERGE_ARG: &str = "merge";
const STATE_DIR_PREFIX: &str = "gw-pr-actions-state";
const DEFAULTS_SEEDED_FILE: &str = ".defaults-seeded";
const LOOPBACK_HOST: &str = "127.0.0.1";
const STATUS_FORBIDDEN: u16 = 403;
const ACTION_ROUTES: [&str; 5] = [
    "/api/prs/ready",
    "/api/prs/update-branch",
    "/api/prs/merge",
    "/api/prs/threads/reply",
    "/api/prs/threads/resolve",
];

struct Fixture {
    _vault: TempWorkspace,
    _shim_dir: TempWorkspace,
    shim: ProcShim,
    cache: PrStatusCache,
    bus: Bus,
    vault_root: PathBuf,
}

impl Fixture {
    fn new(gh_stdout: &str, write_back: bool) -> Self {
        let vault = TempWorkspace::new("gw-pr-actions-vault");
        let shim_dir = TempWorkspace::new("gw-pr-actions-shim");
        let shim = ProcShim::new(GH, &shim_dir);
        shim.set_stdout(gh_stdout);
        let vault_root = vault.path().to_path_buf();
        if write_back {
            let todo = vault_root.join(TODO_DIR);
            fs::create_dir_all(&todo).unwrap();
            fs::write(todo.join(CONFIG_FILE), WRITE_BACK_CONFIG).unwrap();
        }
        Fixture {
            _vault: vault,
            _shim_dir: shim_dir,
            shim,
            cache: PrStatusCache::new(Box::new(SystemClock)),
            bus: Bus::new(),
            vault_root,
        }
    }

    fn context(&self) -> PrActionContext<'_> {
        PrActionContext {
            cache: &self.cache,
            bus: &self.bus,
            vault_root: &self.vault_root,
        }
    }

    fn seed_ticket(&self, source: Source, status: Status) {
        let now = chrono::Utc::now();
        let mut ticket = Ticket {
            key: TICKET_KEY.to_string(),
            source,
            r#type: IssueType::Task,
            summary: "sample".to_string(),
            status,
            priority: Priority::Medium,
            priority_rank: 0,
            service: SERVICE.to_string(),
            template: None,
            track: None,
            labels: Vec::new(),
            components: Vec::new(),
            epic: None,
            assignee: None,
            created: now,
            updated: now,
            github: None,
            route: None,
            flow: None,
            milestone: None,
            autopilot: Autopilot {
                pr: Some(Pr {
                    url: PR_URL.to_string(),
                    number: Some(PR_NUMBER),
                    state: None,
                    commented: false,
                }),
                ..Autopilot::default()
            },
            tags: Vec::new(),
        };
        TicketStore::open(&self.vault_root)
            .unwrap()
            .save(&mut ticket, "", &serde_yaml::Mapping::new())
            .unwrap();
    }

    fn prime_cache(&self) {
        self.cache
            .get(REPO, PR_NUMBER, false, || Ok(json!({})))
            .unwrap();
    }

    fn events_of(&self, event_type: &str) -> Vec<Value> {
        self.bus
            .recent(100, 0)
            .into_iter()
            .filter(|event| event["type"] == event_type)
            .collect()
    }

    fn gh_called_with(&self, needle: &str) -> bool {
        self.shim
            .calls()
            .iter()
            .any(|call| call.iter().any(|arg| arg == needle))
    }
}

fn mergeable_stdout() -> String {
    let mut data: Value = serde_json::from_str(GRAPHQL_OPEN).unwrap();
    let passing = json!([{"__typename": "CheckRun", "name": "build", "status": "COMPLETED",
        "conclusion": "SUCCESS", "detailsUrl": "u", "startedAt": "2026-09-28T09:00:00Z"}]);
    *data.pointer_mut(CHECKS_PATH).unwrap() = passing;
    *data.pointer_mut(THREADS_PATH).unwrap() = json!([]);
    json!({ "data": data, SQUASH_KEY: true }).to_string()
}

/// Unwraps a handler result, showing the route error's status and body on failure.
fn succeed(result: Result<Value, PrRouteError>) -> Value {
    result.unwrap_or_else(|error| panic!("{} {}", error.status(), error.body()))
}

fn merge_request() -> MergeRequest {
    MergeRequest {
        repo: REPO.to_string(),
        number: PR_NUMBER,
        expected_head_sha: HEAD_SHA.to_string(),
    }
}

/// A merged PR moves its in-review ticket to done as the human, journaled, and publishes `pr.merged`.
#[test]
fn merge_moves_the_in_review_ticket_to_done() {
    let fixture = Fixture::new(&mergeable_stdout(), false);
    fixture.seed_ticket(Source::Local, Status::InReview);
    fixture.prime_cache();

    let body = succeed(handle_merge(&fixture.context(), merge_request()));

    assert_eq!(body["merged"], true);
    assert_eq!(body["ticket"], TICKET_KEY);
    assert!(
        fixture.gh_called_with(PR_MERGE_ARG),
        "{:?}",
        fixture.shim.calls()
    );
    let store = TicketStore::open(&fixture.vault_root).unwrap();
    let (ticket, _, _) = store.get(TICKET_KEY).unwrap();
    assert_eq!(ticket.status, Status::Done);
    let journal = ledger::read(&fixture.vault_root, Some(TICKET_KEY), 10);
    let last = journal.last().unwrap();
    assert_eq!(last.from, Some(Status::InReview));
    assert_eq!(last.to, Status::Done);
    assert_eq!(last.actor, Actor::Human);
    assert_eq!(last.reason.as_deref(), Some(MERGED_FROM_VIEWER_REASON));
}

/// The merge publishes `pr.merged` carrying the ticket key and drops the cached PR status.
#[test]
fn merge_publishes_the_event_and_invalidates_the_status_cache() {
    let fixture = Fixture::new(&mergeable_stdout(), false);
    fixture.seed_ticket(Source::Local, Status::InReview);
    fixture.prime_cache();
    assert_eq!(fixture.cache.cached_count(), 1);

    succeed(handle_merge(&fixture.context(), merge_request()));

    let merged = fixture.events_of(EVENT_MERGED);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0]["data"]["ticket"], TICKET_KEY);
    assert_eq!(merged[0]["data"]["repo"], REPO);
    assert_eq!(merged[0]["data"]["number"], PR_NUMBER);
    assert_eq!(fixture.cache.cached_count(), 0);
    assert_eq!(fixture.events_of(EVENT_TICKET_STATUS).len(), 1);
}

/// A merge with no ticket behind the PR still succeeds and reports a null ticket.
#[test]
fn merge_without_a_ticket_succeeds_with_a_null_ticket() {
    let fixture = Fixture::new(&mergeable_stdout(), false);

    let body = succeed(handle_merge(&fixture.context(), merge_request()));

    assert_eq!(body["merged"], true);
    assert!(body["ticket"].is_null());
}

/// Resolving threads answers one result per id, in order, and publishes the event.
#[test]
fn threads_resolve_returns_a_result_per_thread() {
    let fixture = Fixture::new("", false);
    fixture.prime_cache();
    let request = ThreadsResolveRequest {
        repo: REPO.to_string(),
        number: PR_NUMBER,
        thread_ids: vec![THREAD_ONE.to_string(), THREAD_TWO.to_string()],
        unresolve: false,
    };

    let body = succeed(handle_threads_resolve(&fixture.context(), request));

    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["id"], THREAD_ONE);
    assert_eq!(results[1]["id"], THREAD_TWO);
    assert!(results.iter().all(|result| result["ok"] == true));
    assert_eq!(body["unresolve"], false);
    assert_eq!(fixture.events_of(EVENT_THREADS_RESOLVED).len(), 1);
    assert_eq!(fixture.cache.cached_count(), 0);
}

/// A reply reports the thread it landed on and publishes the event.
#[test]
fn thread_reply_reports_the_thread() {
    let fixture = Fixture::new("", false);
    let request = ThreadReplyRequest {
        repo: REPO.to_string(),
        number: PR_NUMBER,
        thread_id: THREAD_ONE.to_string(),
        body: "done".to_string(),
    };

    let body = succeed(handle_thread_reply(&fixture.context(), request));

    assert_eq!(body["threadId"], THREAD_ONE);
    assert_eq!(body["replied"], true);
    assert_eq!(fixture.events_of(EVENT_THREAD_REPLIED).len(), 1);
}

fn boot_read_only_server() -> (crate::http::ServeHandle, u16, TempWorkspace) {
    let workspace = TempWorkspace::new(STATE_DIR_PREFIX);
    fs::write(workspace.path().join(DEFAULTS_SEEDED_FILE), "").unwrap();
    let listener = TcpListener::bind((LOOPBACK_HOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir: workspace.path().to_path_buf(),
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: true,
        allowed_origins: vec![],
    };
    (serve_on(listener, config).unwrap(), port, workspace)
}

fn post_status(port: u16, route: &str) -> u16 {
    let agent = ureq::AgentBuilder::new().build();
    let result = agent
        .post(&format!("http://{LOOPBACK_HOST}:{port}{route}"))
        .set("Content-Type", "application/json")
        .send_string("{}");
    match result {
        Ok(response) => response.status(),
        Err(ureq::Error::Status(code, _)) => code,
        Err(_) => 0,
    }
}

/// With the daemon read-only, every action route answers 403.
#[test]
fn it_every_action_route_is_forbidden_when_read_only() {
    let (handle, port, _workspace) = boot_read_only_server();

    for route in ACTION_ROUTES {
        assert_eq!(post_status(port, route), STATUS_FORBIDDEN, "route {route}");
    }
    handle.stop();
}
