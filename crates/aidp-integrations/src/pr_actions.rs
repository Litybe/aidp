//! Mutating pull request actions, shelling out to `gh`.

use crate::gh::{gh, repo_slug};
use crate::models::merge_outcome::MergeOutcome;
use crate::models::pr_view_fields::PrViewFields;
use crate::models::ready_outcome::ReadyOutcome;
use crate::models::thread_result::ThreadResult;
use crate::models::update_outcome::UpdateOutcome;
use crate::pr::{pr_status, Mergeable, PrState};
use crate::IntegrationError;
use serde_json::Value;
use std::time::{Duration, Instant};

const NO_REPOSITORY: &str = "cannot resolve the repository";
const PR_NOT_OPEN: &str = "the pull request is merged or closed";
const SQUASH_DISABLED: &str = "squash disabled on this repository";
const MERGEABLE_TIMEOUT: Duration = Duration::from_secs(60);
const MERGEABLE_INTERVAL: Duration = Duration::from_secs(3);
const MERGEABLE_UNKNOWN: &str =
    "github did not compute `mergeable` after the rebase within the allowed time";
const REBASE_NOT_APPLIED: &str = "github did not apply the rebase within the allowed time";
const CONFLICT_AFTER_REBASE: &str = "the pull request conflicts with its base after the rebase";
const BRANCH_MOVED: &str = "the branch moved since you read it";
const HEAD_MISMATCH_MARKER: &str = "head branch was modified";
const REBASE_CONFLICT_MARKER: &str = "conflict";
const ALLOW_SQUASH_KEY: &str = "allow_squash_merge";
const PR_VIEW_FIELDS: &str = "isDraft,state";
const HEAD_VIEW_FIELDS: &str = "headRefOid";
const REPLY_MUTATION: &str = "mutation($id:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$id,body:$body}){comment{id}}}";
const RESOLVE_MUTATION: &str =
    "mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{id}}}";
const UNRESOLVE_MUTATION: &str =
    "mutation($id:ID!){unresolveReviewThread(input:{threadId:$id}){thread{id}}}";

fn upstream(error: anyhow::Error) -> IntegrationError {
    IntegrationError::Upstream(error.to_string())
}

fn slug_of(repo: Option<&str>) -> Result<String, IntegrationError> {
    repo_slug(repo).ok_or_else(|| IntegrationError::NotFound(NO_REPOSITORY.to_string()))
}

fn view_fields(
    number: &str,
    fields: &str,
    repo: Option<&str>,
) -> Result<PrViewFields, IntegrationError> {
    let text = gh(&["pr", "view", number, "--json", fields], repo).map_err(upstream)?;
    serde_json::from_str(&text).map_err(|error| IntegrationError::Upstream(error.to_string()))
}

/// Marks the PR ready, or back to draft with `undo`. No mutation when it
/// already is in the wanted state; merged or closed PRs are refused.
pub fn pr_ready(pr: u64, repo: Option<&str>, undo: bool) -> Result<ReadyOutcome, IntegrationError> {
    let number = pr.to_string();
    let view = view_fields(&number, PR_VIEW_FIELDS, repo)?;
    if view.state != PrState::Open {
        return Err(IntegrationError::Conflict(PR_NOT_OPEN.to_string()));
    }
    let draft = view.is_draft;
    if draft == undo {
        return Ok(ReadyOutcome {
            changed: false,
            draft,
        });
    }
    let mut args = vec!["pr", "ready", number.as_str()];
    if undo {
        args.push("--undo");
    }
    gh(&args, repo).map_err(upstream)?;
    Ok(ReadyOutcome {
        changed: true,
        draft: undo,
    })
}

/// Rebases the PR branch onto its base through GitHub, then re-reads the head. A rebase replays
/// each commit and can conflict where the final state does not: the base is then merged instead.
pub fn pr_update_branch(pr: u64, repo: Option<&str>) -> Result<UpdateOutcome, IntegrationError> {
    let number = pr.to_string();
    if let Err(error) = gh(&["pr", "update-branch", &number, "--rebase"], repo) {
        if !error
            .to_string()
            .to_lowercase()
            .contains(REBASE_CONFLICT_MARKER)
        {
            return Err(upstream(error));
        }
        gh(&["pr", "update-branch", &number], repo).map_err(upstream)?;
    }
    let view = view_fields(&number, HEAD_VIEW_FIELDS, repo)?;
    Ok(UpdateOutcome {
        head_sha: view.head_ref_oid,
    })
}

