//! Reading the open pull requests of the squad repositories through `gh`.

use crate::models::constants::squad::{
    ALIAS_PREFIX, DETAIL_BATCH_SIZE, GH_API, GH_FIELD_FLAG, GH_GRAPHQL, GH_JQ_FLAG,
    GH_TYPED_FIELD_FLAG, GH_USER_LOGIN_JQ, GH_USER_PATH, KEY_BASE_REF, KEY_BODY, KEY_CREATED_AT,
    KEY_HEAD_OID, KEY_HEAD_REF, KEY_ID, KEY_IS_DRAFT, KEY_IS_RESOLVED, KEY_MERGEABLE, KEY_NAME,
    KEY_NUMBER, KEY_REVIEW_DECISION, KEY_STATE, KEY_SUBMITTED_AT, KEY_TITLE, KEY_UPDATED_AT,
    KEY_URL, LIST_BATCH_SIZE, LIST_FIELDS, MESSAGE_CALL_PANICKED, MESSAGE_INVALID_REPO,
    MESSAGE_KEPT_PREVIOUS, MESSAGE_NO_DATA, MESSAGE_NO_LOGIN, MESSAGE_REPO_MISSING, PARALLEL_CALLS,
    PRS_PER_REPO, PR_FIELDS, PTR_AUTHOR_LOGIN, PTR_COMMENTS_TOTAL, PTR_COMMITS_TOTAL, PTR_DATA,
    PTR_LABEL_NODES, PTR_PR_NODES, PTR_PULL_REQUEST, PTR_RECENT_COMMENT_NODES,
    PTR_RECENT_THREAD_NODES, PTR_REQUESTED_LOGIN, PTR_REQUESTED_SLUG, PTR_REQUEST_NODES,
    PTR_REVIEW_COMMIT, PTR_REVIEW_NODES, PTR_ROLLUP_STATE, PTR_THREAD_COMMENT_NODES,
    PTR_THREAD_NODES, QUERY_ARG_PREFIX, SLUG_EXTRA_CHARS, SLUG_SEPARATOR, TRANSIENT_HTTP_STATUSES,
    VARIABLE_ME, VARIABLE_NAME_PREFIX, VARIABLE_NUMBER_PREFIX, VARIABLE_OWNER_PREFIX,
};
use crate::models::enums::squad_degraded_kind::SquadDegradedKind;
use crate::models::squad_comment::SquadComment;
use crate::models::squad_degraded::SquadDegraded;
use crate::models::squad_listed_pr::SquadListedPr;
use crate::models::squad_pr::SquadPr;
use crate::models::squad_prs::SquadPrs;
use crate::models::squad_review::SquadReview;
use crate::pr::{Mergeable, ReviewState, StatusContextState};
use crate::pr_comments::SUMMARY_MARKER;
use aidp_core::clock::Clock;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;

/// Runs `gh` with the given arguments and returns its stdout, or a failure message.
/// `Sync`: independent calls run on several threads at once.
pub trait GhRunner: Sync {
    fn run(&self, args: &[String]) -> Result<String, String>;
}

/// The real runner: spawns `gh` through the shared helper.
pub struct OsGhRunner;

impl GhRunner for OsGhRunner {
    fn run(&self, args: &[String]) -> Result<String, String> {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::gh::gh_stdout(GH_API, &borrowed).map_err(|error| error.to_string())
    }
}

/// Login of the authenticated GitHub user.
pub fn me_login(runner: &dyn GhRunner) -> Result<String, String> {
    let args = [GH_API, GH_USER_PATH, GH_JQ_FLAG, GH_USER_LOGIN_JQ].map(str::to_string);
    let login = runner.run(&args)?.trim().to_string();
    if login.is_empty() {
        Err(MESSAGE_NO_LOGIN.to_string())
    } else {
        Ok(login)
    }
}

/// Splits `owner/name`, accepting only characters a GitHub slug can hold.
fn split_repo(slug: &str) -> Option<(&str, &str)> {
    let (owner, name) = slug.split_once(SLUG_SEPARATOR)?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || SLUG_EXTRA_CHARS.contains(&c))
    };
    (valid(owner) && valid(name)).then_some((owner, name))
}

fn alias_of(index: usize) -> String {
    format!("{ALIAS_PREFIX}{index}")
}

fn repo_variables(index: usize) -> String {
    format!("${VARIABLE_OWNER_PREFIX}{index}:String!,${VARIABLE_NAME_PREFIX}{index}:String!,")
}

