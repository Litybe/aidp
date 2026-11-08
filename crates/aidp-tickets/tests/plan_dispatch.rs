//! Integration test: `aidp_tickets::plan` exercised end to end through the
//! crate's public API only (no `#[cfg(test)]` access to `plan.rs`'s
//! internals). One `#[test]` per rule in `plan`'s doc comment: launch
//! ordering (GitHub-before-local, priority, FIFO, manual priority), the
//! project/slot/budget/pause guards, the routing/template skip reasons, the
//! demand-mode choice per template, and `TemplateCatalog::from_skill_root`'s
//! tolerant YAML read (including the falsy-default sentinel).

use std::fs;

use chrono::{DateTime, Duration, Utc};

use aidp_testkit::TempWorkspace;
use aidp_tickets::config::DispatcherConfig;
use aidp_tickets::config::DEFAULT_MAX_WORKTREES_PER_PROJECT;
use aidp_tickets::dispatcher_state::worktree_cap_warning;
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::models::enums::issue_state::IssueState;
use aidp_tickets::models::github_meta::GithubMeta;
use aidp_tickets::plan::{
    cap_verdict, plan, plan_with_worktrees, CapVerdict, Demand, RunningSlot, SkipReason,
    TemplateCatalog, Verdict,
};
use std::collections::HashMap;

/// Build a `Ticket`, modelled on the `ticket()` helper in `src/order.rs`'s
/// test module, extended with `template` and `service` so it can drive
/// `plan` (which `order::sort_key` alone does not need).
fn ticket(
    key: &str,
    source: Source,
    priority: Priority,
    service: &str,
    template: Option<&str>,
    created: DateTime<Utc>,
) -> Ticket {
    Ticket {
        key: key.to_string(),
        source,
        r#type: IssueType::Task,
        summary: key.to_string(),
        status: Status::Todo,
        priority,
        priority_rank: priority.rank(),
        service: service.to_string(),
        template: template.map(|t| t.to_string()),
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created,
        updated: created,
        github: (source == Source::Github).then(|| GithubMeta {
            repo: "o/r".to_string(),
            number: key
                .rsplit('-')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0),
            url: String::new(),
            state: IssueState::Open,
            synced_at: None,
            blocked_by: Vec::new(),
        }),
        route: None,
        flow: None,
        milestone: None,
        autopilot: Autopilot::default(),
        tags: Vec::new(),
    }
}

fn cfg(slots: u32, paused: bool) -> DispatcherConfig {
    DispatcherConfig {
        slots,
        paused,
        ..Default::default()
    }
}

fn running_slot(project: &str, origin_kind: &str, ticket_key: Option<&str>) -> RunningSlot {
    RunningSlot {
        project: project.to_string(),
        origin_kind: origin_kind.to_string(),
        ticket_key: ticket_key.map(|k| k.to_string()),
        started_at_ms: 0,
        main_checkout: true,
    }
}

/// A catalog with one template declaring both `issue` and `prompt` —
/// the shape the ordering/guard tests need without caring about demand
/// choice, which `demand_follows_the_template_inputs` covers on its own.
fn dual_mode_catalog(template: &str) -> TemplateCatalog {
    let mut catalog = TemplateCatalog::empty();
    catalog.insert(template, vec!["issue".to_string(), "prompt".to_string()]);
    catalog
}

#[test]
fn github_before_local() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let candidates = vec![
        ticket(
            "LOC-1",
            Source::Local,
            Priority::Medium,
            "svc-b",
            Some("dev-x"),
            base,
        ),
        ticket(
            "PIXEL-1",
            Source::Github,
            Priority::Medium,
            "svc-a",
            Some("dev-x"),
            base,
        ),
    ];

    let result = plan(
        &candidates,
        &[],
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, false),
    );

    assert_eq!(
        result.launches.len(),
        2,
        "both tickets are eligible: {result:?}"
    );
    assert_eq!(
        result.launches[0].key, "PIXEL-1",
        "the GitHub mirror must launch before the local ticket"
    );
    assert_eq!(result.launches[1].key, "LOC-1");
}

#[test]
fn priority_orders_launches() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let candidates = vec![
        ticket(
            "PIXEL-1",
            Source::Github,
            Priority::Medium,
            "svc-b",
            Some("dev-x"),
            base,
        ),
        ticket(
            "PIXEL-2",
            Source::Github,
            Priority::Highest,
            "svc-a",
            Some("dev-x"),
            base,
        ),
    ];

    let result = plan(
        &candidates,
        &[],
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, false),
    );

    assert_eq!(result.launches.len(), 2);
    assert_eq!(
        result.launches[0].key, "PIXEL-2",
        "Highest must launch before Medium"
    );
    assert_eq!(result.launches[1].key, "PIXEL-1");
}

