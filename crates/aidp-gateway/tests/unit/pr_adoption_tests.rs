use super::*;
use aidp_tickets::models::enums::pr_state::PrState;

const OPEN_LIST: &[u8] = include_bytes!("../fixtures/ticket_adoption/gh_pr_list_open.json");
const MERGED_LIST: &[u8] = include_bytes!("../fixtures/ticket_adoption/gh_pr_list_merged.json");
const CLOSED_LIST: &[u8] = include_bytes!("../fixtures/ticket_adoption/gh_pr_list_closed.json");
const OPEN_VIEW: &str = include_str!("../fixtures/ticket_adoption/open_pr_view.json");
const CLOSED_VIEW: &str = include_str!("../fixtures/ticket_adoption/closed_pr_view.json");
const TICKET_KEY: &str = "SVC-1";
const FALLBACK_BRANCH: &str = "feat/SVC-1";
const OPEN_LIST_NUMBER: u64 = 61;

/// An open pull request in a `gh pr list` payload is adoptable as open.
#[test]
fn open_list_classifies_as_open() {
    let candidate = pick_from_list(OPEN_LIST, FALLBACK_BRANCH).unwrap();

    assert_eq!(candidate.adopted.pr.state, Some(PrState::Open));
    assert_eq!(candidate.adopted.pr.number, Some(OPEN_LIST_NUMBER));
}

/// A merged pull request in a `gh pr list` payload is adoptable as merged.
#[test]
fn merged_list_classifies_as_merged() {
    let candidate = pick_from_list(MERGED_LIST, FALLBACK_BRANCH).unwrap();

    assert_eq!(candidate.adopted.pr.state, Some(PrState::Merged));
}

/// A closed unmerged pull request is never adopted.
#[test]
fn closed_list_yields_no_candidate() {
    assert!(pick_from_list(CLOSED_LIST, FALLBACK_BRANCH).is_none());
}

/// The fallback branch stands in when `gh` reports no head branch.
#[test]
fn missing_head_branch_falls_back_to_the_feature_branch() {
    let view: GhPrView = serde_json::from_str(OPEN_VIEW).unwrap();

    match classify_view(&view, &feature_branch_of(TICKET_KEY)) {
        PrClassification::Adoptable(candidate) => {
            assert_eq!(candidate.adopted.branch, FALLBACK_BRANCH);
            assert!(candidate.is_draft);
        }
        PrClassification::ClosedUnmerged => panic!("an open pull request is adoptable"),
    }
}

/// A `gh pr view` value of a closed pull request has no candidate.
#[test]
fn closed_view_value_yields_no_candidate() {
    let value: Value = serde_json::from_str(CLOSED_VIEW).unwrap();

    assert!(candidate_from_value(&value, FALLBACK_BRANCH).is_none());
}
