//! Pull-request status served by the daemon: a short-lived cache in front of
//! `aidp_integrations::pr`, invalidated by webhooks, plus the two route handlers.

use crate::bus::Bus;
use crate::models::enums::pr_refusal_reason::PrRefusalReason;
use crate::models::enums::ApiStatus;
use crate::models::ApiError;
use crate::sync::lock;
use aidp_core::clock::Clock;
use aidp_core::models::enums::HasErrorKind;
use aidp_integrations::gh::{in_dir, pr_resolve_checked, repo_slug};
use aidp_integrations::models::constants::branch::{FEATURE_BRANCH_PREFIX, FIX_BRANCH_PREFIX};
use aidp_integrations::pr::{fetch_branch_rules, pr_status_with_rules, BranchRules};
use aidp_integrations::IntegrationError;
use aidp_tickets::model::Ticket;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub const STATUS_TTL_MS: i64 = 20_000;
pub const RULES_TTL_MS: i64 = 3_600_000;
pub const PR_STATUS_EVENT: &str = "pr.status";
pub const NO_PR_REASON_PREFIX: &str = "no pull request for ";
pub const TICKET_BRANCH_PREFIX: &str = FEATURE_BRANCH_PREFIX;
/// Every branch prefix a ticket's work can use, the default one first.
pub const TICKET_BRANCH_PREFIXES: [&str; 2] = [TICKET_BRANCH_PREFIX, FIX_BRANCH_PREFIX];

const EVENT_PULL_REQUEST: &str = "pull_request";
const EVENT_PULL_REQUEST_REVIEW: &str = "pull_request_review";
const EVENT_PULL_REQUEST_REVIEW_COMMENT: &str = "pull_request_review_comment";
const EVENT_CHECK_RUN: &str = "check_run";
const EVENT_CHECK_SUITE: &str = "check_suite";
const KEY_REPOSITORY: &str = "repository";
const KEY_FULL_NAME: &str = "full_name";
const KEY_NUMBER: &str = "number";
const KEY_PULL_REQUESTS: &str = "pull_requests";
const KEY_REPO: &str = "repo";
const KEY_SHA: &str = "sha";
const KEY_HEAD: &str = "head";
const EVENT_STATUS: &str = "status";
const KEY_ERROR: &str = "error";
const KEY_CODE: &str = "code";
const KEY_REASON: &str = "reason";
const URL_SEPARATOR: char = '/';

/// A failed route: the status/message pair plus an optional machine-read reason.
#[derive(Debug)]
pub struct PrRouteError {
    pub error: ApiError,
    pub reason: Option<PrRefusalReason>,
}

impl PrRouteError {
    pub fn status(&self) -> u16 {
        self.error.status.status()
    }

    pub fn body(&self) -> Value {
        let mut body = json!({
            KEY_ERROR: self.error.message,
            KEY_CODE: self.error.status.code(),
        });
        if let Some(reason) = self.reason {
            body[KEY_REASON] = json!(reason);
        }
        body
    }
}

impl From<IntegrationError> for PrRouteError {
    fn from(error: IntegrationError) -> Self {
        PrRouteError {
            error: ApiError {
                status: error.kind().into(),
                message: error.to_string(),
            },
            reason: None,
        }
    }
}

type StatusKey = (String, u64);

/// Statuses (short TTL) and branch rules (long TTL), time read from the injected clock.
pub struct PrStatusCache {
    clock: Box<dyn Clock>,
    statuses: Mutex<HashMap<StatusKey, (Value, i64)>>,
    rules: Mutex<HashMap<String, (BranchRules, i64)>>,
}

impl PrStatusCache {
    pub fn new(clock: Box<dyn Clock>) -> Self {
        PrStatusCache {
            clock,
            statuses: Mutex::new(HashMap::new()),
            rules: Mutex::new(HashMap::new()),
        }
    }