fn repository(index: usize) -> String {
    format!(
        "{alias}: repository(owner:${VARIABLE_OWNER_PREFIX}{index},name:${VARIABLE_NAME_PREFIX}{index})",
        alias = alias_of(index)
    )
}

/// Listing document: the open pull requests of `count` aliased repositories, scalars only.
fn list_query(count: usize) -> String {
    let variables: String = (0..count).map(repo_variables).collect();
    let selections: String = (0..count)
        .map(|i| {
            format!(
                "{repo}{{ pullRequests(states:OPEN,first:{PRS_PER_REPO}){{ nodes{{ {LIST_FIELDS} }} }} }} ",
                repo = repository(i)
            )
        })
        .collect();
    format!("query({variables}){{ {selections} }}")
}

/// Detail document: `count` aliased pull requests read in full.
fn detail_query(count: usize) -> String {
    let variables: String = (0..count)
        .map(|i| format!("{}${VARIABLE_NUMBER_PREFIX}{i}:Int!,", repo_variables(i)))
        .collect();
    let selections: String = (0..count)
        .map(|i| {
            format!(
                "{repo}{{ pullRequest(number:${VARIABLE_NUMBER_PREFIX}{i}){{ {PR_FIELDS} }} }} ",
                repo = repository(i)
            )
        })
        .collect();
    format!("query(${VARIABLE_ME}:String!,{variables}){{ {selections} }}")
}

fn push_field(args: &mut Vec<String>, flag: &str, value: String) {
    args.push(flag.to_string());
    args.push(value);
}

fn graphql_args(query: String) -> Vec<String> {
    let mut args: Vec<String> = vec![GH_API.into(), GH_GRAPHQL.into()];
    push_field(
        &mut args,
        GH_FIELD_FLAG,
        format!("{QUERY_ARG_PREFIX}{query}"),
    );
    args
}

fn push_repo(args: &mut Vec<String>, index: usize, owner: &str, name: &str) {
    push_field(
        args,
        GH_FIELD_FLAG,
        format!("{VARIABLE_OWNER_PREFIX}{index}={owner}"),
    );
    push_field(
        args,
        GH_FIELD_FLAG,
        format!("{VARIABLE_NAME_PREFIX}{index}={name}"),
    );
}

fn list_args(batch: &[(&str, &str)]) -> Vec<String> {
    let mut args = graphql_args(list_query(batch.len()));
    for (index, (owner, name)) in batch.iter().enumerate() {
        push_repo(&mut args, index, owner, name);
    }
    args
}

/// Detail arguments; the numbers go through `-F` so GraphQL receives them as `Int`.
fn detail_args(targets: &[(&str, &str, u64)], me: &str) -> Vec<String> {
    let mut args = graphql_args(detail_query(targets.len()));
    push_field(&mut args, GH_FIELD_FLAG, format!("{VARIABLE_ME}={me}"));
    for (index, (owner, name, number)) in targets.iter().enumerate() {
        push_repo(&mut args, index, owner, name);
        push_field(
            &mut args,
            GH_TYPED_FIELD_FLAG,
            format!("{VARIABLE_NUMBER_PREFIX}{index}={number}"),
        );
    }
    args
}

fn is_transient(message: &str) -> bool {
    TRANSIENT_HTTP_STATUSES
        .iter()
        .any(|status| message.contains(status))
}

/// The `data` object of a GraphQL call.
fn graphql_data(runner: &dyn GhRunner, args: &[String]) -> Result<Value, String> {
    let text = runner.run(args)?;
    let response: Value = serde_json::from_str(&text).map_err(|_| MESSAGE_NO_DATA.to_string())?;
    response
        .pointer(PTR_DATA)
        .filter(|data| data.is_object())
        .cloned()
        .ok_or_else(|| MESSAGE_NO_DATA.to_string())
}

/// The `data` of every call, [`PARALLEL_CALLS`] at a time, in call order.
fn run_all(runner: &dyn GhRunner, calls: &[Vec<String>]) -> Vec<Result<Value, String>> {
    let mut results = Vec::with_capacity(calls.len());
    for wave in calls.chunks(PARALLEL_CALLS) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = wave
                .iter()
                .map(|args| scope.spawn(move || graphql_data(runner, args)))
                .collect();
            results.extend(handles.into_iter().map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(MESSAGE_CALL_PANICKED.to_string()))
            }));
        });
    }
    results
}

