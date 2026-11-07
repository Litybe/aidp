use super::*;
use crate::models::constants::squad::{LIST_BATCH_SIZE, MESSAGE_KEPT_PREVIOUS};
use crate::models::enums::squad_degraded_kind::SquadDegradedKind;
use crate::pr::StatusContextState;
use aidp_core::clock::FixedClock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::squad::test_support::*;

const SEVENTEEN_REPOS: usize = 17;
const ONE_BATCH_OF_REPOS: usize = LIST_BATCH_SIZE;
const FIRST_PR: u64 = 11;
const SECOND_PR: u64 = 12;
const FIRST_REPO: &str = "acme/r00";
const OLDER_UPDATE: &str = "2026-09-01T10:00:00Z";
const KEPT_LABEL: &str = "kept-from-cache";
const FIRST_NUMBER_ARG: &str = "k0=11";
const SECOND_NUMBER_ARG: &str = "k1=12";
const TYPED_FIELD_FLAG: &str = "-F";
const LAST_REPO: &str = "acme/r16";
const SECOND_PR_REPO: &str = "acme/r10";
/// Every repository name of the second listing batch (r10 to r16) holds this text.
const SECOND_BATCH_NAME_MARK: &str = "=r1";
/// Name argument only a listing of the whole first batch (r00 to r04) carries.
const LAST_OF_FIRST_BATCH_NAME: &str = "n4=r04";
const FIRST_AUTHOR: &str = "bob";
const FIRST_HEAD: &str = "sha11";
const FIRST_LABEL: &str = "bug";
const BAD_SLUG: &str = "not-a-slug";
const GOOD_SLUG: &str = "acme/ok";
const NOT_JSON: &str = "not json";
const LOGIN_OUTPUT: &str = "alice\n";
const BLANK_OUTPUT: &str = "  \n";
const BAD_GATEWAY_STATUS: &str = "502";
const UNRESOLVED_THREADS: u64 = 1;
const COMMENT_COUNT: u64 = 3;
const COMMIT_COUNT: u64 = 2;

fn fetch(repos: &[String], runner: &FakeRunner, previous: Option<&SquadPrs>) -> SquadPrs {
    squad_prs(repos, ME, runner, &FixedClock::from_rfc3339(NOW), previous)
}

fn first_fetch() -> SquadPrs {
    fetch(
        &repo_names(SEVENTEEN_REPOS),
        &FakeRunner::new(graphql_responder()),
        None,
    )
}

fn pr_numbered(data: &SquadPrs, number: u64) -> &SquadPr {
    data.prs.iter().find(|pr| pr.number == number).unwrap()
}

fn detail_calls(runner: &FakeRunner) -> Vec<Vec<String>> {
    runner
        .calls()
        .into_iter()
        .filter(|args| is_detail(args))
        .collect()
}

#[test]
fn a_first_fetch_lists_every_batch_then_reads_the_pull_requests_in_one_detail_query() {
    let runner = FakeRunner::new(graphql_responder());
    let result = fetch(&repo_names(SEVENTEEN_REPOS), &runner, None);
    let listings = SEVENTEEN_REPOS.div_ceil(LIST_BATCH_SIZE);
    assert_eq!(
        runner.call_count(),
        listings + 1,
        "every listing, then one detail query"
    );
    assert_eq!(detail_calls(&runner).len(), 1);
    assert_eq!(result.prs.len(), 2);
    assert!(result.degraded.is_empty());
    assert_eq!(result.repos.len(), SEVENTEEN_REPOS);
}

#[test]
fn the_detail_read_parses_the_pull_request_fields() {
    let result = first_fetch();
    let pr = pr_numbered(&result, FIRST_PR);
    assert_eq!(pr.repo, FIRST_REPO);
    assert_eq!(pr.author, FIRST_AUTHOR);
    assert_eq!(pr.head_sha, FIRST_HEAD);
    assert_eq!(pr.review_requests, vec![ME.to_string()]);
    assert_eq!(pr.unresolved_threads, UNRESOLVED_THREADS);
    assert_eq!(pr.comments, COMMENT_COUNT);
    assert_eq!(pr.commits, COMMIT_COUNT);
    assert_eq!(pr.checks_state, Some(StatusContextState::Success));
    assert_eq!(pr.labels, vec![FIRST_LABEL.to_string()]);
    assert_eq!(result.me, ME);
}

