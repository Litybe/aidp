//! PR status and merge gate read through gh; pure verdicts plus one GraphQL, compare and rules read.

use crate::gh::{gh_stdout, in_dir, pr_resolve_checked, repo_slug};
use crate::IntegrationError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::path::Path;

/// Wire names read from GitHub payloads.
mod wire {
    pub const APPROVING_REVIEW_COUNT: &str = "required_approving_review_count";
    pub const AUTHOR: &str = "author";
    pub const BASE_REF_NAME: &str = "baseRefName";
    pub const BEHIND_BY: &str = "behind_by";
    pub const COMMIT: &str = "commit";
    pub const COMMITS: &str = "commits";
    pub const CONCLUSION: &str = "conclusion";
    pub const CONTEXT: &str = "context";
    pub const CONTEXTS: &str = "contexts";
    pub const DATA: &str = "data";
    pub const DETAILS_URL: &str = "detailsUrl";
    pub const HEAD_REF_NAME: &str = "headRefName";
    pub const HEAD_REF_OID: &str = "headRefOid";
    pub const IS_DRAFT: &str = "isDraft";
    pub const IS_RESOLVED: &str = "isResolved";
    pub const COMMENTS: &str = "comments";
    pub const REVIEWS: &str = "reviews";
    pub const ID: &str = "id";
    pub const PAGE_INFO: &str = "pageInfo";
    pub const HAS_NEXT_PAGE: &str = "hasNextPage";
    pub const TOTAL_COUNT: &str = "totalCount";
    pub const OID: &str = "oid";
    pub const LATEST_REVIEWS: &str = "latestOpinionatedReviews";
    pub const LOGIN: &str = "login";
    pub const MERGEABLE: &str = "mergeable";
    pub const NAME: &str = "name";
    pub const NODES: &str = "nodes";
    pub const NUMBER: &str = "number";
    pub const PARAMETERS: &str = "parameters";
    pub const PULL_REQUEST: &str = "pullRequest";
    pub const REPOSITORY: &str = "repository";
    pub const REQUESTED_REVIEWER: &str = "requestedReviewer";
    pub const REVIEW_DECISION: &str = "reviewDecision";
    pub const REVIEW_REQUESTS: &str = "reviewRequests";
    pub const REVIEW_THREADS: &str = "reviewThreads";
    pub const RULE_TYPE: &str = "type";
    pub const STARTED_AT: &str = "startedAt";
    pub const STATE: &str = "state";
    pub const STATUS: &str = "status";
    pub const STATUS_CHECK_ROLLUP: &str = "statusCheckRollup";
    pub const SUBMITTED_AT: &str = "submittedAt";
    pub const TARGET_URL: &str = "targetUrl";
    pub const THREAD_RULE: &str = "required_review_thread_resolution";
    pub const PULL_REQUEST_RULE: &str = "pull_request";
    pub const TITLE: &str = "title";
    pub const TYPENAME: &str = "__typename";
    pub const URL: &str = "url";
    pub const VIEWER: &str = "viewer";
    pub const STATUS_CONTEXT_TYPE: &str = "StatusContext";
}

/// Verdict over the checks of a PR head commit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChecksVerdict {
    None,
    Pending,
    Passed,
    Failed,
}

/// Review decision, as GitHub reports it or as derived from the latest reviews.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Where a review decision came from.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSource {
    Github,
    Derived,
}

/// Lifecycle state of a pull request; `Unknown` when it could not be read.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    #[serde(alias = "OPEN")]
    Open,
    #[serde(alias = "MERGED")]
    Merged,
    #[serde(alias = "CLOSED")]
    Closed,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Whether the PR merges cleanly into its base.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mergeable {
    Mergeable,
    Conflicting,
    #[default]
    #[serde(other)]
    Unknown,
}

/// A part of the status document that could not be read.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DegradedRead {
    Checks,
    Reviews,
    Threads,
    Rules,
}

/// Identifier of one merge gate condition.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateItemId {
    Open,
    NotDraft,
    NoChangesRequested,
    Approved,
    Checks,
    NoConflict,
    Threads,
}

/// Status of a check run as GitHub reports it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckRunStatus {
    Queued,
    InProgress,
    Completed,
    Waiting,
    Pending,
    Requested,
    #[serde(other)]
    Unknown,
}

/// Conclusion of a completed check run as GitHub reports it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckConclusion {
    Success,
    Neutral,
    Skipped,
    Failure,
    Cancelled,
    TimedOut,
    ActionRequired,
    Stale,
    StartupFailure,
    #[serde(other)]
    Unknown,
}

