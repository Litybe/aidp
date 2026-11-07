use super::*;
use aidp_testkit::TempWorkspace;

fn temp_dir(tag: &str) -> TempWorkspace {
    TempWorkspace::new(&format!("queue-test-{tag}"))
}

fn entry(project_id: &str, priority: i32, queued_at: &str) -> QueuedLaunch {
    QueuedLaunch {
        id: format!("q-{project_id}-{queued_at}-{priority}"),
        project_id: project_id.to_string(),
        project_name: format!("{project_id} name"),
        template_id: Some("dev".to_string()),
        target: None,
        engine: "v7".to_string(),
        priority,
        status: "queued".to_string(),
        queued_at: queued_at.to_string(),
        origin: crate::sessions::Origin::new("api"),
        workspace_key: None,
        reason: None,
        request: json!({ "secret": "do-not-expose" }),
    }
}

fn entry_in(workspace_key: &str, queued_at: &str) -> QueuedLaunch {
    QueuedLaunch {
        id: format!("q-{workspace_key}-{queued_at}"),
        workspace_key: Some(workspace_key.to_string()),
        ..entry("p1", 0, queued_at)
    }
}

#[test]
fn admit_starts_when_the_project_is_idle() {
    assert_eq!(admit("p1", &[]), Admission::Start);
}

#[test]
fn admit_queues_when_the_project_is_busy() {
    let running = vec!["p1".to_string()];
    assert_eq!(admit("p1", &running), Admission::Queue);
}

#[test]
fn admit_starts_when_another_project_is_busy() {
    let running = vec!["p2".to_string()];
    assert_eq!(admit("p1", &running), Admission::Start);
}

#[test]
fn enqueue_then_list_returns_the_entry() {
    let _ws = temp_dir("enqueue");
    let dir = _ws.path();
    let queue = Queue::load(dir, Arc::new(Bus::new()));

    let e = entry("p1", 0, "2026-01-01T00:00:00.000Z");
    queue.enqueue(e.clone());

    let listed = queue.list(None);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, e.id);
    assert_eq!(queue.depth(), 1);
}

#[test]
fn list_is_in_dispatch_order() {
    let _ws = temp_dir("order");
    let dir = _ws.path();
    let queue = Queue::load(dir, Arc::new(Bus::new()));

    let low = entry("p1", 0, "2026-01-01T00:00:00.000Z");
    let high = entry("p1", 5, "2026-01-02T00:00:00.000Z");
    let earliest_same_priority = entry("p1", 0, "2026-01-01T00:00:01.000Z");
    queue.enqueue(low.clone());
    queue.enqueue(high.clone());
    queue.enqueue(earliest_same_priority.clone());

    let listed = queue.list(Some("p1"));
    assert_eq!(
        listed.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec![high.id, low.id, earliest_same_priority.id]
    );
}

#[test]
fn promote_makes_the_entry_next_for_its_project() {
    let _ws = temp_dir("promote");
    let dir = _ws.path();
    let queue = Queue::load(dir, Arc::new(Bus::new()));

    let first = entry("p1", 0, "2026-01-01T00:00:00.000Z");
    let second = entry("p1", 0, "2026-01-01T00:00:01.000Z");
    let other_project = entry("p2", 10, "2026-01-01T00:00:00.000Z");
    queue.enqueue(first.clone());
    queue.enqueue(second.clone());
    queue.enqueue(other_project.clone());

    let promoted = queue.promote(&second.id).unwrap();
    assert!(promoted.priority > first.priority);

    let listed = queue.list(Some("p1"));
    assert_eq!(listed[0].id, second.id);

    // Another project's ordering is undisturbed.
    let other_listed = queue.list(Some("p2"));
    assert_eq!(other_listed[0].id, other_project.id);
    assert_eq!(other_listed[0].priority, 10);
}

#[test]
fn cancel_removes_the_entry_and_returns_none_for_an_unknown_id() {
    let _ws = temp_dir("cancel");
    let dir = _ws.path();
    let queue = Queue::load(dir, Arc::new(Bus::new()));

    let e = entry("p1", 0, "2026-01-01T00:00:00.000Z");
    queue.enqueue(e.clone());

    assert!(queue.cancel(&e.id).is_some());
    assert!(queue.cancel(&e.id).is_none());
    assert_eq!(queue.depth(), 0);
}

#[test]
fn take_next_pops_the_head_and_persists() {
    let _ws = temp_dir("take-next");
    let dir = _ws.path();
    let queue = Queue::load(dir, Arc::new(Bus::new()));

    let first = entry("p1", 5, "2026-01-01T00:00:00.000Z");
    let second = entry("p1", 0, "2026-01-01T00:00:01.000Z");
    queue.enqueue(first.clone());
    queue.enqueue(second.clone());

    let taken = queue.take_next("p1").unwrap();
    assert_eq!(taken.id, first.id);

    let reloaded = Queue::load(dir, Arc::new(Bus::new()));
    let listed = reloaded.list(Some("p1"));
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, second.id);
}

#[test]
fn public_view_omits_the_raw_request() {
    let e = entry("p1", 0, "2026-01-01T00:00:00.000Z");
    let v = e.public();
    assert!(v.get("request").is_none());
    assert_eq!(v.get("id").and_then(|x| x.as_str()), Some(e.id.as_str()));
    assert_eq!(v.get("status").and_then(|x| x.as_str()), Some("queued"));
    assert_eq!(v.get("projectId").and_then(|x| x.as_str()), Some("p1"));
}

#[test]
fn admit_queues_a_second_launch_on_the_same_workspace_key() {
    let running = vec!["/vault/repo/LOC-1/.worktree".to_string()];
    assert_eq!(
        admit("/vault/repo/LOC-1/.worktree", &running),
        Admission::Queue
    );
}