fn squash_allowed(slug: &str) -> Result<bool, IntegrationError> {
    let text = gh(&["api", &format!("repos/{slug}")], None).map_err(upstream)?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| IntegrationError::Upstream(error.to_string()))?;
    Ok(value
        .get(ALLOW_SQUASH_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

fn delete_branch(slug: &str, branch: &str) -> anyhow::Result<()> {
    let path = format!("repos/{slug}/git/refs/heads/{branch}");
    gh(&["api", "-X", "DELETE", &path], None).map(|_| ())
}

/// How long, and how often, to poll GitHub for `mergeable` after a rebase.
#[derive(Debug, Clone, Copy)]
pub struct MergeablePoll {
    pub timeout: Duration,
    pub interval: Duration,
}

impl Default for MergeablePoll {
    fn default() -> Self {
        MergeablePoll {
            timeout: MERGEABLE_TIMEOUT,
            interval: MERGEABLE_INTERVAL,
        }
    }
}

/// GitHub applies `update-branch` asynchronously: right after the call the PR
/// may still report its old head and its old `mergeable`. Polls `read` until the
/// head has left `previous_head` and GitHub knows whether it merges, then
/// returns the rebased head.
pub(crate) fn wait_for_rebased_head(
    poll: MergeablePoll,
    previous_head: &str,
    mut read: impl FnMut() -> Result<(Option<String>, Mergeable), IntegrationError>,
) -> Result<String, IntegrationError> {
    let deadline = Instant::now() + poll.timeout;
    loop {
        let (head, mergeable) = read()?;
        let rebased_head = head.filter(|sha| sha != previous_head);
        let head_moved = rebased_head.is_some();
        if let Some(sha) = rebased_head {
            match mergeable {
                Mergeable::Mergeable => return Ok(sha),
                Mergeable::Conflicting => {
                    return Err(IntegrationError::Conflict(
                        CONFLICT_AFTER_REBASE.to_string(),
                    ))
                }
                Mergeable::Unknown => {}
            }
        }
        if Instant::now() >= deadline {
            let reason = if head_moved {
                MERGEABLE_UNKNOWN
            } else {
                REBASE_NOT_APPLIED
            };
            return Err(IntegrationError::Conflict(reason.to_string()));
        }
        std::thread::sleep(poll.interval);
    }
}

/// Squash-merges the PR when its gate is open, pinned to `expected_head_sha`.
/// A branch behind its base is rebased first; the merge then waits (bounded) for
/// GitHub to recompute `mergeable` and is pinned to the rebased head. `Ok` always
/// means merged: a merge that cannot happen is an error.
pub fn pr_merge(
    pr: u64,
    repo: Option<&str>,
    expected_head_sha: &str,
    keep_branch: bool,
) -> Result<MergeOutcome, IntegrationError> {
    pr_merge_polling(
        pr,
        repo,
        expected_head_sha,
        keep_branch,
        MergeablePoll::default(),
    )
}

pub fn pr_merge_polling(
    pr: u64,
    repo: Option<&str>,
    expected_head_sha: &str,
    keep_branch: bool,
    poll: MergeablePoll,
) -> Result<MergeOutcome, IntegrationError> {
    let status = pr_status(repo, Some(pr), None)?;
    if !status.gate.ok {
        let reason = status.gate.reason.clone().unwrap_or_default();
        return Err(IntegrationError::Conflict(reason));
    }
    let slug = slug_of(repo)?;
    if !squash_allowed(&slug)? {
        return Err(IntegrationError::Conflict(SQUASH_DISABLED.to_string()));
    }
    let number = pr.to_string();
    let mut rebased = false;
    let mut expected_head_sha = expected_head_sha.to_string();
    if status.behind_by.unwrap_or(0) > 0 {
        let previous_head = status
            .head
            .sha
            .clone()
            .unwrap_or_else(|| expected_head_sha.clone());
        pr_update_branch(pr, repo)?;
        rebased = true;
        expected_head_sha = wait_for_rebased_head(poll, &previous_head, || {
            let head = view_fields(&number, HEAD_VIEW_FIELDS, repo)?.head_ref_oid;
            let mergeable = pr_status(repo, Some(pr), None)?.mergeable;
            Ok((head, mergeable))
        })?;
    }
    let expected_head_sha = expected_head_sha.as_str();
    let title = status.title.clone().unwrap_or_default();
    let subject = format!("{title} (#{pr})");
    let merge = gh(
        &[
            "pr",
            "merge",
            &number,
            "--squash",
            "--match-head-commit",
            expected_head_sha,
            "--subject",
            &subject,
            "--body",
            "",
        ],
        repo,
    );
    if let Err(error) = merge {
        let message = error.to_string();
        if message.to_lowercase().contains(HEAD_MISMATCH_MARKER) {
            return Err(IntegrationError::Conflict(BRANCH_MOVED.to_string()));
        }
        return Err(IntegrationError::Upstream(message));
    }
    let mut outcome = MergeOutcome {
        merged: true,
        rebased,
        sha: Some(expected_head_sha.to_string()),
        ..MergeOutcome::default()
    };
    if !keep_branch {
        match delete_branch(&slug, &status.head.name) {
            Ok(()) => outcome.branch_deleted = true,
            Err(error) => outcome.detail.push(error.to_string()),
        }
    }
    Ok(outcome)
}

fn run_mutation(query: &str, thread_id: &str, body: Option<&str>) -> anyhow::Result<()> {
    let query_arg = format!("query={query}");
    let id_arg = format!("id={thread_id}");
    let mut args = vec!["api", "graphql", "-f", &query_arg, "-f", &id_arg];
    let body_arg = body.map(|text| format!("body={text}"));
    if let Some(arg) = &body_arg {
        args.extend(["-f", arg]);
    }
    gh(&args, None).map(|_| ())
}

/// Posts a reply on a review thread.
pub fn thread_reply(thread_id: &str, body: &str) -> Result<(), IntegrationError> {
    run_mutation(REPLY_MUTATION, thread_id, Some(body)).map_err(upstream)
}

/// Resolves, or with `unresolve` reopens, each thread in order; a failure on
/// one thread is recorded and the rest still run.
pub fn thread_resolve(ids: &[String], unresolve: bool) -> Vec<ThreadResult> {
    let query = if unresolve {
        UNRESOLVE_MUTATION
    } else {
        RESOLVE_MUTATION
    };
    ids.iter()
        .map(|id| match run_mutation(query, id, None) {
            Ok(()) => ThreadResult {
                id: id.clone(),
                ok: true,
                error: None,
            },
            Err(error) => ThreadResult {
                id: id.clone(),
                ok: false,
                error: Some(error.to_string()),
            },
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/pr_actions_tests.rs"]
mod tests;
