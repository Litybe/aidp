//! The two lanes of the dispatcher plan: dev flows bounded by `slots`, test flows by `test_slots`
//! per project, `to-test` tickets taken oldest first with a manual promotion ahead.

use std::collections::HashMap;

use chrono::{Duration, Utc};

use aidp_tickets::config::{DispatcherConfig, RepoRoute, Routing};
use aidp_tickets::model::{Autopilot, IssueType, Phase, Priority, Source, Status, Ticket};
use aidp_tickets::models::constants::lane::BUG_TAG;
use aidp_tickets::plan::{plan_with_lanes, LaneLoad, SkipReason, TemplateCatalog, Verdict};

fn ticket(key: &str, phase: Phase, status: Status, age_minutes: i64) -> Ticket {
    let now = Utc::now();
    let mut autopilot = Autopilot::default();
    autopilot.set_phase(phase, now - Duration::minutes(age_minutes));
    Ticket {
        key: key.to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: key.to_string(),
        status,
        priority: Priority::Medium,
        priority_rank: Priority::Medium.rank(),
        service: "game".to_string(),
        template: None,
        track: None,
        route: Some("code".to_string()),
        flow: Some("game-code".to_string()),
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        milestone: None,
        created: now - Duration::minutes(age_minutes),
        updated: now,
        github: None,
        autopilot,
        tags: Vec::new(),
    }
}

fn routing() -> Routing {
    let mut routing = Routing::default();
    routing.repos.insert(
        "game-code".to_string(),
        RepoRoute {
            flow: Some("game-code".to_string()),
            route: Some("code".to_string()),
            test_flow: Some("game-test-code".to_string()),
            ..RepoRoute::default()
        },
    );
    routing
}

fn config(slots: u32, test_slots: u32) -> DispatcherConfig {
    DispatcherConfig {
        slots,
        test_slots,
        ..DispatcherConfig::default()
    }
}

fn plan(
    tickets: &[Ticket],
    cfg: &DispatcherConfig,
    dev_flows: usize,
    test_flows: &[(&str, usize)],
) -> aidp_tickets::plan::DispatchPlan {
    let routing = routing();
    plan_with_lanes(
        tickets,
        &[],
        &Verdict::unconstrained(),
        &TemplateCatalog::empty(),
        cfg,
        &HashMap::new(),
        &LaneLoad {
            routing: Some(&routing),
            dev_flows,
            test_flows: test_flows
                .iter()
                .map(|(project, count)| (project.to_string(), *count))
                .collect(),
            ..LaneLoad::default()
        },
    )
}

fn bug(key: &str, phase: Phase, status: Status, age_minutes: i64) -> Ticket {
    let mut bug = ticket(key, phase, status, age_minutes);
    bug.r#type = IssueType::Bug;
    bug
}

#[test]
fn a_bug_launches_beside_a_full_dev_lane() {
    let tickets = [
        ticket("D-1", Phase::Dev, Status::Todo, 5),
        bug("B-1", Phase::Dev, Status::Todo, 1),
    ];
    let result = plan(&tickets, &config(1, 1), 1, &[]);
    let launched: Vec<&str> = result.launches.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(launched, vec!["B-1"]);
    assert_eq!(result.slots_used, 1);
    assert_eq!(result.bug_slots_max, 1);
}

#[test]
fn a_bug_does_not_take_a_dev_slot() {
    let tickets = [
        bug("B-1", Phase::Dev, Status::Todo, 5),
        ticket("D-1", Phase::Dev, Status::Todo, 1),
    ];
    let result = plan(&tickets, &config(1, 1), 0, &[]);
    assert_eq!(result.launches.len(), 2, "{:?}", result.skipped);
}

#[test]
fn the_bug_lane_is_bounded_by_bug_slots() {
    let tickets = [
        bug("B-1", Phase::Dev, Status::Todo, 5),
        bug("B-2", Phase::Dev, Status::Todo, 1),
    ];
    let cfg = DispatcherConfig {
        bug_slots: 1,
        ..config(3, 1)
    };
    let result = plan(&tickets, &cfg, 0, &[]);
    assert_eq!(result.launches.len(), 1);
    assert!(result
        .skipped
        .contains(&("B-2".to_string(), SkipReason::NoSlot)));
}

#[test]
fn running_bug_flows_use_the_bug_slots_only() {
    let tickets = [
        bug("B-1", Phase::Dev, Status::Todo, 5),
        ticket("D-1", Phase::Dev, Status::Todo, 1),
    ];
    let routing = routing();
    let result = plan_with_lanes(
        &tickets,
        &[],
        &Verdict::unconstrained(),
        &TemplateCatalog::empty(),
        &config(3, 1),
        &HashMap::new(),
        &LaneLoad {
            routing: Some(&routing),
            bug_flows: 1,
            ..LaneLoad::default()
        },
    );
    let launched: Vec<&str> = result.launches.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(launched, vec!["D-1"]);
    assert_eq!(result.bug_slots_used, 1);
    assert_eq!(result.slots_used, 0);
}