#[test]
fn fifo_breaks_equal_priority() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let candidates = vec![
        ticket(
            "PIXEL-2",
            Source::Github,
            Priority::Medium,
            "svc-b",
            Some("dev-x"),
            base + Duration::seconds(20),
        ),
        ticket(
            "PIXEL-1",
            Source::Github,
            Priority::Medium,
            "svc-a",
            Some("dev-x"),
            base,
        ),
    ];

    let result = plan(
        &candidates,
        &[],
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, false),
    );

    assert_eq!(result.launches.len(), 2);
    assert_eq!(
        result.launches[0].key, "PIXEL-1",
        "equal source and priority: the earlier `created` launches first"
    );
    assert_eq!(result.launches[1].key, "PIXEL-2");
}

#[test]
fn manual_priority_goes_first() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let mut manual = ticket(
        "LOC-1",
        Source::Local,
        Priority::Medium,
        "svc-b",
        Some("dev-x"),
        base,
    );
    manual.autopilot.manual_priority = Some(true);
    let candidates = vec![
        ticket(
            "PIXEL-1",
            Source::Github,
            Priority::Highest,
            "svc-a",
            Some("dev-x"),
            base - Duration::seconds(10),
        ),
        manual,
    ];

    let result = plan(
        &candidates,
        &[],
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, false),
    );

    assert_eq!(result.launches.len(), 2);
    assert_eq!(
        result.launches[0].key, "LOC-1",
        "manual_priority must jump a Medium local ticket ahead of a Highest GitHub one"
    );
    assert!(
        result.launches[0].manual,
        "the launch must record manual: true"
    );
    assert_eq!(result.launches[1].key, "PIXEL-1");
}

#[test]
fn one_ticket_per_project_per_tick() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let candidates = vec![
        ticket(
            "PIXEL-1",
            Source::Github,
            Priority::Highest,
            "svc-a",
            Some("dev-x"),
            base,
        ),
        ticket(
            "PIXEL-2",
            Source::Github,
            Priority::Medium,
            "svc-a",
            Some("dev-x"),
            base,
        ),
    ];

    let result = plan(
        &candidates,
        &[],
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, false),
    );

    assert_eq!(
        result.launches.len(),
        1,
        "only one launch per project: {result:?}"
    );
    assert_eq!(result.launches[0].key, "PIXEL-1");
    assert_eq!(
        result.skipped,
        vec![("PIXEL-2".to_string(), SkipReason::ProjectBusy)]
    );
}

#[test]
fn ui_session_occupies_the_project() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let candidates = vec![ticket(
        "PIXEL-1",
        Source::Github,
        Priority::Highest,
        "svc-a",
        Some("dev-x"),
        base,
    )];
    let running = vec![running_slot("svc-a", "ui", None)];

    let result = plan(
        &candidates,
        &running,
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, false),
    );

    assert!(
        result.launches.is_empty(),
        "the UI session must occupy svc-a: {result:?}"
    );
    assert_eq!(
        result.skipped,
        vec![("PIXEL-1".to_string(), SkipReason::ProjectBusy)]
    );
    assert_eq!(
        result.slots_used, 0,
        "a UI-origin session occupies a project but must not consume a dispatcher slot (vision D4)"
    );
}