/// State of a commit status context as GitHub reports it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatusContextState {
    Success,
    Pending,
    Expected,
    Error,
    Failure,
    #[serde(other)]
    Unknown,
}

/// State of one review as GitHub reports it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
    #[serde(other)]
    Unknown,
}

/// Status of a check entry: a check run status or a status context state.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum CheckStatus {
    Run(CheckRunStatus),
    Context(StatusContextState),
}

/// Sentences the gate returns as `reason`.
pub mod reason {
    pub const MERGED: &str = "this PR is already merged";
    pub const CLOSED: &str = "this PR is closed";
    pub const STATE_UNREADABLE: &str = "the state of this PR could not be read";
    pub const DRAFT: &str = "this PR is still a draft";
    pub const CHANGES_REQUESTED: &str = "changes were requested on this PR";
    pub const NOT_APPROVED: &str = "this PR is not approved yet";
    pub const NO_APPROVAL_RULE: &str = "no approval rule on this repository";
    pub const APPROVALS_PREFIX: &str = "this PR has ";
    pub const APPROVALS_OF: &str = " of ";
    pub const APPROVALS_SUFFIX: &str = " required approvals";
    pub const REVIEWS_UNREADABLE: &str = "the reviews of this PR could not be read";
    pub const CHECKS_FAILED: &str = "checks are failing";
    pub const CHECKS_PENDING: &str = "checks are still running";
    pub const CHECKS_UNREADABLE: &str = "the checks of this PR could not be read";
    pub const CONFLICT: &str = "this PR has merge conflicts";
    pub const ONE_THREAD: &str = "1 review conversation is still unresolved";
    pub const MANY_THREADS_SUFFIX: &str = "review conversations are still unresolved";
}

mod gh_args {
    pub const API: &str = "api";
    pub const GRAPHQL: &str = "graphql";
    pub const STRING_FIELD: &str = "-f";
    pub const TYPED_FIELD: &str = "-F";
    pub const QUERY_FIELD: &str = "query=";
    pub const OWNER_FIELD: &str = "owner=";
    pub const NAME_FIELD: &str = "name=";
    pub const NUMBER_FIELD: &str = "number=";
    pub const GRAPHQL_LABEL: &str = "gh api graphql";
    pub const COMPARE_LABEL: &str = "gh api compare";
    pub const RULES_LABEL: &str = "gh api rules";
    pub const REPOS_SEGMENT: &str = "repos";
    pub const RULES_SEGMENT: &str = "rules/branches";
    pub const COMPARE_SEGMENT: &str = "compare";
    pub const COMPARE_RANGE_SEPARATOR: &str = "...";
    pub const NO_PR_MESSAGE: &str = "pull request";
    pub const NO_REPOSITORY_MESSAGE: &str = "repository";
    pub const MISSING_PR_PAYLOAD: &str = "GitHub returned no pull request payload";
}

const GH_PAGINATE: &str = "--paginate";
const GH_SLURP: &str = "--slurp";
const PR_STATUS_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!,$endCursor:String){viewer{login} repository(owner:$owner,name:$name){pullRequest(number:$number){number url title state isDraft headRefName headRefOid baseRefName mergeable reviewDecision author{login} latestOpinionatedReviews(first:30){nodes{author{login} state submittedAt commit{oid}}} reviews(last:100){nodes{author{login} state submittedAt commit{oid}}} reviewRequests(first:20){nodes{requestedReviewer{... on User{login} ... on Team{name}}}} commits(last:1){nodes{commit{statusCheckRollup{contexts(first:100){nodes{__typename ... on CheckRun{name status conclusion detailsUrl startedAt} ... on StatusContext{context state targetUrl}}}}}}} reviewThreads(first:100,after:$endCursor){pageInfo{hasNextPage endCursor} nodes{id isResolved isOutdated comments(last:1){totalCount nodes{id author{login} createdAt}}}}}}}";

/// A check run or status context, reduced to what the verdict needs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckEntry {
    pub name: String,
    pub status: CheckStatus,
    pub conclusion: Option<CheckConclusion>,
    pub url: Option<String>,
    #[serde(skip)]
    pub started_at: String,
}

/// Verdict over the deduplicated checks of a PR head commit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChecksReport {
    pub verdict: ChecksVerdict,
    pub runs: Vec<CheckEntry>,
    #[serde(skip)]
    pub degraded: bool,
}

/// How one check entry weighs on the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckOutcome {
    Pass,
    Waiting,
    Fail,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn nodes<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    value.get(key)?.get(wire::NODES)?.as_array()
}

