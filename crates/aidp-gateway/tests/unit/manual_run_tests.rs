use super::*;
use aidp_testkit::TempWorkspace;
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};

const FLOW_YAML: &str = include_str!("../fixtures/game_flows/game-code.yaml");
const KEY: &str = "LOC-9";

fn skill_with_flow() -> TempWorkspace {
    let skill = TempWorkspace::new("manual-run-skill");
    std::fs::create_dir_all(skill.child("templates/flows")).unwrap();
    std::fs::write(skill.child("templates/flows/game-code.yaml"), FLOW_YAML).unwrap();
    skill
}

fn store_with_ticket(workspace: &TempWorkspace, autopilot: Autopilot) -> TicketStore {
    let now = chrono::Utc::now();
    let mut ticket = Ticket {
        key: KEY.to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "sample".to_string(),
        status: Status::InProgress,
        priority: Priority::Medium,
        priority_rank: 0,
        service: "svc".to_string(),
        template: None,
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created: now,
        updated: now,
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot,
        tags: Vec::new(),
    };
    let store = TicketStore::open(workspace.path()).unwrap();
    store
        .save(&mut ticket, "", &serde_yaml::Mapping::new())
        .unwrap();
    store
}

#[test]
fn check_flow_accepts_a_catalogue_flow_and_refuses_the_rest() {
    let skill = skill_with_flow();
    assert!(check_flow(Some(skill.path().to_path_buf()), "game-code").is_ok());
    let unknown = check_flow(Some(skill.path().to_path_buf()), "nope").unwrap_err();
    assert_eq!(unknown.to_string(), "unknown flow `nope`");
    assert!(check_flow(Some(skill.path().to_path_buf()), "../escape").is_err());
    assert!(check_flow(None, "game-code").is_err());
}

#[test]
fn attach_launch_adds_flow_key_and_run_id_only_for_the_launched_ticket() {
    let vault = TempWorkspace::new("manual-run-attach");
    let autopilot = Autopilot {
        flow: Some("game-code".to_string()),
        run_id: Some("run-1".to_string()),
        ..Autopilot::default()
    };
    let store = store_with_ticket(&vault, autopilot);

    let mut launched = json!({ "launched": [KEY], "skipped": [] });
    attach_launch(&store, KEY, &mut launched);
    assert_eq!(launched["flowKey"], KEY);
    assert_eq!(launched["runId"], "run-1");

    let mut other = json!({ "launched": ["OTHER-1"], "skipped": [] });
    attach_launch(&store, KEY, &mut other);
    assert!(other.get("flowKey").is_none() && other.get("runId").is_none());
}

#[test]
fn attach_launch_adds_nothing_when_the_ticket_has_neither() {
    let vault = TempWorkspace::new("manual-run-attach-none");
    let store = store_with_ticket(&vault, Autopilot::default());
    let mut launched = json!({ "launched": [KEY] });
    attach_launch(&store, KEY, &mut launched);
    assert_eq!(launched, json!({ "launched": [KEY] }));
}