#[test]
fn slot_ceiling_caps_launches() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let running = vec![
        running_slot("svc-a", "dispatcher", Some("PIXEL-90")),
        running_slot("svc-b", "dispatcher", Some("PIXEL-91")),
        running_slot("svc-c", "dispatcher", Some("PIXEL-92")),
    ];
    // Five eligible tickets on five OTHER, free services.
    let candidates = vec![
        ticket(
            "PIXEL-1",
            Source::Github,
            Priority::Highest,
            "svc-d",
            Some("dev-x"),
            base,
        ),
        ticket(
            "PIXEL-2",
            Source::Github,
            Priority::Highest,
            "svc-e",
            Some("dev-x"),
            base + Duration::seconds(1),
        ),
        ticket(
            "PIXEL-3",
            Source::Github,
            Priority::Highest,
            "svc-f",
            Some("dev-x"),
            base + Duration::seconds(2),
        ),
        ticket(
            "PIXEL-4",
            Source::Github,
            Priority::Highest,
            "svc-g",
            Some("dev-x"),
            base + Duration::seconds(3),
        ),
        ticket(
            "PIXEL-5",
            Source::Github,
            Priority::Highest,
            "svc-h",
            Some("dev-x"),
            base + Duration::seconds(4),
        ),
    ];

    let result = plan(
        &candidates,
        &running,
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, false),
    );

    assert_eq!(
        result.slots_used, 3,
        "three dispatcher-origin sessions are running"
    );
    assert_eq!(result.slots_max, 5);
    assert_eq!(
        result.launches.len(),
        2,
        "only 5 - 3 = 2 free slots remain: {result:?}"
    );
    let skipped_reasons: Vec<&SkipReason> = result.skipped.iter().map(|(_, r)| r).collect();
    assert_eq!(result.skipped.len(), 3);
    assert!(
        skipped_reasons.iter().all(|r| **r == SkipReason::NoSlot),
        "the remaining three must be skipped no-slot: {:?}",
        result.skipped
    );
}

#[test]
fn paused_skips_everything() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let candidates = vec![
        ticket(
            "PIXEL-1",
            Source::Github,
            Priority::Highest,
            "svc-a",
            Some("dev-x"),
            base,
        ),
        ticket(
            "PIXEL-2",
            Source::Github,
            Priority::Medium,
            "svc-b",
            Some("dev-x"),
            base,
        ),
    ];

    let result = plan(
        &candidates,
        &[],
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, true),
    );

    assert!(result.launches.is_empty());
    for (_, reason) in &result.skipped {
        assert_eq!(reason.as_str(), "paused");
    }
    assert_eq!(result.skipped.len(), 2);
}

#[test]
fn unrouted_is_needs_routing() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");

    let unrouted = ticket(
        "PIXEL-1",
        Source::Github,
        Priority::Highest,
        "unrouted",
        Some("dev-x"),
        base,
    );
    let no_template = ticket(
        "PIXEL-2",
        Source::Github,
        Priority::Highest,
        "svc-a",
        None,
        base,
    );

    let result = plan(
        &[unrouted, no_template],
        &[],
        &Verdict::unconstrained(),
        &catalog,
        &cfg(5, false),
    );

    assert!(result.launches.is_empty());
    let reasons: Vec<String> = result.skipped.iter().map(|(_, r)| r.as_str()).collect();
    assert!(reasons.contains(&"needs-routing".to_string()));
    assert!(reasons.contains(&"no-template".to_string()));
    assert_eq!(
        result
            .skipped
            .iter()
            .find(|(k, _)| k == "PIXEL-1")
            .map(|(_, r)| r.as_str()),
        Some("needs-routing".to_string())
    );
    assert_eq!(
        result
            .skipped
            .iter()
            .find(|(k, _)| k == "PIXEL-2")
            .map(|(_, r)| r.as_str()),
        Some("no-template".to_string())
    );
}