#[test]
fn a_bug_is_tested_before_older_tickets() {
    let tickets = [
        ticket("T-old", Phase::ToTest, Status::InProgress, 30),
        bug("B-new", Phase::ToTest, Status::InProgress, 1),
    ];
    let result = plan(&tickets, &config(3, 1), 0, &[]);
    assert_eq!(result.launches.len(), 1);
    assert_eq!(result.launches[0].key, "B-new");
}

#[test]
fn a_manual_promotion_still_passes_a_bug_in_the_test_lane() {
    let mut promoted = ticket("T-man", Phase::ToTest, Status::InProgress, 1);
    promoted.autopilot.manual_priority = Some(true);
    let tickets = [bug("B-1", Phase::ToTest, Status::InProgress, 30), promoted];
    let result = plan(&tickets, &config(3, 1), 0, &[]);
    assert_eq!(result.launches[0].key, "T-man");
}

#[test]
fn a_bug_ticket_carries_the_bug_tag() {
    let mut tagged = bug("B-1", Phase::Dev, Status::Todo, 1);
    tagged.recompute_derived();
    assert!(tagged.tags.iter().any(|tag| tag == BUG_TAG));
    let mut plain = ticket("D-1", Phase::Dev, Status::Todo, 1);
    plain.recompute_derived();
    assert!(!plain.tags.iter().any(|tag| tag == BUG_TAG));
}

#[test]
fn a_to_test_ticket_launches_its_test_flow_and_uses_no_dev_slot() {
    let tickets = [ticket("T-1", Phase::ToTest, Status::InProgress, 5)];
    let result = plan(&tickets, &config(1, 1), 1, &[]);
    assert_eq!(result.launches.len(), 1);
    let launch = &result.launches[0];
    assert!(launch.test);
    assert_eq!(launch.flow.as_deref(), Some("game-test-code"));
    assert_eq!(result.slots_used, 1);
    assert_eq!(result.slots_max, 1);
}

#[test]
fn two_to_test_tickets_go_one_after_the_other_oldest_first() {
    let tickets = [
        ticket("T-new", Phase::ToTest, Status::InProgress, 1),
        ticket("T-old", Phase::ToTest, Status::InProgress, 30),
    ];
    let first = plan(&tickets, &config(3, 1), 0, &[]);
    assert_eq!(first.launches.len(), 1);
    assert_eq!(first.launches[0].key, "T-old");
    assert!(first
        .skipped
        .contains(&("T-new".to_string(), SkipReason::NoTestSlot)));

    let second = plan(&tickets[..1], &config(3, 1), 0, &[("game", 1)]);
    assert!(second.launches.is_empty());
    assert_eq!(second.test_slots_used, 1);
}

#[test]
fn a_manual_promotion_goes_first_in_the_test_lane() {
    let mut promoted = ticket("T-new", Phase::ToTest, Status::InProgress, 1);
    promoted.autopilot.manual_priority = Some(true);
    let tickets = [
        ticket("T-old", Phase::ToTest, Status::InProgress, 30),
        promoted,
    ];
    let result = plan(&tickets, &config(3, 1), 0, &[]);
    assert_eq!(result.launches[0].key, "T-new");
}

#[test]
fn three_dev_tickets_launch_in_parallel_while_a_test_runs() {
    let tickets = [
        ticket("D-1", Phase::Dev, Status::Todo, 3),
        ticket("D-2", Phase::Dev, Status::Todo, 2),
        ticket("D-3", Phase::Dev, Status::Todo, 1),
        ticket("D-4", Phase::Dev, Status::Todo, 0),
    ];
    let result = plan(&tickets, &config(3, 1), 0, &[("game", 1)]);
    let launched: Vec<&str> = result.launches.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(launched.len(), 3, "{launched:?}");
    assert!(result.launches.iter().all(|launch| !launch.test));
    assert!(result
        .skipped
        .iter()
        .any(|(key, reason)| key == "D-4" && *reason == SkipReason::NoSlot));
    assert_eq!(result.test_slots_used, 1);
}

#[test]
fn running_dev_flows_use_the_dev_slots() {
    let tickets = [ticket("D-1", Phase::Dev, Status::Todo, 1)];
    let result = plan(&tickets, &config(3, 1), 3, &[]);
    assert!(result.launches.is_empty());
    assert_eq!(result.slots_used, 3);
}

