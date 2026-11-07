//! Tests for blocker-aware planning, flow routes and the skip reason wire forms.

use super::*;
use crate::config::DispatcherConfig;
use crate::model::{Autopilot, Source};
use crate::models::blocker_link::BlockerLink;
use crate::models::enums::issue_state::IssueState;

#[path = "test_support.rs"]
#[allow(clippy::duplicate_mod)]
mod test_support;

use test_support::{github_meta, link, ticket};

const TEMPLATE: &str = "dev-x";
const BLOCKED_KEY: &str = "GH-2";
const BLOCKER_KEY: &str = "GH-1";
const DEMAND_INPUTS: [&str; 2] = ["issue", "prompt"];
const SLOTS: u32 = 5;

/// A GitHub mirror blocked by the given links, optionally started by hand.
fn blocked_ticket(key: &str, blocked_by: Vec<BlockerLink>, manual: bool) -> Ticket {
    let number = key.trim_start_matches("GH-").parse().unwrap_or(0);
    Ticket {
        template: Some(TEMPLATE.to_string()),
        github: Some(github_meta(number, blocked_by)),
        autopilot: Autopilot {
            manual_priority: manual.then_some(true),
            ..Default::default()
        },
        ..ticket(key, Source::Github)
    }
}

fn catalog() -> TemplateCatalog {
    let mut catalog = TemplateCatalog::empty();
    catalog.insert(
        TEMPLATE,
        DEMAND_INPUTS.iter().map(|name| name.to_string()).collect(),
    );
    catalog
}

/// Plans the candidates against an unconstrained budget.
fn run_plan(candidates: &[Ticket]) -> DispatchPlan {
    let cfg = DispatcherConfig {
        slots: SLOTS,
        ..Default::default()
    };
    plan_with_blockers(
        candidates,
        &[],
        &Verdict::unconstrained(),
        &catalog(),
        &cfg,
        &HashMap::new(),
    )
}

/// An open blocker skips the ticket, which stays to-do.
#[test]
fn open_blocker_skips_with_waiting_on_and_ticket_stays_todo() {
    let candidates = vec![blocked_ticket(
        BLOCKED_KEY,
        vec![link(BLOCKER_KEY, IssueState::Open)],
        false,
    )];
    let result = run_plan(&candidates);
    assert!(result.launches.is_empty());
    assert_eq!(
        result.skipped,
        vec![(
            BLOCKED_KEY.to_string(),
            SkipReason::WaitingOn(vec![BLOCKER_KEY.to_string()])
        )]
    );
    assert_eq!(result.skipped[0].1.as_str(), "waiting-on:GH-1");
    assert_eq!(candidates[0].status, Status::Todo);
}

/// A closed blocker no longer blocks.
#[test]
fn closed_blocker_launches() {
    let result = run_plan(&[blocked_ticket(
        BLOCKED_KEY,
        vec![link(BLOCKER_KEY, IssueState::Closed)],
        false,
    )]);
    assert_eq!(result.launches.len(), 1);
}

/// Two tickets blocking each other still launch, flagged with the cycle detail.
#[test]
fn mutual_blockers_launch_with_blocker_cycle_detail() {
    let first = blocked_ticket(
        BLOCKER_KEY,
        vec![link(BLOCKED_KEY, IssueState::Open)],
        false,
    );
    let second = blocked_ticket(
        BLOCKED_KEY,
        vec![link(BLOCKER_KEY, IssueState::Open)],
        false,
    );
    let result = run_plan(&[first, second]);
    let launched = result
        .launches
        .iter()
        .find(|launch| launch.key == BLOCKER_KEY)
        .expect("first ticket launches");
    assert_eq!(launched.detail, Some(LaunchDetail::BlockerCycle));
}

/// A manually started ticket launches despite its blocker.
#[test]
fn blocked_ticket_with_manual_start_launches() {
    let result = run_plan(&[blocked_ticket(
        BLOCKED_KEY,
        vec![link(BLOCKER_KEY, IssueState::Open)],
        true,
    )]);
    assert_eq!(result.launches.len(), 1);
    assert!(result.launches[0].manual);
}

/// A GitHub mirror routed to an issue-mode template carries its issue number.
#[test]
fn github_mirror_launches_in_issue_mode() {
    let result = run_plan(&[blocked_ticket("GH-237", Vec::new(), false)]);
    assert_eq!(result.launches[0].demand, Demand::Issue { number: 237 });
    assert_eq!(result.launches[0].flow, None);
}

/// A ticket whose route names a flow launches that flow, with no template needed.
#[test]
fn flow_route_launches_without_template() {
    let mut routed = blocked_ticket("GH-12", Vec::new(), false);
    routed.template = None;
    routed.flow = Some("game-code".to_string());
    let launch = build_launch(&routed, &TemplateCatalog::empty()).expect("launches");
    assert_eq!(launch.flow.as_deref(), Some("game-code"));
    assert_eq!(launch.demand, Demand::Issue { number: 12 });
}

/// The waiting-on wire form joins the blocker keys with commas.
#[test]
fn waiting_on_wire_form_joins_keys_with_commas() {
    let reason = SkipReason::WaitingOn(vec!["A".to_string(), "B".to_string()]);
    assert_eq!(reason.as_str(), "waiting-on:A,B");
}