/// Reads every batch. GitHub times out on a query it finds too heavy, so a batch it
/// answers 502, 503 or 504 is split in halves, and a single item is retried once.
fn read_batches<'a, T>(
    batches: Vec<&'a [T]>,
    runner: &dyn GhRunner,
    args_of: impl Fn(&[T]) -> Vec<String>,
) -> Vec<(&'a [T], Result<Value, String>)> {
    let mut read = Vec::new();
    let mut pending: Vec<(&'a [T], bool)> =
        batches.into_iter().map(|batch| (batch, false)).collect();
    while !pending.is_empty() {
        let calls: Vec<Vec<String>> = pending.iter().map(|(batch, _)| args_of(batch)).collect();
        let mut next = Vec::new();
        for ((batch, retried), answer) in pending.into_iter().zip(run_all(runner, &calls)) {
            match answer {
                Err(message) if is_transient(&message) && batch.len() > 1 => {
                    let (left, right) = batch.split_at(batch.len() / 2);
                    next.push((left, false));
                    next.push((right, false));
                }
                Err(message) if is_transient(&message) && !retried => next.push((batch, true)),
                answer => read.push((batch, answer)),
            }
        }
        pending = next;
    }
    read
}

fn pairs_of<'a>(slugs: &[&'a str]) -> Vec<(&'a str, &'a str)> {
    slugs.iter().filter_map(|slug| split_repo(slug)).collect()
}

fn triples_of<'a>(targets: &[&'a SquadListedPr]) -> Vec<(&'a str, &'a str, u64)> {
    targets
        .iter()
        .filter_map(|target| {
            split_repo(&target.repo).map(|(owner, name)| (owner, name, target.number))
        })
        .collect()
}