#[test]
fn a_to_test_ticket_without_a_test_flow_is_skipped() {
    let tickets = [ticket("T-1", Phase::ToTest, Status::InProgress, 5)];
    let result = plan_with_lanes(
        &tickets,
        &[],
        &Verdict::unconstrained(),
        &TemplateCatalog::empty(),
        &config(3, 1),
        &HashMap::new(),
        &LaneLoad::default(),
    );
    assert!(result.launches.is_empty());
    assert_eq!(result.skipped.len(), 1);
}

#[test]
fn a_to_test_ticket_in_progress_is_never_a_dev_launch() {
    let tickets = [ticket("T-1", Phase::ToTest, Status::Todo, 5)];
    let result = plan(&tickets, &config(3, 1), 0, &[]);
    assert_eq!(result.launches.len(), 1);
    assert!(result.launches[0].test);
}

fn slot(origin_kind: &str, main_checkout: bool) -> aidp_tickets::plan::RunningSlot {
    aidp_tickets::plan::RunningSlot {
        project: "game".to_string(),
        origin_kind: origin_kind.to_string(),
        ticket_key: None,
        started_at_ms: 0,
        main_checkout,
        test_checkout: false,
    }
}

fn plan_running(
    tickets: &[Ticket],
    running: &[aidp_tickets::plan::RunningSlot],
) -> aidp_tickets::plan::DispatchPlan {
    let routing = routing();
    plan_with_lanes(
        tickets,
        running,
        &Verdict::unconstrained(),
        &TemplateCatalog::empty(),
        &config(3, 1),
        &HashMap::new(),
        &LaneLoad {
            routing: Some(&routing),
            ..LaneLoad::default()
        },
    )
}

#[test]
fn a_run_session_in_a_worktree_does_not_block_the_test_lane() {
    let tickets = [ticket("T-1", Phase::ToTest, Status::InProgress, 5)];
    let result = plan_running(&tickets, &[slot("run", false)]);
    assert_eq!(result.launches.len(), 1, "{:?}", result.skipped);
}

fn test_checkout_slot(origin_kind: &str) -> aidp_tickets::plan::RunningSlot {
    aidp_tickets::plan::RunningSlot {
        main_checkout: false,
        test_checkout: true,
        ..slot(origin_kind, false)
    }
}

#[test]
fn a_session_in_the_test_checkout_blocks_the_test_lane_only() {
    let tickets = [ticket("T-1", Phase::ToTest, Status::InProgress, 5)];
    let result = plan_running(&tickets, &[test_checkout_slot("ui")]);
    assert!(result.launches.is_empty());
    assert!(result
        .skipped
        .contains(&("T-1".to_string(), SkipReason::ProjectBusy)));
    let flow = plan_running(&tickets, &[test_checkout_slot("flow")]);
    assert_eq!(flow.launches.len(), 1);
}

#[test]
fn a_session_in_the_main_checkout_does_not_block_the_test_lane() {
    let tickets = [ticket("T-1", Phase::ToTest, Status::InProgress, 5)];
    let result = plan_running(&tickets, &[slot("ui", true)]);
    assert_eq!(result.launches.len(), 1, "{:?}", result.skipped);
}

#[test]
fn no_session_blocks_a_dev_or_bug_flow_launch() {
    let tickets = [
        ticket("D-1", Phase::Dev, Status::Todo, 1),
        bug("B-1", Phase::Dev, Status::Todo, 1),
    ];
    for running in [
        slot("ui", true),
        slot("run", false),
        test_checkout_slot("ui"),
    ] {
        let result = plan_running(&tickets, &[running]);
        assert_eq!(result.launches.len(), 2, "{:?}", result.skipped);
    }
}

fn plan_with_holders(
    tickets: &[Ticket],
    holders: &[&str],
    bug_keys: &[&str],
    max: usize,
) -> aidp_tickets::plan::DispatchPlan {
    let routing = routing();
    let cfg = DispatcherConfig {
        max_worktrees_per_project: max,
        ..config(3, 1)
    };
    let holders = HashMap::from([(
        "game".to_string(),
        holders.iter().map(ToString::to_string).collect::<Vec<_>>(),
    )]);
    plan_with_lanes(
        tickets,
        &[],
        &Verdict::unconstrained(),
        &TemplateCatalog::empty(),
        &cfg,
        &holders,
        &LaneLoad {
            routing: Some(&routing),
            bug_keys: bug_keys.iter().map(ToString::to_string).collect(),
            ..LaneLoad::default()
        },
    )
}

