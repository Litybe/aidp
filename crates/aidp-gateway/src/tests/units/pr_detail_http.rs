//! Pull-request page routes served over HTTP with `gh` replaced by a recording shim.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_gateway::models::enums::pr_refusal_reason::PrRefusalReason;
use aidp_gateway::models::file_blob::FileBlob;
use aidp_gateway::models::pr_comment_request::PrCommentRequest;
use aidp_gateway::models::pr_drafts_request::PrDraftsRequest;
use aidp_gateway::models::pr_review_comment_request::PrReviewCommentRequest;
use aidp_gateway::models::pr_review_request::PrReviewRequest;
use aidp_gateway::models::pr_viewed_request::PrViewedRequest;
use aidp_gateway::pr_viewed::PrViewed;
use aidp_integrations::models::enums::diff_side::DiffSide;
use aidp_integrations::models::enums::review_event::ReviewEvent;
use aidp_integrations::models::review_draft::ReviewDraft;
use aidp_testkit::{EnvGuard, ProcShim, TempWorkspace};
use serde::Serialize;
use serde_json::Value;
use std::net::TcpListener;
use std::path::PathBuf;

const HOOK_TOKEN: &str = "hook-secret";
const LOOPBACK_HOST: &str = "127.0.0.1";
const GH: &str = "gh";
const REPO: &str = "acme/widgets";
const PR_NUMBER: u64 = 42;
const COMMENT_ID: u64 = 777;
const STATUS_OK: u16 = 200;
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_CONFLICT: u16 = 409;
const NOT_NEEDED_BODY: &str = "nit";
const REWRITTEN_BODY: &str = "rewritten";
const PLEASE_FIX_BODY: &str = "please fix";
const HI_BODY: &str = "hi";
const LOOKS_GOOD_BODY: &str = "looks good";
const SECOND_FILE: &str = "src/b.rs";
const FILES_AFTER_PUSH: &str = include_str!("fixtures/pr_detail/files_after_push.json");
const PR_STATUS_EVENT: &str = "pr.status";
const COMMENT_CREATED_EVENT: &str = "pr.comment_created";
const DEFAULTS_SEEDED_FILE: &str = ".defaults-seeded";
const SONAR_ENV_KEYS: [&str; 3] = ["SONAR_HOST_URL", "SONAR_TOKEN", "SONAR_PROJECT_KEY"];
const COLLEAGUE_PR: &str = include_str!("fixtures/pr_detail/detail.json");
const FILES_PAYLOAD: &str = include_str!("fixtures/pr_detail/files.json");
const COMMENT_PAYLOAD: &str = include_str!("fixtures/pr_detail/review_submitted.json");
const COLLEAGUE_AUTHOR: &str = r#""author":{"login":"dev"}"#;
const VIEWER_AUTHOR: &str = r#""author":{"login":"me"}"#;
const WRITE_METHODS: [&str; 3] = ["POST", "PATCH", "DELETE"];
const FIRST_FILE: &str = "src/a.rs";
const ORIGINAL_BLOB: &str = "blob-a-1";
const CHANGED_BLOB: &str = "blob-a-2";

/// Base URL of the served gateway.
fn base(port: u16) -> String {
    format!("http://{LOOPBACK_HOST}:{port}/api")
}

/// Serves the gateway on a free port with its own state dir.
fn boot_server(read_only: bool) -> (ServeHandle, u16, PathBuf) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-pr-detail-state")));
    let state_dir: PathBuf = workspace.path().to_path_buf();
    std::fs::write(state_dir.join(DEFAULTS_SEEDED_FILE), "").unwrap();
    let listener = TcpListener::bind((LOOPBACK_HOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: PathBuf::new(),
        hook_token: HOOK_TOKEN.to_string(),
        ui_token: "ui-secret".to_string(),
        read_only,
        allowed_origins: vec![],
    };
    (serve_on(listener, config).unwrap(), port, state_dir)
}