#[test]
fn admit_starts_two_workspaces_of_one_project() {
    let running = vec!["/repo".to_string()];
    assert_eq!(
        admit("/vault/repo/LOC-1/.worktree", &running),
        Admission::Start
    );
    let running = vec!["/vault/repo/LOC-1/.worktree".to_string()];
    assert_eq!(admit("/repo", &running), Admission::Start);
}

#[test]
fn take_next_only_returns_entries_of_the_freed_workspace() {
    let ws = temp_dir("take-by-key");
    let queue = Queue::load(ws.path(), Arc::new(Bus::new()));
    queue.enqueue(entry_in("/repo", "2026-01-01T00:00:00.000Z"));
    queue.enqueue(entry_in("/wt", "2026-01-01T00:00:01.000Z"));

    let taken = queue.take_next("/wt").expect("worktree entry");
    assert_eq!(taken.workspace_key.as_deref(), Some("/wt"));
    assert!(queue.take_next("/wt").is_none());
    assert_eq!(queue.depth(), 1);
}

#[test]
fn legacy_queue_entry_without_workspace_key_deserializes() {
    let legacy = json!({
        "id": "q1",
        "projectId": "p1",
        "projectName": "P1",
        "engine": "v7",
        "status": "queued",
        "queuedAt": "2026-01-01T00:00:00.000Z",
        "origin": { "kind": "api" },
        "request": {},
    });
    let launch: QueuedLaunch = serde_json::from_value(legacy).expect("legacy entry");
    assert_eq!(launch.workspace_key, None);
    assert_eq!(launch.reason, None);
    assert_eq!(launch.admission_key(), "p1");
}

#[test]
fn queue_reason_is_workspace_busy_for_writers() {
    assert_eq!(queue_reason(false), QueueReason::WorkspaceBusy);
    assert_eq!(queue_reason(true), QueueReason::ReadOnlyCeiling);
}

#[test]
fn reason_is_exposed_publicly_and_persisted() {
    let ws = temp_dir("reason");
    let queue = Queue::load(ws.path(), Arc::new(Bus::new()));
    let queued = QueuedLaunch {
        reason: Some(QueueReason::WorkspaceBusy),
        ..entry_in("/wt", "2026-01-01T00:00:00.000Z")
    };
    assert_eq!(queued.public()["reason"], "workspace-busy");
    queue.enqueue(queued);
    let reloaded = Queue::load(ws.path(), Arc::new(Bus::new()));
    assert_eq!(
        reloaded.list(None)[0].reason,
        Some(QueueReason::WorkspaceBusy)
    );
}

const RUN_A: &str = "run-a";
const RUN_B: &str = "run-b";

fn keys(key: &str, count: usize) -> Vec<String> {
    vec![key.to_string(); count]
}

fn writers(key: &str, group: Option<&str>, count: usize) -> Vec<WriterLane> {
    vec![(key.to_string(), group.map(str::to_string)); count]
}

#[test]
fn second_writer_on_one_key_is_queued_as_workspace_busy() {
    let running = writers("/wt", None, 1);
    assert_eq!(
        admit_lane("/wt", false, None, &running, &[]),
        Admission::Queue
    );
    assert_eq!(queue_reason(false), QueueReason::WorkspaceBusy);
}

#[test]
fn writer_of_the_same_group_is_admitted_beside_live_writers() {
    let running = writers("/wt", Some(RUN_A), 2);
    assert_eq!(
        admit_lane("/wt", false, Some(RUN_A), &running, &[]),
        Admission::Start
    );
}

#[test]
fn writer_of_another_group_is_queued() {
    let running = writers("/wt", Some(RUN_A), 1);
    assert_eq!(
        admit_lane("/wt", false, Some(RUN_B), &running, &[]),
        Admission::Queue
    );
}

#[test]
fn ungrouped_writer_is_queued_behind_a_grouped_one() {
    let running = writers("/wt", Some(RUN_A), 1);
    assert_eq!(
        admit_lane("/wt", false, None, &running, &[]),
        Admission::Queue
    );
    assert_eq!(
        admit_lane("/wt", false, Some(""), &running, &[]),
        Admission::Queue
    );
}

#[test]
fn one_ungrouped_live_writer_blocks_a_grouped_one() {
    let mut running = writers("/wt", Some(RUN_A), 1);
    running.extend(writers("/wt", None, 1));
    assert_eq!(
        admit_lane("/wt", false, Some(RUN_A), &running, &[]),
        Admission::Queue
    );
}

#[test]
fn grouped_writer_on_a_free_workspace_starts() {
    let running = writers("/other", Some(RUN_B), 1);
    assert_eq!(
        admit_lane("/wt", false, Some(RUN_A), &running, &[]),
        Admission::Start
    );
}

#[test]
fn plan_session_is_admitted_beside_a_running_writer() {
    let running = writers("/wt", None, 1);
    assert_eq!(
        admit_lane("/wt", true, None, &running, &[]),
        Admission::Start
    );
}

#[test]
fn fourth_plan_session_on_one_key_is_queued() {
    let read_only = keys("/wt", READ_ONLY_CEILING);
    assert_eq!(
        admit_lane("/wt", true, None, &[], &read_only[..2]),
        Admission::Start
    );
    assert_eq!(
        admit_lane("/wt", true, None, &[], &read_only),
        Admission::Queue
    );
    assert_eq!(
        admit_lane("/other", true, None, &[], &read_only),
        Admission::Start
    );
    assert_eq!(queue_reason(true), QueueReason::ReadOnlyCeiling);
}
