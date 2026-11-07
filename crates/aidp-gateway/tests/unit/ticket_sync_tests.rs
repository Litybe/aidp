use super::*;
use aidp_testkit::TempWorkspace;
use aidp_tickets::models::enums::issue_state::IssueState;
use aidp_tickets::models::github_issue::{GithubIssue, IssueDependency};
use std::sync::Mutex;

/// One recorded label edit: issue number, labels added, labels removed.
type LabelEdit = (u64, Vec<String>, Vec<String>);

/// A one-repository GitHub that records the label edits it receives.
#[derive(Default)]
struct FakeGithub {
    issues: Vec<GithubIssue>,
    edits: Mutex<Vec<LabelEdit>>,
}

impl GithubPort for FakeGithub {
    fn list_open(&self, _repo: &str, _limit: u32) -> anyhow::Result<Vec<GithubIssue>> {
        Ok(self.issues.clone())
    }

    fn view(&self, _repo: &str, number: u64) -> anyhow::Result<Option<GithubIssue>> {
        Ok(self.issues.iter().find(|i| i.number == number).cloned())
    }

    fn blocked_by(&self, _repo: &str, _number: u64) -> anyhow::Result<Vec<IssueDependency>> {
        Ok(Vec::new())
    }

    fn edit_labels(
        &self,
        _repo: &str,
        number: u64,
        add: &[String],
        remove: &[String],
    ) -> anyhow::Result<()> {
        self.edits
            .lock()
            .unwrap()
            .push((number, add.to_vec(), remove.to_vec()));
        Ok(())
    }

    fn comment(&self, _repo: &str, _number: u64, _body: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

fn issue(number: u64, body: &str, labels: &[&str]) -> GithubIssue {
    GithubIssue {
        number,
        title: format!("Issue {number}"),
        body: body.to_string(),
        labels: labels.iter().map(|l| l.to_string()).collect(),
        milestone: None,
        state: IssueState::Open,
        url: format!("https://github.com/o/r/issues/{number}"),
        created_at: None,
        updated_at: None,
    }
}

fn vault(with_repo: bool) -> TempWorkspace {
    let workspace = TempWorkspace::new("ticket-sync");
    let todo = workspace.path().join("TODO");
    std::fs::create_dir_all(&todo).unwrap();
    if with_repo {
        std::fs::write(todo.join("config.yaml"), "github:\n  repos: [o/r]\n").unwrap();
    }
    std::fs::write(
        todo.join("routing.yaml"),
        "rules:\n  - { match: { label: dev }, service: code }\nrepos:\n  code: { route: code, flow: game-code }\n",
    )
    .unwrap();
    workspace
}

#[test]
fn an_unknown_source_is_refused() {
    let workspace = vault(true);
    let store = TicketStore::open(workspace.path()).unwrap();
    let request = SyncRequest {
        source: Some("jira".to_string()),
        ..SyncRequest::default()
    };
    let error = run(&store, &FakeGithub::default(), &request).unwrap_err();
    assert!(error.downcast_ref::<SyncRefused>().is_some());
}

#[test]
fn no_configured_repository_is_refused() {
    let workspace = vault(false);
    let store = TicketStore::open(workspace.path()).unwrap();
    let error = run(&store, &FakeGithub::default(), &SyncRequest::default()).unwrap_err();
    assert!(error.downcast_ref::<SyncRefused>().is_some());
}

#[test]
fn a_sync_returns_the_report_and_a_view_carries_the_github_fields() {
    let workspace = vault(true);
    let store = TicketStore::open(workspace.path()).unwrap();
    let port = FakeGithub {
        issues: vec![issue(12, "CA-1 le dash", &["dev", "aidp:ready"])],
        ..FakeGithub::default()
    };
    let report = run(
        &store,
        &port,
        &SyncRequest {
            source: Some("github".to_string()),
            ..SyncRequest::default()
        },
    )
    .unwrap();
    assert_eq!(report["created"], 1);
    assert_eq!(report["source"], "github");

    let (ticket, _, _) = store.get("GH-12").unwrap();
    let view = ticket_view(&ticket);
    assert_eq!(view["githubNumber"], 12);
    assert_eq!(view["url"], "https://github.com/o/r/issues/12");
    assert_eq!(view["route"], "code");
    assert_eq!(view["flow"], "game-code");
    assert_eq!(view["labels"], serde_json::json!(["dev", "aidp:ready"]));
    assert!(view["milestone"].is_null());
}

#[test]
fn the_background_pass_writes_back_the_label_of_a_changed_status() {
    let workspace = vault(true);
    let store = TicketStore::open(workspace.path()).unwrap();
    let port = FakeGithub {
        issues: vec![issue(12, "CA-1 le dash", &["dev", "aidp:ready"])],
        ..FakeGithub::default()
    };
    run(&store, &port, &SyncRequest::default()).unwrap();
    assert!(
        port.edits.lock().unwrap().is_empty(),
        "already labelled ready"
    );

    let (mut ticket, body, extra) = store.get("GH-12").unwrap();
    ticket.status = aidp_tickets::model::Status::Queued;
    store.save(&mut ticket, &body, &extra).unwrap();

    let result = background_pass(workspace.path(), &port, false);
    assert_eq!(result["labelsPushed"], 1, "{result}");
    let edits = port.edits.lock().unwrap().clone();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].1, vec!["aidp:queued".to_string()]);
    assert_eq!(edits[0].2, vec!["aidp:ready".to_string()]);
}

#[test]
fn after_tick_does_nothing_without_a_configured_repository() {
    let workspace = vault(false);
    after_tick(workspace.path());
    assert!(!RUNNING.load(Ordering::SeqCst));
}