fn wire_enum<T: DeserializeOwned>(value: &Value, key: &str) -> Option<T> {
    serde_json::from_value(value.get(key)?.clone()).ok()
}

fn lowercase_enum<T: DeserializeOwned>(value: &Value, key: &str) -> Option<T> {
    let lowered = text(value, key)?.to_lowercase();
    serde_json::from_value(Value::String(lowered)).ok()
}

fn context_outcome(state: StatusContextState) -> CheckOutcome {
    match state {
        StatusContextState::Success => CheckOutcome::Pass,
        StatusContextState::Pending | StatusContextState::Expected => CheckOutcome::Waiting,
        StatusContextState::Error | StatusContextState::Failure | StatusContextState::Unknown => {
            CheckOutcome::Fail
        }
    }
}

fn conclusion_outcome(conclusion: Option<CheckConclusion>) -> CheckOutcome {
    match conclusion {
        Some(CheckConclusion::Success | CheckConclusion::Neutral | CheckConclusion::Skipped) => {
            CheckOutcome::Pass
        }
        _ => CheckOutcome::Fail,
    }
}

fn run_outcome(status: CheckRunStatus, conclusion: Option<CheckConclusion>) -> CheckOutcome {
    match status {
        CheckRunStatus::Completed => conclusion_outcome(conclusion),
        CheckRunStatus::Queued
        | CheckRunStatus::InProgress
        | CheckRunStatus::Waiting
        | CheckRunStatus::Pending
        | CheckRunStatus::Requested
        | CheckRunStatus::Unknown => CheckOutcome::Waiting,
    }
}

fn parse_check_entry(node: &Value) -> Option<(CheckEntry, CheckOutcome)> {
    if text(node, wire::TYPENAME).as_deref() == Some(wire::STATUS_CONTEXT_TYPE) {
        let state: StatusContextState = wire_enum(node, wire::STATE)?;
        let entry = CheckEntry {
            name: text(node, wire::CONTEXT)?,
            status: CheckStatus::Context(state),
            conclusion: None,
            url: text(node, wire::TARGET_URL),
            started_at: String::new(),
        };
        return Some((entry, context_outcome(state)));
    }
    let status: CheckRunStatus = wire_enum(node, wire::STATUS)?;
    let conclusion: Option<CheckConclusion> = wire_enum(node, wire::CONCLUSION);
    let entry = CheckEntry {
        name: text(node, wire::NAME)?,
        status: CheckStatus::Run(status),
        conclusion,
        url: text(node, wire::DETAILS_URL),
        started_at: text(node, wire::STARTED_AT).unwrap_or_default(),
    };
    Some((entry, run_outcome(status, conclusion)))
}

/// Keeps the latest entry per name, ordered by `started_at`; ties keep the
/// later node.
fn dedupe_latest(entries: Vec<(CheckEntry, CheckOutcome)>) -> Vec<(CheckEntry, CheckOutcome)> {
    let mut kept: Vec<(CheckEntry, CheckOutcome)> = Vec::new();
    for candidate in entries {
        match kept
            .iter()
            .position(|(existing, _)| existing.name == candidate.0.name)
        {
            Some(index)
                if candidate.0.started_at.cmp(&kept[index].0.started_at) != Ordering::Less =>
            {
                kept[index] = candidate;
            }
            Some(_) => {}
            None => kept.push(candidate),
        }
    }
    kept
}

/// Verdict for a `statusCheckRollup` value. A null rollup has no checks; a
/// rollup without a `contexts` key is a degraded read.
pub fn checks_verdict(rollup: &Value) -> ChecksReport {
    if rollup.is_null() {
        return ChecksReport {
            verdict: ChecksVerdict::None,
            runs: Vec::new(),
            degraded: false,
        };
    }
    let Some(contexts) = nodes(rollup, wire::CONTEXTS) else {
        return ChecksReport {
            verdict: ChecksVerdict::None,
            runs: Vec::new(),
            degraded: true,
        };
    };
    let parsed = contexts.iter().filter_map(parse_check_entry).collect();
    let latest = dedupe_latest(parsed);
    let verdict = if latest.is_empty() {
        ChecksVerdict::None
    } else if latest
        .iter()
        .any(|(_, outcome)| *outcome == CheckOutcome::Waiting)
    {
        ChecksVerdict::Pending
    } else if latest
        .iter()
        .any(|(_, outcome)| *outcome == CheckOutcome::Fail)
    {
        ChecksVerdict::Failed
    } else {
        ChecksVerdict::Passed
    };
    ChecksReport {
        verdict,
        runs: latest.into_iter().map(|(entry, _)| entry).collect(),
        degraded: false,
    }
}