    /// A fresh cached status, else `fetch`'s result, stored. No lock is held during `fetch`.
    pub fn get(
        &self,
        repo: &str,
        number: u64,
        refresh: bool,
        fetch: impl FnOnce() -> Result<Value, IntegrationError>,
    ) -> Result<Value, IntegrationError> {
        let key = (repo.to_string(), number);
        let now = self.clock.now_ms();
        if !refresh {
            let cached = lock(&self.statuses).get(&key).cloned();
            if let Some((value, stored_at)) = cached {
                if now - stored_at < STATUS_TTL_MS {
                    return Ok(value);
                }
            }
        }
        let value = fetch()?;
        lock(&self.statuses).insert(key, (value.clone(), self.clock.now_ms()));
        Ok(value)
    }

    /// Branch rules for `repo/base`, cached for an hour unless the read was degraded.
    pub fn rules(
        &self,
        repo: &str,
        base: &str,
        fetch: impl FnOnce() -> BranchRules,
    ) -> BranchRules {
        let key = format!("{repo}{URL_SEPARATOR}{base}");
        let now = self.clock.now_ms();
        let cached = lock(&self.rules).get(&key).copied();
        if let Some((rules, stored_at)) = cached {
            if now - stored_at < RULES_TTL_MS {
                return rules;
            }
        }
        let rules = fetch();
        if !rules.degraded {
            lock(&self.rules).insert(key, (rules, self.clock.now_ms()));
        }
        rules
    }

    /// Drops the cached status and tells subscribers the PR changed.
    pub fn invalidate(&self, bus: &Bus, repo: &str, number: u64) {
        lock(&self.statuses).remove(&(repo.to_string(), number));
        bus.emit(
            PR_STATUS_EVENT,
            json!({ KEY_REPO: repo, KEY_NUMBER: number }),
        );
    }

    /// Invalidates every cached PR of `repo` whose status head is `sha`.
    pub fn invalidate_by_sha(&self, bus: &Bus, repo: &str, sha: &str) {
        let numbers: Vec<u64> = lock(&self.statuses)
            .iter()
            .filter(|((cached_repo, _), (status, _))| {
                cached_repo == repo
                    && status
                        .get(KEY_HEAD)
                        .and_then(|head| head.get(KEY_SHA))
                        .and_then(Value::as_str)
                        == Some(sha)
            })
            .map(|((_, number), _)| *number)
            .collect();
        for number in numbers {
            self.invalidate(bus, repo, number);
        }
    }

    pub fn cached_count(&self) -> usize {
        lock(&self.statuses).len()
    }
}

fn pull_request_numbers(container: &Value) -> Vec<u64> {
    container
        .get(KEY_PULL_REQUESTS)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| entry.get(KEY_NUMBER).and_then(Value::as_u64))
                .collect()
        })
        .unwrap_or_default()
}