#[test]
fn demand_follows_the_template_inputs() {
    let base = Utc::now();

    // An issue-only template.
    let mut issue_only = TemplateCatalog::empty();
    issue_only.insert("dev-lambda-ts", vec!["issue".to_string()]);

    let github_mirror = ticket(
        "PIXEL-1",
        Source::Github,
        Priority::Highest,
        "svc-a",
        Some("dev-lambda-ts"),
        base,
    );
    let result = plan(
        &[github_mirror],
        &[],
        &Verdict::unconstrained(),
        &issue_only,
        &cfg(5, false),
    );
    assert_eq!(result.launches.len(), 1);
    assert_eq!(result.launches[0].demand, Demand::Issue { number: 1 });

    let local_on_issue_only = ticket(
        "LOC-1",
        Source::Local,
        Priority::Highest,
        "svc-a",
        Some("dev-lambda-ts"),
        base,
    );
    let result = plan(
        &[local_on_issue_only],
        &[],
        &Verdict::unconstrained(),
        &issue_only,
        &cfg(5, false),
    );
    assert!(
        result.launches.is_empty(),
        "a local ticket on an issue-only template must not launch: {result:?}"
    );
    assert_eq!(
        result.skipped,
        vec![("LOC-1".to_string(), SkipReason::TemplateWithoutDemandMode)]
    );

    // A dual-mode template: a GitHub mirror gets Demand::Issue, local ticket gets
    // Demand::Prompt with feature == key.
    let dual = dual_mode_catalog("dev-dual");
    let github_mirror = ticket(
        "PIXEL-2",
        Source::Github,
        Priority::Highest,
        "svc-b",
        Some("dev-dual"),
        base,
    );
    let result = plan(
        &[github_mirror],
        &[],
        &Verdict::unconstrained(),
        &dual,
        &cfg(5, false),
    );
    assert_eq!(result.launches[0].demand, Demand::Issue { number: 2 });

    let local = ticket(
        "LOC-2",
        Source::Local,
        Priority::Highest,
        "svc-b",
        Some("dev-dual"),
        base,
    );
    let result = plan(
        &[local],
        &[],
        &Verdict::unconstrained(),
        &dual,
        &cfg(5, false),
    );
    match &result.launches[0].demand {
        Demand::Prompt { feature, prompt } => {
            assert_eq!(feature, "LOC-2");
            assert_eq!(
                prompt, "",
                "plan fills prompt with an empty-string placeholder"
            );
        }
        other => panic!("expected Demand::Prompt, got {other:?}"),
    }

    // A prompt-only template: even a GitHub mirror gets Demand::Prompt.
    let mut prompt_only = TemplateCatalog::empty();
    prompt_only.insert("dev-discovery", vec!["prompt".to_string()]);
    let github_mirror = ticket(
        "PIXEL-3",
        Source::Github,
        Priority::Highest,
        "svc-c",
        Some("dev-discovery"),
        base,
    );
    let result = plan(
        &[github_mirror],
        &[],
        &Verdict::unconstrained(),
        &prompt_only,
        &cfg(5, false),
    );
    match &result.launches[0].demand {
        Demand::Prompt { feature, .. } => assert_eq!(feature, "PIXEL-3"),
        other => panic!("expected Demand::Prompt even for a GitHub mirror, got {other:?}"),
    }
}

#[test]
fn budget_zero_blocks_every_launch() {
    let base = Utc::now();
    let catalog = dual_mode_catalog("dev-x");
    let candidates = vec![ticket(
        "PIXEL-1",
        Source::Github,
        Priority::Highest,
        "svc-a",
        Some("dev-x"),
        base,
    )];

    let result = plan(
        &candidates,
        &[],
        &Verdict::blocked("5h window at 92%"),
        &catalog,
        &cfg(5, false),
    );

    assert!(result.launches.is_empty());
    assert_eq!(
        result.skipped,
        vec![(
            "PIXEL-1".to_string(),
            SkipReason::Budget("5h window at 92%".to_string())
        )]
    );
    assert_eq!(result.skipped[0].1.as_str(), "budget:5h window at 92%");
}