/// Sends a request with an optional JSON body; error statuses are returned, not raised.
fn send(method: &str, url: &str, body: Option<Value>) -> (u16, Value) {
    let request = ureq::request(method, url);
    let outcome = match body {
        Some(payload) => request.send_json(payload),
        None => request.call(),
    };
    let response = outcome.unwrap_or_else(|error| error.into_response().unwrap());
    (
        response.status(),
        response.into_json().unwrap_or(Value::Null),
    )
}

/// A `gh` shim that answers every call with `stdout`.
fn gh_shim(stdout: &str) -> (ProcShim, TempWorkspace) {
    let workspace = TempWorkspace::new("gw-pr-detail-shim");
    let shim = ProcShim::new(GH, &workspace);
    shim.set_stdout(stdout);
    (shim, workspace)
}

/// True when some recorded `gh` call is a mutating REST call.
fn wrote_to_github(shim: &ProcShim) -> bool {
    shim.calls()
        .iter()
        .flatten()
        .any(|arg| WRITE_METHODS.contains(&arg.as_str()))
}

/// Names of the bus events published so far.
fn event_types(port: u16) -> Vec<String> {
    let (_, events) = send("GET", &format!("{}/events/recent", base(port)), None);
    events
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|event| event["type"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Serializes a typed request body for `send`.
fn payload(body: &impl Serialize) -> Option<Value> {
    Some(serde_json::to_value(body).unwrap())
}

fn draft(id: &str, line: u32) -> ReviewDraft {
    ReviewDraft {
        id: id.to_string(),
        path: FIRST_FILE.to_string(),
        line,
        side: DiffSide::Right,
        start_line: None,
        start_side: None,
        body: NOT_NEEDED_BODY.to_string(),
    }
}

fn two_drafts() -> PrDraftsRequest {
    PrDraftsRequest {
        repo: REPO.to_string(),
        number: PR_NUMBER,
        drafts: vec![draft("d1", 2), draft("d2", 3)],
    }
}

fn review(event: ReviewEvent) -> PrReviewRequest {
    PrReviewRequest {
        repo: REPO.to_string(),
        number: PR_NUMBER,
        event,
        body: PLEASE_FIX_BODY.to_string(),
    }
}

fn comment(body: &str) -> PrCommentRequest {
    PrCommentRequest {
        repo: REPO.to_string(),
        number: PR_NUMBER,
        body: body.to_string(),
    }
}

fn viewed_request(viewed: bool) -> PrViewedRequest {
    PrViewedRequest {
        repo: REPO.to_string(),
        number: PR_NUMBER,
        path: FIRST_FILE.to_string(),
        viewed,
    }
}

fn comment_url(port: u16, segment: &str) -> String {
    format!(
        "{}/prs/{segment}/{COMMENT_ID}?repo={REPO}&number={PR_NUMBER}",
        base(port)
    )
}

/// Approving your own pull request is a 409 carrying the `own-pr` reason and writes nothing.
#[test]
fn it_approving_your_own_pr_is_a_conflict_with_no_github_write() {
    let (shim, _ws) = gh_shim(&COLLEAGUE_PR.replace(COLLEAGUE_AUTHOR, VIEWER_AUTHOR));
    let (handle, port, _) = boot_server(false);

    let (status, body) = send(
        "POST",
        &format!("{}/prs/review", base(port)),
        payload(&review(ReviewEvent::Approve)),
    );

    assert_eq!(status, STATUS_CONFLICT, "body: {body}");
    assert_eq!(
        body["reason"],
        serde_json::to_value(PrRefusalReason::OwnPr).unwrap()
    );
    assert!(!wrote_to_github(&shim), "calls: {:?}", shim.calls());
    handle.stop();
}

/// Editing or deleting a colleague's comment is a 403 and never reaches GitHub.
#[test]
fn it_changing_a_colleagues_comment_is_forbidden_with_no_github_write() {
    let (shim, _ws) = gh_shim(COLLEAGUE_PR);
    let (handle, port, _) = boot_server(false);
    let edit = comment(REWRITTEN_BODY);

    let (patched, _) = send("PATCH", &comment_url(port, "comments"), payload(&edit));
    let (deleted, _) = send("DELETE", &comment_url(port, "review-comments"), None);

    assert_eq!((patched, deleted), (STATUS_FORBIDDEN, STATUS_FORBIDDEN));
    assert!(!wrote_to_github(&shim), "calls: {:?}", shim.calls());
    handle.stop();
}

/// A submit that GitHub does not accept leaves the stored drafts untouched.
#[test]
fn it_failed_review_submit_keeps_the_drafts() {
    let (_shim, _ws) = gh_shim(COLLEAGUE_PR);
    let (handle, port, _) = boot_server(false);
    let drafts_url = format!("{}/prs/drafts", base(port));
    let (stored, _) = send("PUT", &drafts_url, payload(&two_drafts()));

    let (submitted, _) = send(
        "POST",
        &format!("{}/prs/review", base(port)),
        payload(&review(ReviewEvent::RequestChanges)),
    );
    let (_, kept) = send(
        "GET",
        &format!("{drafts_url}?repo={REPO}&number={PR_NUMBER}"),
        None,
    );

    assert_eq!(stored, STATUS_OK);
    assert_ne!(submitted, STATUS_OK);
    assert_eq!(kept.as_array().map(Vec::len), Some(2), "kept: {kept}");
    handle.stop();
}

/// A repository with no local checkout is refused instead of reading another repo's Sonar project.
#[test]
fn it_sonar_of_a_repository_without_a_checkout_is_refused() {
    let _guards: Vec<EnvGuard> = SONAR_ENV_KEYS
        .iter()
        .map(|key| EnvGuard::unset(key))
        .collect();
    let (handle, port, _) = boot_server(false);

    let (status, body) = send(
        "GET",
        &format!("{}/prs/sonar?repo={REPO}&number={PR_NUMBER}", base(port)),
        None,
    );

    assert_eq!(status, STATUS_NOT_FOUND, "body: {body}");
    handle.stop();
}

/// Every write verb of the page is refused in read-only mode, before any handler runs.
#[test]
fn it_read_only_mode_refuses_every_write_route() {
    let (shim, _ws) = gh_shim(COMMENT_PAYLOAD);
    let (handle, port, _) = boot_server(true);
    let api = base(port);
    let hi = payload(&comment(HI_BODY));
    let inline = PrReviewCommentRequest {
        repo: REPO.to_string(),
        number: PR_NUMBER,
        path: FIRST_FILE.to_string(),
        line: 2,
        side: DiffSide::Right,
        start_line: None,
        start_side: None,
        body: HI_BODY.to_string(),
    };
    let requests = [
        ("POST", format!("{api}/prs/comments"), hi.clone()),
        ("PATCH", comment_url(port, "comments"), hi),
        ("DELETE", comment_url(port, "comments"), None),
        (
            "POST",
            format!("{api}/prs/review-comments"),
            payload(&inline),
        ),
        ("PUT", format!("{api}/prs/drafts"), payload(&two_drafts())),
        (
            "POST",
            format!("{api}/prs/review"),
            payload(&review(ReviewEvent::Comment)),
        ),
        (
            "POST",
            format!("{api}/prs/viewed"),
            payload(&viewed_request(true)),
        ),
    ];

    for (method, url, body) in requests {
        let (status, _) = send(method, &url, body);
        assert_eq!(status, STATUS_FORBIDDEN, "{method} {url}");
    }

    assert!(shim.calls().is_empty(), "calls: {:?}", shim.calls());
    handle.stop();
}

/// A write refreshes the PR status cache and publishes its bus event.
#[test]
fn it_comment_create_invalidates_the_status_cache_and_publishes_its_event() {
    let (shim, _ws) = gh_shim(COMMENT_PAYLOAD);
    let (handle, port, _) = boot_server(false);
    let created = comment(LOOKS_GOOD_BODY);

    let (status, body) = send(
        "POST",
        &format!("{}/prs/comments", base(port)),
        payload(&created),
    );

    let events = event_types(port);
    assert_eq!(status, STATUS_OK, "body: {body} calls: {:?}", shim.calls());
    assert!(
        events.iter().any(|name| name == COMMENT_CREATED_EVENT),
        "events: {events:?}"
    );
    assert!(
        events.iter().any(|name| name == PR_STATUS_EVENT),
        "events: {events:?}"
    );
    handle.stop();
}

/// The comment routes of the page do not shadow the existing state route.
#[test]
fn it_comments_state_stays_routed_to_its_own_handler() {
    let (handle, port, _) = boot_server(false);
    let api = base(port);

    let (state, _) = send("GET", &format!("{api}/prs/comments/state"), None);

    assert_ne!(state, STATUS_NOT_FOUND);
    handle.stop();
}

/// A comment id that is not a number is rejected before any lookup.
#[test]
fn it_non_numeric_comment_id_is_a_bad_request() {
    let (handle, port, _) = boot_server(false);
    let url = format!(
        "{}/prs/comments/abc?repo={REPO}&number={PR_NUMBER}",
        base(port)
    );

    let (status, _) = send("DELETE", &url, None);

    assert_eq!(status, STATUS_BAD_REQUEST);
    handle.stop();
}

/// Marking a file viewed over HTTP succeeds; a later blob change makes it unviewed.
#[test]
fn it_viewed_file_turns_unviewed_when_its_blob_changes() {
    let (_shim, _ws) = gh_shim(FILES_PAYLOAD);
    let (handle, port, state_dir) = boot_server(false);
    let viewed = viewed_request(true);

    let (status, body) = send(
        "POST",
        &format!("{}/prs/viewed", base(port)),
        payload(&viewed),
    );

    let blob = |sha: &str| {
        vec![FileBlob {
            path: FIRST_FILE.to_string(),
            sha: sha.to_string(),
        }]
    };
    let store = PrViewed::new(&state_dir);
    let unchanged = store.viewed_state(REPO, PR_NUMBER, &blob(ORIGINAL_BLOB));
    let changed = store.viewed_state(REPO, PR_NUMBER, &blob(CHANGED_BLOB));
    assert_eq!(status, STATUS_OK, "body: {body}");
    assert!(unchanged[0].viewed);
    assert!(!changed[0].viewed);
    handle.stop();
}

/// A file marked viewed then changed by a new commit comes back unviewed and listed since the review.
#[test]
fn it_files_route_reports_viewed_state_and_changes_since_the_review() {
    let (shim, _ws) = gh_shim(FILES_PAYLOAD);
    let (handle, port, _) = boot_server(false);
    let api = base(port);
    let files_url = format!("{api}/prs/files?repo={REPO}&number={PR_NUMBER}");
    send(
        "POST",
        &format!("{api}/prs/viewed"),
        payload(&viewed_request(true)),
    );

    let (_, before) = send("GET", &files_url, None);
    shim.set_stdout(FILES_AFTER_PUSH);
    let (_, after) = send("GET", &files_url, None);

    let viewed_flags = |reply: &Value| -> Vec<(String, bool)> {
        reply["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| {
                (
                    file["filename"].as_str().unwrap().to_string(),
                    file["viewed"].as_bool().unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(
        viewed_flags(&before),
        vec![
            (FIRST_FILE.to_string(), true),
            (SECOND_FILE.to_string(), false)
        ]
    );
    assert!(before["sinceLastReview"].as_array().unwrap().is_empty());
    assert_eq!(
        viewed_flags(&after),
        vec![
            (FIRST_FILE.to_string(), false),
            (SECOND_FILE.to_string(), false)
        ]
    );
    assert_eq!(after["sinceLastReview"].as_array().unwrap().len(), 1);
    assert_eq!(after["sinceLastReview"][0], FIRST_FILE);
    handle.stop();
}