fn text_at(node: &Value, key: &str) -> String {
    node.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn opt_text_at(node: &Value, key: &str) -> Option<String> {
    node.get(key).and_then(Value::as_str).map(str::to_string)
}

fn nodes_at<'a>(node: &'a Value, pointer: &str) -> &'a [Value] {
    node.pointer(pointer)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn count_at(node: &Value, pointer: &str) -> u64 {
    node.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

/// Enum read from a GitHub value spelled as the enum itself.
fn enum_at<T: DeserializeOwned>(node: &Value, key: &str) -> Option<T> {
    node.get(key)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Enum read from a GitHub upper-case value, for enums serialized in snake case.
fn lowercase_enum_at<T: DeserializeOwned>(node: &Value, key: &str) -> Option<T> {
    let lowered = node.get(key)?.as_str()?.to_lowercase();
    serde_json::from_value(Value::String(lowered)).ok()
}

fn parse_review(node: &Value) -> SquadReview {
    SquadReview {
        author: node
            .pointer(PTR_AUTHOR_LOGIN)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        state: enum_at(node, KEY_STATE).unwrap_or(ReviewState::Unknown),
        commit_oid: node
            .pointer(PTR_REVIEW_COMMIT)
            .and_then(Value::as_str)
            .map(str::to_string),
        submitted_at: opt_text_at(node, KEY_SUBMITTED_AT),
    }
}

fn parse_comment(node: &Value) -> SquadComment {
    SquadComment {
        id: text_at(node, KEY_ID),
        author: node
            .pointer(PTR_AUTHOR_LOGIN)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created_at: text_at(node, KEY_CREATED_AT),
        body_marker: text_at(node, KEY_BODY).contains(SUMMARY_MARKER),
    }
}

/// Issue comments and review-thread comments of a pull request, oldest first.
fn parse_recent_comments(node: &Value) -> Vec<SquadComment> {
    let thread_comments = nodes_at(node, PTR_RECENT_THREAD_NODES)
        .iter()
        .flat_map(|thread| nodes_at(thread, PTR_THREAD_COMMENT_NODES));
    let mut comments: Vec<SquadComment> = nodes_at(node, PTR_RECENT_COMMENT_NODES)
        .iter()
        .chain(thread_comments)
        .map(parse_comment)
        .collect();
    comments.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    comments
}

fn parse_requested(node: &Value) -> Option<String> {
    [PTR_REQUESTED_LOGIN, PTR_REQUESTED_SLUG]
        .iter()
        .find_map(|pointer| node.pointer(pointer).and_then(Value::as_str))
        .map(str::to_string)
}

fn is_draft(node: &Value) -> bool {
    node.get(KEY_IS_DRAFT)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn rollup_state(node: &Value) -> Option<StatusContextState> {
    node.pointer(PTR_ROLLUP_STATE)
        .and_then(|value| serde_json::from_value::<StatusContextState>(value.clone()).ok())
}

fn parse_listed(repo: &str, node: &Value) -> SquadListedPr {
    SquadListedPr {
        repo: repo.to_string(),
        number: node.get(KEY_NUMBER).and_then(Value::as_u64).unwrap_or(0),
        updated_at: text_at(node, KEY_UPDATED_AT),
        head_sha: text_at(node, KEY_HEAD_OID),
        is_draft: is_draft(node),
        mergeable: lowercase_enum_at(node, KEY_MERGEABLE).unwrap_or(Mergeable::Unknown),
        review_decision: lowercase_enum_at(node, KEY_REVIEW_DECISION),
        checks_state: rollup_state(node),
    }
}

/// The cached pull request with the states the listing reads afresh.
fn refreshed(cached: &SquadPr, listed: &SquadListedPr) -> SquadPr {
    SquadPr {
        is_draft: listed.is_draft,
        mergeable: listed.mergeable,
        review_decision: listed.review_decision,
        checks_state: listed.checks_state,
        ..cached.clone()
    }
}

fn parse_pr(repo: &str, node: &Value) -> SquadPr {
    SquadPr {
        repo: repo.to_string(),
        number: node.get(KEY_NUMBER).and_then(Value::as_u64).unwrap_or(0),
        title: text_at(node, KEY_TITLE),
        url: text_at(node, KEY_URL),
        is_draft: is_draft(node),
        author: node
            .pointer(PTR_AUTHOR_LOGIN)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created_at: text_at(node, KEY_CREATED_AT),
        updated_at: text_at(node, KEY_UPDATED_AT),
        base_ref: text_at(node, KEY_BASE_REF),
        head_ref: text_at(node, KEY_HEAD_REF),
        head_sha: text_at(node, KEY_HEAD_OID),
        mergeable: lowercase_enum_at(node, KEY_MERGEABLE).unwrap_or(Mergeable::Unknown),
        review_decision: lowercase_enum_at(node, KEY_REVIEW_DECISION),
        review_requests: nodes_at(node, PTR_REQUEST_NODES)
            .iter()
            .filter_map(parse_requested)
            .collect(),
        my_reviews: nodes_at(node, PTR_REVIEW_NODES)
            .iter()
            .map(parse_review)
            .collect(),
        commits: count_at(node, PTR_COMMITS_TOTAL),
        checks_state: rollup_state(node),
        unresolved_threads: nodes_at(node, PTR_THREAD_NODES)
            .iter()
            .filter(|thread| {
                !thread
                    .get(KEY_IS_RESOLVED)
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .count() as u64,
        comments: count_at(node, PTR_COMMENTS_TOTAL),
        labels: nodes_at(node, PTR_LABEL_NODES)
            .iter()
            .filter_map(|label| label.get(KEY_NAME).and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
        recent_comments: parse_recent_comments(node),
    }
}

/// What one fetch gathers: the pull requests to serve and what could not be read.
struct Gathered<'a> {
    previous: Option<&'a SquadPrs>,
    prs: Vec<SquadPr>,
    degraded: Vec<SquadDegraded>,
}

impl Gathered<'_> {
    /// Reports a failed read and serves the cached pull requests it covered, if any.
    fn keep_previous(
        &mut self,
        kind: SquadDegradedKind,
        repos: Vec<String>,
        message: String,
        covered: impl Fn(&SquadPr) -> bool,
    ) {
        let kept: Vec<SquadPr> = self
            .previous
            .map(|previous| {
                previous
                    .prs
                    .iter()
                    .filter(|pr| covered(pr))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let message = if kept.is_empty() {
            message
        } else {
            format!("{message} ({MESSAGE_KEPT_PREVIOUS})")
        };
        self.prs.extend(kept);
        self.degraded.push(SquadDegraded {
            kind,
            repos,
            message,
        });
    }

    /// Takes the listing of one batch of repositories; a failure keeps their cached pull requests.
    fn list_batch(
        &mut self,
        slugs: &[&str],
        read: Result<Value, String>,
        listed: &mut Vec<SquadListedPr>,
    ) {
        let data = match read {
            Ok(data) => data,
            Err(message) => {
                let repos: Vec<String> = slugs.iter().map(|slug| slug.to_string()).collect();
                return self.keep_previous(SquadDegradedKind::Batch, repos, message, |pr| {
                    slugs.contains(&pr.repo.as_str())
                });
            }
        };
        for (index, slug) in slugs.iter().enumerate() {
            match data.get(alias_of(index)).filter(|repo| !repo.is_null()) {
                Some(repo) => listed.extend(
                    nodes_at(repo, PTR_PR_NODES)
                        .iter()
                        .map(|node| parse_listed(slug, node)),
                ),
                None => self.keep_previous(
                    SquadDegradedKind::Repo,
                    vec![slug.to_string()],
                    MESSAGE_REPO_MISSING.to_string(),
                    |pr| pr.repo == *slug,
                ),
            }
        }
    }

    /// Takes the full read of one batch of pull requests; a failure keeps their cached version.
    fn detail_batch(&mut self, targets: &[&SquadListedPr], read: Result<Value, String>) {
        let data = match read {
            Ok(data) => data,
            Err(message) => {
                let mut repos: Vec<String> =
                    targets.iter().map(|target| target.repo.clone()).collect();
                repos.dedup();
                return self.keep_previous(SquadDegradedKind::Batch, repos, message, |pr| {
                    targets
                        .iter()
                        .any(|target| target.repo == pr.repo && target.number == pr.number)
                });
            }
        };
        for (index, target) in targets.iter().enumerate() {
            let node = data
                .get(alias_of(index))
                .and_then(|repo| repo.pointer(PTR_PULL_REQUEST))
                .filter(|node| !node.is_null());
            if let Some(node) = node {
                self.prs.push(parse_pr(&target.repo, node));
            }
        }
    }
}

/// Open pull requests of `repos`: a light listing of every repository, then a full read of
/// the pull requests that changed since `previous` only. A read that fails keeps the cached
/// pull requests it covered and is reported in `degraded`.
pub fn squad_prs(
    repos: &[String],
    me: &str,
    runner: &dyn GhRunner,
    clock: &dyn Clock,
    previous: Option<&SquadPrs>,
) -> SquadPrs {
    let mut gathered = Gathered {
        previous,
        prs: Vec::new(),
        degraded: Vec::new(),
    };
    let (valid, invalid): (Vec<&str>, Vec<&str>) = repos
        .iter()
        .map(String::as_str)
        .partition(|slug| split_repo(slug).is_some());
    gathered
        .degraded
        .extend(invalid.into_iter().map(|slug| SquadDegraded {
            kind: SquadDegradedKind::Repo,
            repos: vec![slug.to_string()],
            message: MESSAGE_INVALID_REPO.to_string(),
        }));
    let mut listed = Vec::new();
    let list_batches: Vec<&[&str]> = valid.chunks(LIST_BATCH_SIZE).collect();
    for (batch, read) in read_batches(list_batches, runner, |batch| list_args(&pairs_of(batch))) {
        gathered.list_batch(batch, read, &mut listed);
    }
    let cached: BTreeMap<(&str, u64), &SquadPr> = previous
        .map(|previous| {
            previous
                .prs
                .iter()
                .map(|pr| ((pr.repo.as_str(), pr.number), pr))
                .collect()
        })
        .unwrap_or_default();
    let mut changed: Vec<&SquadListedPr> = Vec::new();
    for item in &listed {
        match cached.get(&(item.repo.as_str(), item.number)) {
            Some(pr) if pr.updated_at == item.updated_at && pr.head_sha == item.head_sha => {
                gathered.prs.push(refreshed(pr, item));
            }
            _ => changed.push(item),
        }
    }
    let detail_batches: Vec<&[&SquadListedPr]> = changed.chunks(DETAIL_BATCH_SIZE).collect();
    for (batch, read) in read_batches(detail_batches, runner, |batch| {
        detail_args(&triples_of(batch), me)
    }) {
        gathered.detail_batch(batch, read);
    }
    let Gathered {
        mut prs, degraded, ..
    } = gathered;
    prs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    SquadPrs {
        fetched_at: clock.now_rfc3339(),
        me: me.to_string(),
        repos: valid.iter().map(|slug| slug.to_string()).collect(),
        prs,
        degraded,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/fetch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/squad_fetch_comments_tests.rs"]
mod squad_fetch_comments_tests;