#[test]
fn the_worktree_cap_is_counted_per_lane() {
    let tickets = [
        ticket("D-9", Phase::Dev, Status::Todo, 1),
        bug("B-9", Phase::Dev, Status::Todo, 1),
    ];
    // Two dev worktrees fill a cap of 2: the bug still launches, the dev ticket does not.
    let result = plan_with_holders(&tickets, &["D-1", "D-2"], &[], 2);
    let launched: Vec<&str> = result.launches.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(launched, vec!["B-9"], "{:?}", result.skipped);
    assert!(result
        .skipped
        .iter()
        .any(|(key, reason)| key == "D-9" && matches!(reason, SkipReason::WorktreeCap(_))));
    // Two bug worktrees fill the bug cap: the dev ticket launches.
    let result = plan_with_holders(&tickets, &["B-1", "B-2"], &["B-1", "B-2", "B-9"], 2);
    let launched: Vec<&str> = result.launches.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(launched, vec!["D-9"], "{:?}", result.skipped);
}

#[test]
fn main_checkout_ignores_case_separators_and_verbatim_prefix() {
    use aidp_tickets::plan::is_main_checkout;
    let project = r"D:\CLAUDE_UNITY\PixelHood";
    assert!(is_main_checkout(r"d:/claude_unity/pixelhood/", project));
    let verbatim = format!("{0}{0}?{0}{1}", '\\', project);
    assert!(is_main_checkout(&verbatim, project));
    assert!(!is_main_checkout(
        r"D:\AIDP\Vault\Pipelines\PixelHood\GH-259\.worktree",
        project
    ));
    assert!(!is_main_checkout("", ""));
}

#[test]
fn a_ticket_already_carrying_a_test_flow_still_launches_it() {
    let mut t = ticket("T-1", Phase::ToTest, Status::InProgress, 5);
    t.flow = Some("game-test-code".to_string());
    let result = plan(&[t], &config(3, 1), 0, &[]);
    assert_eq!(result.launches.len(), 1, "{:?}", result.skipped);
    assert_eq!(result.launches[0].flow.as_deref(), Some("game-test-code"));
}

fn waiting_for(bug: &str, state: aidp_tickets::models::enums::issue_state::IssueState) -> Ticket {
    use aidp_tickets::models::blocker_link::BlockerLink;
    use aidp_tickets::models::github_meta::GithubMeta;
    let mut parent = ticket("GH-272", Phase::ToTest, Status::InProgress, 5);
    parent.source = Source::Github;
    parent.github = Some(GithubMeta {
        repo: "o/r".to_string(),
        number: 272,
        url: String::new(),
        state: aidp_tickets::models::enums::issue_state::IssueState::Open,
        synced_at: None,
        blocked_by: vec![BlockerLink {
            key: bug.to_string(),
            state,
        }],
        base_branch: None,
        parent: None,
    });
    parent
}

#[test]
fn a_to_test_ticket_waits_while_its_bug_is_open() {
    use aidp_tickets::models::enums::issue_state::IssueState;
    let tickets = [waiting_for("GH-300", IssueState::Open)];
    let result = plan(&tickets, &config(1, 1), 0, &[]);
    assert!(result.launches.is_empty());
    assert_eq!(
        result.skipped,
        vec![(
            "GH-272".to_string(),
            SkipReason::WaitingOn(vec!["GH-300".to_string()])
        )]
    );
    assert_eq!(result.skipped[0].1.as_str(), "waiting-on:GH-300");
}

#[test]
fn a_to_test_ticket_goes_back_to_its_test_flow_once_the_bug_is_closed() {
    use aidp_tickets::models::enums::issue_state::IssueState;
    let tickets = [waiting_for("GH-300", IssueState::Closed)];
    let result = plan(&tickets, &config(1, 1), 0, &[]);
    assert_eq!(result.launches.len(), 1);
    assert_eq!(result.launches[0].flow.as_deref(), Some("game-test-code"));
    assert!(result.launches[0].test);
}

#[test]
fn a_paused_dispatcher_claims_no_new_ticket() {
    let mut cfg = config(3, 1);
    cfg.paused = true;
    let tickets = [
        ticket("D-1", Phase::Dev, Status::Todo, 5),
        bug("B-1", Phase::Dev, Status::Queued, 1),
    ];
    let result = plan(&tickets, &cfg, 0, &[]);
    assert!(result.launches.is_empty());
    assert_eq!(result.skipped.len(), 2);
    assert!(result
        .skipped
        .iter()
        .all(|(_, reason)| *reason == SkipReason::Paused));
}

#[test]
fn a_paused_dispatcher_still_starts_the_test_lane_of_a_started_ticket() {
    let mut cfg = config(3, 1);
    cfg.paused = true;
    let tickets = [
        ticket("T-1", Phase::ToTest, Status::InProgress, 5),
        ticket("D-1", Phase::Dev, Status::Todo, 5),
    ];
    let result = plan(&tickets, &cfg, 1, &[]);
    let launched: Vec<&str> = result.launches.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(launched, vec!["T-1"]);
    assert!(result.launches[0].test);
    assert!(result
        .skipped
        .iter()
        .any(|(key, reason)| key == "D-1" && *reason == SkipReason::Paused));
}
