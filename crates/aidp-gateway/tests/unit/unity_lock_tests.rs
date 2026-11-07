use super::*;

const EDITOR: &str = "unity-editor:p1";

fn holder(flow: &str) -> ResourceHolder {
    ResourceHolder {
        flow_key: flow.to_string(),
        ticket_key: format!("GH-{flow}"),
        step_id: "implement".to_string(),
        since: "2026-11-06T10:00:00.000Z".to_string(),
    }
}

#[test]
fn resource_key_joins_name_and_project() {
    assert_eq!(resource_key("unity-editor", "p1"), EDITOR);
}

#[test]
fn first_flow_holds_and_the_second_queues() {
    let manager = ResourceManager::default();

    let (first, change) = manager.acquire(EDITOR, holder("a"));
    let (second, queued) = manager.acquire(EDITOR, holder("b"));

    assert_eq!(first, Acquire::Granted);
    assert!(change.unwrap().held);
    assert_eq!(second, Acquire::Busy);
    assert_eq!(queued.unwrap().waiting, vec!["b".to_string()]);
    assert_eq!(manager.snapshot(EDITOR).0.unwrap().flow_key, "a");
}

#[test]
fn acquire_is_idempotent_for_the_holder_and_for_a_queued_flow() {
    let manager = ResourceManager::default();
    manager.acquire(EDITOR, holder("a"));
    manager.acquire(EDITOR, holder("b"));

    assert_eq!(
        manager.acquire(EDITOR, holder("a")),
        (Acquire::Granted, None)
    );
    assert_eq!(manager.acquire(EDITOR, holder("b")), (Acquire::Busy, None));
    assert_eq!(manager.snapshot(EDITOR).1, vec!["b".to_string()]);
}

#[test]
fn waiters_are_served_first_in_first_out() {
    let manager = ResourceManager::default();
    manager.acquire(EDITOR, holder("a"));
    manager.acquire(EDITOR, holder("b"));
    manager.acquire(EDITOR, holder("c"));

    let released = manager.sync("a", None, None);

    assert!(released[0].released);
    assert!(!manager.can_grant(EDITOR, "c"));
    assert!(manager.can_grant(EDITOR, "b"));
    assert_eq!(manager.acquire(EDITOR, holder("c")).0, Acquire::Busy);
    assert_eq!(manager.acquire(EDITOR, holder("b")).0, Acquire::Granted);
    assert_eq!(manager.snapshot(EDITOR).1, vec!["c".to_string()]);
}

#[test]
fn a_newcomer_does_not_overtake_the_queue_once_the_resource_is_free() {
    let manager = ResourceManager::default();
    manager.acquire(EDITOR, holder("a"));
    manager.acquire(EDITOR, holder("b"));
    manager.sync("a", None, None);

    assert_eq!(manager.acquire(EDITOR, holder("z")).0, Acquire::Busy);
}

#[test]
fn sync_drops_a_waiter_that_no_longer_waits() {
    let manager = ResourceManager::default();
    manager.acquire(EDITOR, holder("a"));
    manager.acquire(EDITOR, holder("b"));

    let changes = manager.sync("b", None, None);

    assert_eq!(changes.len(), 1);
    assert!(manager.snapshot(EDITOR).1.is_empty());
    assert!(manager.sync("b", None, None).is_empty());
}

#[test]
fn sync_keeps_a_holder_that_still_holds() {
    let manager = ResourceManager::default();
    manager.acquire(EDITOR, holder("a"));

    assert!(manager.sync("a", Some(EDITOR), None).is_empty());
    assert!(manager.snapshot(EDITOR).0.is_some());
}

#[test]
fn claim_never_queues_and_refuses_a_second_holder() {
    let manager = ResourceManager::default();

    assert!(manager.claim(EDITOR, holder("a")).0);
    assert!(!manager.claim(EDITOR, holder("b")).0);
    assert!(manager.snapshot(EDITOR).1.is_empty());
}

#[test]
fn force_release_frees_the_holder_and_blocks_a_reclaim_until_it_stops_holding() {
    let manager = ResourceManager::default();
    manager.acquire(EDITOR, holder("a"));

    let (former, change) = manager.force_release(EDITOR).unwrap();

    assert_eq!(former.flow_key, "a");
    assert!(!change.held && change.released);
    assert!(!manager.claim(EDITOR, holder("a")).0);
    manager.sync("a", None, None);
    assert!(manager.claim(EDITOR, holder("a")).0);
    assert!(manager.force_release("unknown").is_none());
}

#[test]
fn resources_are_independent_per_project() {
    let manager = ResourceManager::default();
    manager.acquire("unity-editor:p1", holder("a"));

    assert_eq!(
        manager.acquire("unity-editor:p2", holder("b")).0,
        Acquire::Granted
    );
}

#[test]
fn change_serializes_in_camel_case_without_the_internal_flag() {
    let manager = ResourceManager::default();
    let (_, change) = manager.acquire(EDITOR, holder("a"));

    let value = serde_json::to_value(change.unwrap()).unwrap();

    assert_eq!(value["resource"], EDITOR);
    assert_eq!(value["held"], true);
    assert_eq!(value["holder"]["flowKey"], "a");
    assert_eq!(value["holder"]["ticketKey"], "GH-a");
    assert!(value.get("released").is_none());
}