#[test]
fn the_detail_query_sends_the_numbers_as_integers() {
    let runner = FakeRunner::new(graphql_responder());
    fetch(&repo_names(SEVENTEEN_REPOS), &runner, None);
    let args = &detail_calls(&runner)[0];
    let typed: Vec<&String> = args
        .windows(2)
        .filter(|pair| pair[0] == TYPED_FIELD_FLAG)
        .map(|pair| &pair[1])
        .collect();
    assert_eq!(typed, vec![FIRST_NUMBER_ARG, SECOND_NUMBER_ARG]);
}

#[test]
fn an_unchanged_pull_request_is_not_read_again_but_its_checks_are() {
    let mut previous = first_fetch();
    for pr in &mut previous.prs {
        pr.checks_state = Some(StatusContextState::Failure);
        pr.labels = vec![KEPT_LABEL.to_string()];
    }
    let runner = FakeRunner::new(graphql_responder());
    let result = fetch(&repo_names(SEVENTEEN_REPOS), &runner, Some(&previous));
    assert!(detail_calls(&runner).is_empty(), "nothing changed");
    let pr = pr_numbered(&result, FIRST_PR);
    assert_eq!(pr.checks_state, Some(StatusContextState::Success));
    assert_eq!(pr.labels, vec![KEPT_LABEL.to_string()]);
}

#[test]
fn only_the_pull_request_updated_since_is_read_again() {
    let mut previous = first_fetch();
    previous
        .prs
        .iter_mut()
        .find(|pr| pr.number == FIRST_PR)
        .unwrap()
        .updated_at = OLDER_UPDATE.to_string();
    let runner = FakeRunner::new(graphql_responder());
    fetch(&repo_names(SEVENTEEN_REPOS), &runner, Some(&previous));
    let details = detail_calls(&runner);
    assert_eq!(details.len(), 1);
    assert!(details[0].iter().any(|arg| arg == FIRST_NUMBER_ARG));
    assert!(!details[0].iter().any(|arg| arg == SECOND_NUMBER_ARG));
}

#[test]
fn a_gateway_timeout_on_a_single_repository_is_retried_once() {
    let failed_once = AtomicBool::new(false);
    let listing = graphql_responder();
    let runner = FakeRunner::new(Box::new(move |args| {
        if !is_detail(args) && !failed_once.swap(true, Ordering::SeqCst) {
            return Err(BAD_GATEWAY.to_string());
        }
        listing(args)
    }));
    let result = fetch(&repo_names(1), &runner, None);
    assert!(result.degraded.is_empty());
    assert_eq!(result.prs.len(), 1);
    assert_eq!(
        runner.call_count(),
        3,
        "listing, its retry, then the detail"
    );
}

#[test]
fn a_batch_github_times_out_on_is_read_again_in_halves() {
    let listing = graphql_responder();
    let runner = FakeRunner::new(Box::new(move |args| {
        let whole_batch = args.iter().any(|arg| arg == LAST_OF_FIRST_BATCH_NAME);
        if !is_detail(args) && whole_batch {
            return Err(BAD_GATEWAY.to_string());
        }
        listing(args)
    }));
    let result = fetch(&repo_names(ONE_BATCH_OF_REPOS), &runner, None);
    assert!(result.degraded.is_empty());
    let listings = runner
        .calls()
        .into_iter()
        .filter(|args| !is_detail(args))
        .count();
    assert_eq!(listings, 3, "the whole batch, then its two halves");
}