/// The `(repo, number)` pairs a webhook delivery concerns; empty for a `status` event or an unknown one.
/// A `status` event is resolved by commit through [`status_target_from_webhook`].
pub fn pr_targets_from_webhook(event: &str, body: &Value) -> Vec<(String, u64)> {
    let Some(repo) = body
        .get(KEY_REPOSITORY)
        .and_then(|repository| repository.get(KEY_FULL_NAME))
        .and_then(Value::as_str)
    else {
        return Vec::new();
    };
    let numbers = match event {
        EVENT_PULL_REQUEST | EVENT_PULL_REQUEST_REVIEW | EVENT_PULL_REQUEST_REVIEW_COMMENT => body
            .get(EVENT_PULL_REQUEST)
            .and_then(|pull_request| pull_request.get(KEY_NUMBER))
            .and_then(Value::as_u64)
            .or_else(|| body.get(KEY_NUMBER).and_then(Value::as_u64))
            .into_iter()
            .collect(),
        EVENT_CHECK_RUN | EVENT_CHECK_SUITE => body
            .get(event)
            .map(pull_request_numbers)
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    numbers
        .into_iter()
        .map(|number| (repo.to_string(), number))
        .collect()
}

/// The `(repo, sha)` a `status` webhook delivery concerns, if it is one.
pub fn status_target_from_webhook(event: &str, body: &Value) -> Option<(String, String)> {
    if event != EVENT_STATUS {
        return None;
    }
    let repo = body
        .get(KEY_REPOSITORY)
        .and_then(|repository| repository.get(KEY_FULL_NAME))
        .and_then(Value::as_str)?;
    let sha = body.get(KEY_SHA).and_then(Value::as_str)?;
    Some((repo.to_string(), sha.to_string()))
}

/// Status of one PR, through the cache.
pub fn handle_status(
    cache: &PrStatusCache,
    repo: &str,
    number: u64,
    refresh: bool,
) -> Result<Value, PrRouteError> {
    let value = cache.get(repo, number, refresh, || {
        let rules = |rules_repo: &str, base: &str| {
            cache.rules(rules_repo, base, || fetch_branch_rules(rules_repo, base))
        };
        let status = pr_status_with_rules(repo, number, &rules)?;
        serde_json::to_value(status).map_err(|error| IntegrationError::Upstream(error.to_string()))
    })?;
    Ok(value)
}

fn number_from_pr_url(url: &str) -> Option<u64> {
    url.rsplit(URL_SEPARATOR).next()?.parse().ok()
}

fn no_pr_error(branch: &str) -> PrRouteError {
    PrRouteError {
        error: ApiError {
            status: ApiStatus::NotFound,
            message: format!("{NO_PR_REASON_PREFIX}{branch}"),
        },
        reason: Some(PrRefusalReason::NoPullRequest),
    }
}

fn resolve_branch_pr(project_dir: &Path, branch: &str) -> Result<Option<u64>, PrRouteError> {
    let resolved = in_dir(project_dir, || pr_resolve_checked(Some(branch), None));
    match resolved {
        Ok(found) => Ok(found.and_then(|pr| pr.get(KEY_NUMBER).and_then(Value::as_u64))),
        Err(IntegrationError::NotFound(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// The ticket key of a `feat/<KEY>` or `fix/<KEY>` branch, `None` for any other branch.
pub fn ticket_key_of_branch(branch: &str) -> Option<&str> {
    TICKET_BRANCH_PREFIXES
        .iter()
        .find_map(|prefix| branch.strip_prefix(prefix))
}

/// The number of the PR of `feat/<key>`, else of `fix/<key>`, in `project_dir`.
fn resolve_ticket_branch_pr(project_dir: &Path, key: &str) -> Result<Option<u64>, PrRouteError> {
    for prefix in TICKET_BRANCH_PREFIXES {
        if let Some(number) = resolve_branch_pr(project_dir, &format!("{prefix}{key}"))? {
            return Ok(Some(number));
        }
    }
    Ok(None)
}

/// Status of a ticket's PR: `autopilot.pr` first, else the PR of `feat/<KEY>` or `fix/<KEY>` in `project_dir`.
pub fn handle_ticket_pr(
    cache: &PrStatusCache,
    ticket: &Ticket,
    key: &str,
    project_dir: &Path,
    refresh: bool,
) -> Result<Value, PrRouteError> {
    let branch = format!("{TICKET_BRANCH_PREFIX}{key}");
    let recorded = ticket
        .autopilot
        .pr
        .as_ref()
        .and_then(|pr| pr.number.or_else(|| number_from_pr_url(&pr.url)));
    let number = match recorded {
        Some(number) => number,
        None => resolve_ticket_branch_pr(project_dir, key)?.ok_or_else(|| no_pr_error(&branch))?,
    };
    let repo = in_dir(project_dir, || repo_slug(None)).ok_or_else(|| no_pr_error(&branch))?;
    handle_status(cache, &repo, number, refresh)
}

#[cfg(test)]
#[path = "tests/units/prs_tests.rs"]
mod tests;