/// One reviewer's latest opinionated review.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Reviewer {
    pub login: String,
    pub state: ReviewState,
    pub at: Option<String>,
    #[serde(rename = "commitSha")]
    pub commit_sha: Option<String>,
}

/// Review decision, where it came from and who is involved.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReviewReport {
    pub decision: ReviewDecision,
    pub source: ReviewSource,
    pub reviewers: Vec<Reviewer>,
    pub submitted: Vec<Reviewer>,
    pub requested: Vec<String>,
    #[serde(skip)]
    pub degraded: bool,
}

fn parse_reviewers(pr: &Value) -> Option<Vec<Reviewer>> {
    let reviews = nodes(pr, wire::LATEST_REVIEWS)?;
    Some(
        reviews
            .iter()
            .filter_map(|review| {
                Some(Reviewer {
                    login: text(review.get(wire::AUTHOR)?, wire::LOGIN)?,
                    state: wire_enum(review, wire::STATE)?,
                    at: text(review, wire::SUBMITTED_AT),
                    commit_sha: review
                        .get(wire::COMMIT)
                        .and_then(|commit| text(commit, wire::OID)),
                })
            })
            .collect(),
    )
}

fn parse_requested(pr: &Value) -> Vec<String> {
    nodes(pr, wire::REVIEW_REQUESTS)
        .map(|requests| {
            requests
                .iter()
                .filter_map(|request| {
                    let reviewer = request.get(wire::REQUESTED_REVIEWER)?;
                    text(reviewer, wire::LOGIN).or_else(|| text(reviewer, wire::NAME))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// GitHub's own `reviewDecision`, when the node carries one.
fn github_decision(pr: &Value) -> Option<ReviewDecision> {
    text(pr, wire::REVIEW_DECISION)?;
    Some(lowercase_enum(pr, wire::REVIEW_DECISION).unwrap_or_default())
}

/// Review state of a pull request node. GitHub's own `reviewDecision` wins
/// when set; otherwise the decision is derived from the latest reviews.
pub fn review_state(pr: &Value) -> ReviewReport {
    let reviewers = parse_reviewers(pr);
    let requested = parse_requested(pr);
    let submitted = nodes(pr, wire::REVIEWS)
        .into_iter()
        .flatten()
        .filter_map(|review| {
            Some(Reviewer {
                login: text(review.get(wire::AUTHOR)?, wire::LOGIN)?,
                state: wire_enum(review, wire::STATE)?,
                at: text(review, wire::SUBMITTED_AT),
                commit_sha: review
                    .get(wire::COMMIT)
                    .and_then(|commit| text(commit, wire::OID)),
            })
        })
        .filter(|review| !matches!(review.state, ReviewState::Pending | ReviewState::Dismissed))
        .collect();
    if let Some(decision) = github_decision(pr) {
        return ReviewReport {
            decision,
            source: ReviewSource::Github,
            reviewers: reviewers.unwrap_or_default(),
            requested,
            submitted,
            degraded: false,
        };
    }
    let Some(reviewers) = reviewers else {
        return ReviewReport {
            decision: ReviewDecision::ReviewRequired,
            source: ReviewSource::Derived,
            reviewers: Vec::new(),
            requested,
            submitted,
            degraded: true,
        };
    };
    let decision = if reviewers
        .iter()
        .any(|reviewer| reviewer.state == ReviewState::ChangesRequested)
    {
        ReviewDecision::ChangesRequested
    } else if reviewers
        .iter()
        .any(|reviewer| reviewer.state == ReviewState::Approved)
    {
        ReviewDecision::Approved
    } else {
        ReviewDecision::ReviewRequired
    };
    ReviewReport {
        decision,
        source: ReviewSource::Derived,
        reviewers,
        requested,
        submitted,
        degraded: false,
    }
}

/// The facts the merge gate evaluates.
#[derive(Debug, Clone)]
pub struct GateFacts {
    pub state: PrState,
    pub is_draft: bool,
    pub review_decision: ReviewDecision,
    pub reviews_degraded: bool,
    pub checks_verdict: ChecksVerdict,
    pub checks_degraded: bool,
    pub mergeable: Mergeable,
    pub unresolved_threads: u64,
    pub require_thread_resolution: bool,
    pub review_decision_from_github: bool,
    pub required_approvals: Option<u64>,
    pub approving_reviewers: u64,
}

/// One gate condition and its result.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GateItem {
    pub id: GateItemId,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The merge verdict: `reason` is the first failing item's sentence.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Gate {
    pub ok: bool,
    pub reason: Option<String>,
    pub items: Vec<GateItem>,
}

fn item(id: GateItemId, failure: Option<String>) -> GateItem {
    GateItem {
        id,
        ok: failure.is_none(),
        reason: failure,
    }
}

fn open_failure(state: PrState) -> Option<String> {
    match state {
        PrState::Open => None,
        PrState::Merged => Some(reason::MERGED.to_string()),
        PrState::Closed => Some(reason::CLOSED.to_string()),
        PrState::Unknown => Some(reason::STATE_UNREADABLE.to_string()),
    }
}

fn passed_with(id: GateItemId, note: &str) -> GateItem {
    GateItem {
        id,
        ok: true,
        reason: Some(note.to_string()),
    }
}

fn missing_approvals_sentence(have: u64, required: u64) -> String {
    format!(
        "{}{have}{}{required}{}",
        reason::APPROVALS_PREFIX,
        reason::APPROVALS_OF,
        reason::APPROVALS_SUFFIX
    )
}

fn approved_item(facts: &GateFacts) -> GateItem {
    let id = GateItemId::Approved;
    if facts.reviews_degraded {
        return item(id, Some(reason::REVIEWS_UNREADABLE.to_string()));
    }
    if facts.review_decision == ReviewDecision::ChangesRequested {
        return item(id, Some(reason::NOT_APPROVED.to_string()));
    }
    if facts.review_decision_from_github {
        return match facts.review_decision {
            ReviewDecision::Approved => item(id, None),
            _ => item(id, Some(reason::NOT_APPROVED.to_string())),
        };
    }
    match facts.required_approvals {
        Some(required) if required > 0 => {
            if facts.approving_reviewers >= required {
                item(id, None)
            } else {
                item(
                    id,
                    Some(missing_approvals_sentence(
                        facts.approving_reviewers,
                        required,
                    )),
                )
            }
        }
        _ => passed_with(id, reason::NO_APPROVAL_RULE),
    }
}

fn checks_failure(facts: &GateFacts) -> Option<String> {
    if facts.checks_degraded {
        return Some(reason::CHECKS_UNREADABLE.to_string());
    }
    match facts.checks_verdict {
        ChecksVerdict::Failed => Some(reason::CHECKS_FAILED.to_string()),
        ChecksVerdict::Pending => Some(reason::CHECKS_PENDING.to_string()),
        ChecksVerdict::None | ChecksVerdict::Passed => None,
    }
}

fn threads_failure(facts: &GateFacts) -> Option<String> {
    if !facts.require_thread_resolution || facts.unresolved_threads == 0 {
        return None;
    }
    if facts.unresolved_threads == 1 {
        return Some(reason::ONE_THREAD.to_string());
    }
    Some(format!(
        "{} {}",
        facts.unresolved_threads,
        reason::MANY_THREADS_SUFFIX
    ))
}

/// Evaluates the gate items in order and reports the first failing sentence.
pub fn merge_gate(facts: &GateFacts) -> Gate {
    let items = vec![
        item(GateItemId::Open, open_failure(facts.state)),
        item(
            GateItemId::NotDraft,
            facts.is_draft.then(|| reason::DRAFT.to_string()),
        ),
        item(
            GateItemId::NoChangesRequested,
            (facts.review_decision == ReviewDecision::ChangesRequested)
                .then(|| reason::CHANGES_REQUESTED.to_string()),
        ),
        approved_item(facts),
        item(GateItemId::Checks, checks_failure(facts)),
        item(
            GateItemId::NoConflict,
            (facts.mergeable == Mergeable::Conflicting).then(|| reason::CONFLICT.to_string()),
        ),
        item(GateItemId::Threads, threads_failure(facts)),
    ];
    let reason = items
        .iter()
        .filter(|entry| !entry.ok)
        .find_map(|entry| entry.reason.clone());
    Gate {
        ok: reason.is_none(),
        reason,
        items,
    }
}

/// Whether a required-thread-resolution rule applies to the base branch, and
/// whether the rules read itself failed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BranchRules {
    pub require_thread_resolution: bool,
    pub required_approving_review_count: u64,
    pub degraded: bool,
}

/// The highest `required_approving_review_count` among the `pull_request`
/// rules; 0 when none applies.
pub fn rules_required_approving_review_count(rules: &Value) -> u64 {
    rules
        .as_array()
        .into_iter()
        .flatten()
        .filter(|rule| text(rule, wire::RULE_TYPE).as_deref() == Some(wire::PULL_REQUEST_RULE))
        .filter_map(|rule| {
            rule.get(wire::PARAMETERS)
                .and_then(|parameters| parameters.get(wire::APPROVING_REVIEW_COUNT))
                .and_then(Value::as_u64)
        })
        .max()
        .unwrap_or(0)
}

/// Number of reviewers whose latest opinionated review is an approval.
fn approving_reviewers(review: &ReviewReport) -> u64 {
    review
        .reviewers
        .iter()
        .filter(|reviewer| reviewer.state == ReviewState::Approved)
        .count() as u64
}

/// True when the rules array holds a `pull_request` rule that requires thread
/// resolution.
pub fn rules_require_thread_resolution(rules: &Value) -> bool {
    rules.as_array().is_some_and(|list| {
        list.iter().any(|rule| {
            text(rule, wire::RULE_TYPE).as_deref() == Some(wire::PULL_REQUEST_RULE)
                && rule
                    .get(wire::PARAMETERS)
                    .and_then(|parameters| parameters.get(wire::THREAD_RULE))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
    })
}

/// Reads the rules that apply to `base`. A failed read is degraded, never
/// blocking.
pub fn fetch_branch_rules(repo: &str, base: &str) -> BranchRules {
    let path = format!(
        "{}/{repo}/{}/{base}",
        gh_args::REPOS_SEGMENT,
        gh_args::RULES_SEGMENT
    );
    let parsed = gh_stdout(gh_args::RULES_LABEL, &[gh_args::API, &path])
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok());
    match parsed {
        Some(rules) => BranchRules {
            require_thread_resolution: rules_require_thread_resolution(&rules),
            required_approving_review_count: rules_required_approving_review_count(&rules),
            degraded: false,
        },
        None => BranchRules {
            degraded: true,
            ..BranchRules::default()
        },
    }
}

/// The comparison is only worth a call while the PR can still be merged.
pub fn needs_compare(state: PrState) -> bool {
    state == PrState::Open
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Branch {
    #[serde(rename = "ref")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsReport {
    pub unresolved: u64,
    pub actionable: Option<u64>,
    pub actionable_ids: Option<Vec<String>>,
    pub require_resolution: bool,
}

/// The status document served by `aidp gh pr-status` and the daemon.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    pub repo: String,
    pub number: u64,
    pub url: Option<String>,
    pub title: Option<String>,
    pub state: PrState,
    pub is_draft: bool,
    pub head: Branch,
    pub base: Branch,
    pub author: Option<String>,
    pub viewer: Option<String>,
    pub review: ReviewReport,
    pub checks: ChecksReport,
    pub mergeable: Mergeable,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind_by: Option<u64>,
    pub threads: ThreadsReport,
    pub gate: Gate,
    pub degraded: Vec<DegradedRead>,
    pub read_at: String,
}

fn lowercase_or_default<T: DeserializeOwned + Default>(value: &Value, key: &str) -> T {
    lowercase_enum(value, key).unwrap_or_default()
}

fn unresolved_threads(pr: &Value) -> Option<u64> {
    let threads = nodes(pr, wire::REVIEW_THREADS)?;
    Some(
        threads
            .iter()
            .filter(|thread| {
                !thread
                    .get(wire::IS_RESOLVED)
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .count() as u64,
    )
}

/// Threads needing a response, excluding resolved and author-answered conversations.
fn actionable_threads(pr: &Value, viewer: Option<&str>) -> Option<Vec<String>> {
    let connection = pr.get(wire::REVIEW_THREADS)?;
    if connection
        .get(wire::PAGE_INFO)?
        .get(wire::HAS_NEXT_PAGE)?
        .as_bool()?
    {
        return None;
    }
    let author = pr
        .get(wire::AUTHOR)
        .and_then(|author| author.get(wire::LOGIN))
        .and_then(Value::as_str);
    let mut actionable = Vec::new();
    for thread in nodes(pr, wire::REVIEW_THREADS)? {
        if thread.get(wire::IS_RESOLVED)?.as_bool()? {
            continue;
        }
        let comment = nodes(thread, wire::COMMENTS)?.last()?;
        let login = comment.get(wire::AUTHOR)?.get(wire::LOGIN)?.as_str()?;
        let replied = thread
            .get(wire::COMMENTS)?
            .get(wire::TOTAL_COUNT)?
            .as_u64()?
            > 1;
        if !replied || (Some(login) != author && Some(login) != viewer) {
            actionable.push(format!(
                "{}:{}",
                text(thread, wire::ID)?,
                text(comment, wire::ID)?
            ));
        }
    }
    Some(actionable)
}

fn rollup_of(pr: &Value) -> &Value {
    pr.get(wire::COMMITS)
        .and_then(|commits| commits.get(wire::NODES))
        .and_then(|list| list.get(0))
        .and_then(|node| node.get(wire::COMMIT))
        .and_then(|commit| commit.get(wire::STATUS_CHECK_ROLLUP))
        .unwrap_or(&Value::Null)
}

/// Folds the GraphQL `data` object, the optional `behind_by` and the branch
/// rules into the status document. Pure: every read has already happened.
pub fn build_status(
    repo: &str,
    data: &Value,
    behind_by: Option<u64>,
    rules: BranchRules,
    read_at: String,
) -> Option<PrStatus> {
    let pr = data
        .get(wire::REPOSITORY)?
        .get(wire::PULL_REQUEST)
        .filter(|value| !value.is_null())?;
    let state: PrState = lowercase_or_default(pr, wire::STATE);
    let is_draft = pr
        .get(wire::IS_DRAFT)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let checks = checks_verdict(rollup_of(pr));
    let review = review_state(pr);
    let thread_count = unresolved_threads(pr);
    let viewer = data
        .get(wire::VIEWER)
        .and_then(|viewer| viewer.get(wire::LOGIN))
        .and_then(Value::as_str);
    let actionable_ids = actionable_threads(pr, viewer);
    let threads = ThreadsReport {
        actionable: actionable_ids.as_ref().map(|ids| ids.len() as u64),
        actionable_ids,
        unresolved: thread_count.unwrap_or(0),
        require_resolution: rules.require_thread_resolution,
    };
    let mergeable: Mergeable = lowercase_or_default(pr, wire::MERGEABLE);
    let gate = merge_gate(&GateFacts {
        state,
        is_draft,
        review_decision: review.decision,
        reviews_degraded: review.degraded,
        checks_verdict: checks.verdict,
        checks_degraded: checks.degraded,
        mergeable,
        unresolved_threads: threads.unresolved,
        require_thread_resolution: threads.require_resolution,
        review_decision_from_github: review.source == ReviewSource::Github,
        required_approvals: (!rules.degraded).then_some(rules.required_approving_review_count),
        approving_reviewers: approving_reviewers(&review),
    });
    let mut degraded = Vec::new();
    if checks.degraded {
        degraded.push(DegradedRead::Checks);
    }
    if review.degraded {
        degraded.push(DegradedRead::Reviews);
    }
    if thread_count.is_none() {
        degraded.push(DegradedRead::Threads);
    }
    if rules.degraded {
        degraded.push(DegradedRead::Rules);
    }
    Some(PrStatus {
        repo: repo.to_string(),
        number: pr.get(wire::NUMBER).and_then(Value::as_u64)?,
        url: text(pr, wire::URL),
        title: text(pr, wire::TITLE),
        behind_by: behind_by.filter(|_| needs_compare(state)),
        state,
        is_draft,
        head: Branch {
            name: text(pr, wire::HEAD_REF_NAME).unwrap_or_default(),
            sha: text(pr, wire::HEAD_REF_OID),
        },
        base: Branch {
            name: text(pr, wire::BASE_REF_NAME).unwrap_or_default(),
            sha: None,
        },
        author: pr
            .get(wire::AUTHOR)
            .and_then(|author| text(author, wire::LOGIN)),
        viewer: data
            .get(wire::VIEWER)
            .and_then(|viewer| text(viewer, wire::LOGIN)),
        review,
        checks,
        mergeable,
        threads,
        gate,
        degraded,
        read_at,
    })
}

fn split_slug(repo: &str) -> Result<(&str, &str), IntegrationError> {
    repo.split_once('/').ok_or_else(|| {
        IntegrationError::NotFound(format!("{} {repo}", gh_args::NO_REPOSITORY_MESSAGE))
    })
}

fn fetch_graphql(repo: &str, number: u64) -> Result<Value, IntegrationError> {
    let (owner, name) = split_slug(repo)?;
    let query_arg = format!("{}{PR_STATUS_QUERY}", gh_args::QUERY_FIELD);
    let owner_arg = format!("{}{owner}", gh_args::OWNER_FIELD);
    let name_arg = format!("{}{name}", gh_args::NAME_FIELD);
    let number_arg = format!("{}{number}", gh_args::NUMBER_FIELD);
    let body = gh_stdout(
        gh_args::GRAPHQL_LABEL,
        &[
            gh_args::API,
            gh_args::GRAPHQL,
            GH_PAGINATE,
            GH_SLURP,
            gh_args::STRING_FIELD,
            &query_arg,
            gh_args::STRING_FIELD,
            &owner_arg,
            gh_args::STRING_FIELD,
            &name_arg,
            gh_args::TYPED_FIELD,
            &number_arg,
        ],
    )
    .map_err(|error| IntegrationError::Upstream(error.to_string()))?;
    let parsed: Value = serde_json::from_str(&body)
        .map_err(|error| IntegrationError::Upstream(error.to_string()))?;
    merge_graphql_pages(parsed)
        .ok_or_else(|| IntegrationError::Upstream(gh_args::MISSING_PR_PAYLOAD.to_string()))
}

/// Combines paginated GraphQL review threads while retaining the first page's PR facts.
fn merge_graphql_pages(parsed: Value) -> Option<Value> {
    let pages = match parsed {
        Value::Array(pages) => pages,
        page => vec![page],
    };
    let mut pages = pages.into_iter();
    let mut data = pages.next()?.get(wire::DATA)?.clone();
    for page in pages {
        let connection = page
            .get(wire::DATA)?
            .get(wire::REPOSITORY)?
            .get(wire::PULL_REQUEST)?
            .get(wire::REVIEW_THREADS)?;
        let target = data
            .get_mut(wire::REPOSITORY)?
            .get_mut(wire::PULL_REQUEST)?
            .get_mut(wire::REVIEW_THREADS)?;
        target
            .get_mut(wire::NODES)?
            .as_array_mut()?
            .extend(connection.get(wire::NODES)?.as_array()?.iter().cloned());
        target[wire::PAGE_INFO] = connection.get(wire::PAGE_INFO)?.clone();
    }
    Some(data)
}

fn fetch_behind_by(repo: &str, base: &str, head_sha: &str) -> Option<u64> {
    let path = format!(
        "{}/{repo}/{}/{base}{}{head_sha}",
        gh_args::REPOS_SEGMENT,
        gh_args::COMPARE_SEGMENT,
        gh_args::COMPARE_RANGE_SEPARATOR
    );
    let body = gh_stdout(gh_args::COMPARE_LABEL, &[gh_args::API, &path]).ok()?;
    serde_json::from_str::<Value>(&body)
        .ok()?
        .get(wire::BEHIND_BY)?
        .as_u64()
}

/// Reads a PR's status with an injected rules source, so a caller can cache
/// rules apart from the PR. `rules` receives `(repo, base)`.
pub fn pr_status_with_rules(
    repo: &str,
    number: u64,
    rules: &dyn Fn(&str, &str) -> BranchRules,
) -> Result<PrStatus, IntegrationError> {
    let data = fetch_graphql(repo, number)?;
    let pr = data
        .get(wire::REPOSITORY)
        .and_then(|repository| repository.get(wire::PULL_REQUEST))
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            IntegrationError::NotFound(format!("{} #{number}", gh_args::NO_PR_MESSAGE))
        })?;
    let state: PrState = lowercase_or_default(pr, wire::STATE);
    let base = text(pr, wire::BASE_REF_NAME).unwrap_or_default();
    let behind_by = match text(pr, wire::HEAD_REF_OID) {
        Some(sha) if needs_compare(state) => fetch_behind_by(repo, &base, &sha),
        _ => None,
    };
    let branch_rules = rules(repo, &base);
    let read_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    build_status(repo, &data, behind_by, branch_rules, read_at)
        .ok_or_else(|| IntegrationError::Upstream(gh_args::MISSING_PR_PAYLOAD.to_string()))
}

/// Reads a PR's status. `repo` and `number` default to the current checkout
/// (or `dir`) when omitted.
pub fn pr_status(
    repo: Option<&str>,
    number: Option<u64>,
    dir: Option<&Path>,
) -> Result<PrStatus, IntegrationError> {
    let run = || -> Result<PrStatus, IntegrationError> {
        let number_text = number.map(|value| value.to_string());
        let resolved_number = match number {
            Some(value) => value,
            None => pr_resolve_checked(number_text.as_deref(), repo)?
                .and_then(|pr| pr.get(wire::NUMBER).and_then(Value::as_u64))
                .ok_or_else(|| IntegrationError::NotFound(gh_args::NO_PR_MESSAGE.to_string()))?,
        };
        let slug = repo_slug(repo).ok_or_else(|| {
            IntegrationError::NotFound(gh_args::NO_REPOSITORY_MESSAGE.to_string())
        })?;
        pr_status_with_rules(&slug, resolved_number, &fetch_branch_rules)
    };
    match dir {
        Some(directory) => in_dir(directory, run),
        None => run(),
    }
}

#[cfg(test)]
#[path = "tests/units/pr_tests.rs"]
mod tests;
