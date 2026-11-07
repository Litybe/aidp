//! Tests for adopting a ticket onto an existing pull request.

use super::*;
use crate::model::{Pr, Source};
use crate::models::enums::pr_state::PrState;

#[path = "test_support.rs"]
#[allow(clippy::duplicate_mod)]
mod test_support;

use test_support::ticket;

/// Pull request URL the adopted ticket points at.
const PR_URL: &str = "https://github.com/example/svc/pull/7";

/// Pull request number the adopted ticket points at.
const PR_NUMBER: u64 = 7;

/// Branch of the adopted pull request.
const PR_BRANCH: &str = "feat/PIXEL-2";

/// Adopting an open pull request records it, its branch and the adopted flag.
#[test]
fn adopt_open_pr_sets_pr_branch_and_adopted_flag() {
    let mut adopted = ticket("PIXEL-2", Source::Github);
    let pr = AdoptedPr {
        pr: Pr {
            url: PR_URL.to_string(),
            number: Some(PR_NUMBER),
            state: Some(PrState::Open),
            commented: false,
        },
        branch: PR_BRANCH.to_string(),
    };
    let event = adopt(&mut adopted, pr, Actor::Dispatcher, Utc::now()).expect("adopt");
    assert_eq!(adopted.status, Status::InReview);
    assert_eq!(event.to, Status::InReview);
    let recorded = adopted.autopilot.pr.as_ref().expect("pr set");
    assert_eq!(recorded.url, PR_URL);
    assert_eq!(recorded.number, Some(PR_NUMBER));
    assert_eq!(recorded.state, Some(PrState::Open));
    assert_eq!(adopted.autopilot.branch.as_deref(), Some(PR_BRANCH));
    assert!(adopted.autopilot.adopted);
}
