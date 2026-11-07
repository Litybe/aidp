//! Finds the pull request a ticket already has and classifies it for adoption.

use crate::models::constants::process::GH_PROGRAM;
use crate::models::enums::PrClassification;
use crate::models::{GhPrView, PrCandidate};
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec};
use aidp_integrations::gh::{in_dir, repo_slug};
use aidp_integrations::pr::PrState as GhState;
use aidp_tickets::model::Pr;
use aidp_tickets::models::adopted_pr::AdoptedPr;
use aidp_tickets::models::enums::pr_state::PrState;
use serde_json::Value;
use std::path::Path;

const GH_PR: &str = "pr";
const GH_LIST: &str = "list";
const GH_VIEW: &str = "view";
const GH_REPO_FLAG: &str = "--repo";
const GH_SEARCH_FLAG: &str = "--search";
const GH_STATE_FLAG: &str = "--state";
const GH_STATE_ALL: &str = "all";
const GH_JSON_FLAG: &str = "--json";
const GH_ADOPTION_FIELDS: &str = "number,url,state,headRefName,isDraft";
const SEARCH_TITLE_SUFFIX: &str = "in:title";
const FEATURE_BRANCH_PREFIX: &str = "feat/";

/// The branch the dispatcher opens for `key`.
pub fn feature_branch_of(key: &str) -> String {
    format!("{FEATURE_BRANCH_PREFIX}{key}")
}

/// Open and merged pull requests are adoptable; any other state is not.
/// `fallback_branch` stands in when `gh` did not report a head branch.
pub fn classify_view(view: &GhPrView, fallback_branch: &str) -> PrClassification {
    let state = match view.state {
        GhState::Open => PrState::Open,
        GhState::Merged => PrState::Merged,
        GhState::Closed | GhState::Unknown => return PrClassification::ClosedUnmerged,
    };
    PrClassification::Adoptable(PrCandidate {
        adopted: AdoptedPr {
            pr: Pr {
                url: view.url.clone(),
                number: Some(view.number),
                state: Some(state),
                commented: false,
            },
            branch: view
                .head_ref_name
                .clone()
                .filter(|branch| !branch.is_empty())
                .unwrap_or_else(|| fallback_branch.to_string()),
        },
        is_draft: view.is_draft,
    })
}

/// The adoptable pull request among a `gh pr list` payload, an open one before a merged one.
pub fn pick_from_list(payload: &[u8], fallback_branch: &str) -> Option<PrCandidate> {
    let views: Vec<GhPrView> = serde_json::from_slice(payload).ok()?;
    let mut merged = None;
    for view in &views {
        if let PrClassification::Adoptable(candidate) = classify_view(view, fallback_branch) {
            match candidate.adopted.pr.state {
                Some(PrState::Open) => return Some(candidate),
                _ => merged = merged.or(Some(candidate)),
            }
        }
    }
    merged
}

/// The adoptable pull request in a `gh pr view` value, `None` when closed unmerged or malformed.
pub fn candidate_from_value(value: &Value, fallback_branch: &str) -> Option<PrCandidate> {
    let view: GhPrView = serde_json::from_value(value.clone()).ok()?;
    match classify_view(&view, fallback_branch) {
        PrClassification::Adoptable(candidate) => Some(candidate),
        PrClassification::ClosedUnmerged => None,
    }
}

/// The `gh pr list` argument vector that searches `key` in pull request titles.
pub fn list_args(slug: &str, key: &str) -> Vec<String> {
    [
        GH_PR,
        GH_LIST,
        GH_REPO_FLAG,
        slug,
        GH_SEARCH_FLAG,
        &format!("{key} {SEARCH_TITLE_SUFFIX}"),
        GH_STATE_FLAG,
        GH_STATE_ALL,
        GH_JSON_FLAG,
        GH_ADOPTION_FIELDS,
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect()
}

/// The `gh pr view` argument vector for a pull request url or number.
pub fn view_args(reference: &str) -> Vec<String> {
    [GH_PR, GH_VIEW, reference, GH_JSON_FLAG, GH_ADOPTION_FIELDS]
        .iter()
        .map(|arg| arg.to_string())
        .collect()
}

fn run_gh(cwd: &Path, args: Vec<String>) -> Option<Vec<u8>> {
    let spec = ProcessSpec::new(GH_PROGRAM)
        .args(args)
        .cwd(cwd.to_path_buf());
    let output = OsProcessRunner.output(&spec).ok()?;
    output.status.success().then_some(output.stdout)
}

/// Resolves a pull request by url or number in the project at `cwd`.
pub fn view_pr(cwd: &Path, reference: &str) -> Option<GhPrView> {
    let stdout = run_gh(cwd, view_args(reference))?;
    serde_json::from_slice(&stdout).ok()
}

/// The pull request `key` already has: its `feat/<key>` branch first, then a title search.
/// It shells out to `gh`, so it runs at launch time only and never in the per-tick plan.
pub fn find_for_ticket(cwd: &Path, key: &str) -> Option<PrCandidate> {
    let branch = feature_branch_of(key);
    if let Some(candidate) = crate::changes::pr_for_branch(cwd, &branch)
        .and_then(|value| candidate_from_value(&value, &branch))
    {
        return Some(candidate);
    }
    let slug = in_dir(cwd, || repo_slug(None))?;
    let stdout = run_gh(cwd, list_args(&slug, key))?;
    pick_from_list(&stdout, &branch)
}

#[cfg(test)]
#[path = "../tests/unit/pr_adoption_tests.rs"]
mod pr_adoption_tests;