#[test]
fn template_catalog_reads_only_input_keys() {
    let _workspace = TempWorkspace::new("plan-dispatch-catalog");
    let root = _workspace.path();
    let templates_dir = root.join("templates");
    fs::create_dir_all(&templates_dir).expect("create templates/");

    fs::write(
        templates_dir.join("dev-discovery.yaml"),
        r#"id: dev-discovery
inputs:
  jira_keys:
    required: false
  prompt:
    required: false
    default: false
"#,
    )
    .expect("write dev-discovery.yaml");

    fs::write(
        templates_dir.join("dev-broken.yaml"),
        "id: dev-broken\ninputs: [this, is, not, a, mapping\n",
    )
    .expect("write dev-broken.yaml");

    let catalog = TemplateCatalog::from_skill_root(root);

    assert!(
        catalog.known("dev-discovery"),
        "the valid template must be read despite the broken sibling"
    );
    assert!(
        catalog.declares("dev-discovery", "jira_keys"),
        "jira_keys must be recorded"
    );
    assert!(
        catalog.declares("dev-discovery", "prompt"),
        "prompt must be recorded even though its default is the falsy sentinel `false` — \
         templates/dev-discovery.yaml relies on the key's PRESENCE, not its default, to mean \
         'this demand mode is available'"
    );
    assert!(
        !catalog.known("dev-broken"),
        "an invalid YAML file must be skipped, not fail the whole read"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn skipped_entry_without_detail_still_deserializes_and_round_trips_with_it() {
    use aidp_tickets::dispatcher_state::SkippedEntry;

    let legacy = serde_json::json!({
        "key": "LOC-2",
        "reason": "project-busy",
    });
    let entry: SkippedEntry = serde_json::from_value(legacy).expect("legacy entry deserializes");
    assert_eq!(entry.key, "LOC-2");
    assert_eq!(entry.reason, "project-busy");
    assert_eq!(entry.detail, None);

    let with_detail = SkippedEntry {
        key: "LOC-3".to_string(),
        reason: "budget-wait".to_string(),
        detail: Some("waiting on the 5h window".to_string()),
    };
    let value = serde_json::to_value(&with_detail).expect("serializes");
    let round_tripped: SkippedEntry =
        serde_json::from_value(value).expect("round trip deserializes");
    assert_eq!(round_tripped, with_detail);
}

fn keys(items: &[&str]) -> Vec<String> {
    items.iter().map(|k| k.to_string()).collect()
}

fn holders_for(project: &str, items: &[&str]) -> HashMap<String, Vec<String>> {
    HashMap::from([(project.to_string(), keys(items))])
}

fn local_ticket(key: &str, service: &str) -> Ticket {
    ticket(
        key,
        Source::Local,
        Priority::Medium,
        service,
        Some("dev-aidp"),
        Utc::now(),
    )
}

fn prompt_catalog() -> TemplateCatalog {
    let mut catalog = TemplateCatalog::empty();
    catalog.insert("dev-aidp", vec!["prompt".to_string()]);
    catalog
}

#[test]
fn fourth_ticket_of_a_full_project_is_skipped_with_the_holders() {
    let holders = holders_for("svc", &["A", "B", "C"]);
    let result = plan_with_worktrees(
        &[local_ticket("D", "svc")],
        &[],
        &Verdict::unconstrained(),
        &prompt_catalog(),
        &cfg(5, false),
        &holders,
    );

    assert!(result.launches.is_empty());
    let (key, reason) = &result.skipped[0];
    assert_eq!(key, "D");
    assert_eq!(reason.as_str(), "worktree-cap");
    assert_eq!(reason.detail().as_deref(), Some("A, B, C"));
}

#[test]
fn ticket_owning_a_worktree_is_never_capped() {
    let holders = holders_for("svc", &["A", "B", "C"]);
    let result = plan_with_worktrees(
        &[local_ticket("B", "svc")],
        &[],
        &Verdict::unconstrained(),
        &prompt_catalog(),
        &cfg(5, false),
        &holders,
    );

    assert_eq!(result.launches.len(), 1);
    assert!(result.skipped.is_empty());
}

#[test]
fn another_project_is_unaffected_by_the_cap() {
    let holders = holders_for("svc", &["A", "B", "C"]);
    let result = plan_with_worktrees(
        &[local_ticket("D", "other")],
        &[],
        &Verdict::unconstrained(),
        &prompt_catalog(),
        &cfg(5, false),
        &holders,
    );

    assert_eq!(result.launches.len(), 1);
}

#[test]
fn plan_without_holders_never_caps() {
    let result = plan(
        &[local_ticket("D", "svc")],
        &[],
        &Verdict::unconstrained(),
        &prompt_catalog(),
        &cfg(5, false),
    );

    assert_eq!(result.launches.len(), 1);
}

#[test]
fn config_without_the_new_keys_gets_defaults() {
    let parsed: DispatcherConfig = serde_yaml::from_str(
        "slots: 7
",
    )
    .expect("parses");

    assert_eq!(
        parsed.max_worktrees_per_project,
        DEFAULT_MAX_WORKTREES_PER_PROJECT
    );
    assert_eq!(
        parsed.worktree_copy,
        keys(&[".claude/settings.local.json", ".aidp/*.json"])
    );
}

#[test]
fn cap_verdict_table() {
    let alive = keys(&["A", "B", "C"]);
    assert_eq!(
        cap_verdict(&alive, 3, false),
        CapVerdict::Cap(alive.clone())
    );
    assert_eq!(cap_verdict(&alive, 3, true), CapVerdict::Allow);
    assert_eq!(cap_verdict(&alive, 4, false), CapVerdict::Allow);
    assert_eq!(cap_verdict(&[], 3, false), CapVerdict::Allow);
}

#[test]
fn branch_checked_out_elsewhere_has_its_slug() {
    assert_eq!(
        SkipReason::BranchCheckedOutElsewhere.as_str(),
        "branch-checked-out-elsewhere"
    );
}

#[test]
fn worktree_cap_warning_format() {
    assert_eq!(
        worktree_cap_warning(3, &keys(&["A", "B", "C"])),
        "worktree-cap: 3 already open (A, B, C)"
    );
}