#[test]
fn a_batch_that_keeps_failing_serves_its_last_known_pull_requests() {
    let previous = first_fetch();
    let listing = graphql_responder();
    let runner = FakeRunner::new(Box::new(move |args| {
        let second_batch = args.iter().any(|arg| arg.contains(SECOND_BATCH_NAME_MARK));
        if !is_detail(args) && second_batch {
            return Err(BAD_GATEWAY.to_string());
        }
        listing(args)
    }));
    let result = fetch(&repo_names(SEVENTEEN_REPOS), &runner, Some(&previous));
    assert!(result.prs.iter().any(|pr| pr.number == SECOND_PR));
    assert!(result
        .degraded
        .iter()
        .all(|entry| entry.kind == SquadDegradedKind::Batch));
    let kept = result
        .degraded
        .iter()
        .find(|entry| entry.repos == vec![SECOND_PR_REPO.to_string()]);
    assert!(kept.unwrap().message.contains(MESSAGE_KEPT_PREVIOUS));
    assert!(result
        .degraded
        .iter()
        .any(|entry| entry.repos.contains(&LAST_REPO.to_string())));
}

#[test]
fn a_failed_detail_read_keeps_the_cached_version() {
    let mut previous = first_fetch();
    previous
        .prs
        .iter_mut()
        .find(|pr| pr.number == FIRST_PR)
        .unwrap()
        .updated_at = OLDER_UPDATE.to_string();
    let listing = graphql_responder();
    let runner = FakeRunner::new(Box::new(move |args| {
        if is_detail(args) {
            return Err(FORBIDDEN.to_string());
        }
        listing(args)
    }));
    let result = fetch(&repo_names(SEVENTEEN_REPOS), &runner, Some(&previous));
    assert_eq!(pr_numbered(&result, FIRST_PR).updated_at, OLDER_UPDATE);
    assert_eq!(result.degraded[0].repos, vec![FIRST_REPO.to_string()]);
    assert_eq!(detail_calls(&runner).len(), 1, "a 403 is not retried");
}

#[test]
fn without_a_cache_a_failed_batch_hides_only_its_repositories() {
    let listing = graphql_responder();
    let runner = FakeRunner::new(Box::new(move |args| {
        let second_batch = args.iter().any(|arg| arg.contains(SECOND_BATCH_NAME_MARK));
        if !is_detail(args) && second_batch {
            return Err(BAD_GATEWAY.to_string());
        }
        listing(args)
    }));
    let result = fetch(&repo_names(SEVENTEEN_REPOS), &runner, None);
    assert_eq!(result.prs.len(), 1);
    assert!(result
        .degraded
        .iter()
        .all(|entry| entry.message.contains(BAD_GATEWAY_STATUS)));
    assert!(!result
        .degraded
        .iter()
        .any(|entry| entry.message.contains(MESSAGE_KEPT_PREVIOUS)));
}

#[test]
fn invalid_repo_slug_is_degraded_and_never_sent() {
    let repos = vec![BAD_SLUG.to_string(), GOOD_SLUG.to_string()];
    let runner = FakeRunner::new(Box::new(|_| Ok(LIST_ONE.to_string())));
    let result = fetch(&repos, &runner, None);
    assert_eq!(result.repos, vec![GOOD_SLUG.to_string()]);
    assert_eq!(result.degraded[0].kind, SquadDegradedKind::Repo);
    assert_eq!(result.degraded[0].repos, vec![BAD_SLUG.to_string()]);
}

#[test]
fn unparseable_listing_is_degraded() {
    let repos = repo_names(2);
    let runner = FakeRunner::new(Box::new(|_| Ok(NOT_JSON.to_string())));
    let result = fetch(&repos, &runner, None);
    assert!(result.prs.is_empty());
    assert_eq!(result.degraded[0].kind, SquadDegradedKind::Batch);
    assert_eq!(result.degraded[0].repos, repos);
}

#[test]
fn me_login_trims_and_rejects_empty_output() {
    let named = FakeRunner::new(Box::new(|_| Ok(LOGIN_OUTPUT.to_string())));
    assert_eq!(me_login(&named).unwrap(), ME);
    let empty = FakeRunner::new(Box::new(|_| Ok(BLANK_OUTPUT.to_string())));
    assert!(me_login(&empty).is_err());
}
