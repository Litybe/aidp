//! Integration test: the GitHub issue sync and its writes (labels, comment) against the mock
//! server's issue routes. A `GithubPort` over the pure router `mock::handle` carries the calls,
//! so no socket is opened.

use aidp_integrations::github_issues::{encode_segment, issue_from_json};
use aidp_integrations::mock::{handle, State};
use aidp_testkit::TempWorkspace;
use aidp_tickets::github_sync::{comment_ticket, label_ticket, sync, SyncOptions};
use aidp_tickets::models::enums::issue_state::IssueState;
use aidp_tickets::models::github_issue::{GithubIssue, IssueDependency};
use aidp_tickets::models::github_port::GithubPort;
use aidp_tickets::store::TicketStore;
use serde_json::{json, Value};
use std::sync::Mutex;
use tiny_http::Method;

const REPO: &str = "Litybe/PixelHood";

/// The mock issue routes behind the port the sync reads and writes through.
struct MockIssuePort {
    state: Mutex<State>,
}

impl MockIssuePort {
    fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }

    fn call(&self, method: Method, url: &str, body: Value) -> anyhow::Result<(u16, Value)> {
        handle(&method, url, &body.to_string(), &self.state)
    }

    fn seed(&self, issues: Value) {
        let (code, _) = self
            .call(
                Method::Post,
                "/__seed_issues",
                json!({ "repo": REPO, "issues": issues }),
            )
            .unwrap();
        assert_eq!(code, 200);
    }

    fn labels_of(&self, number: u64) -> Vec<String> {
        let (_, issue) = self
            .call(
                Method::Get,
                &format!("/github/repos/{REPO}/issues/{number}"),
                Value::Null,
            )
            .unwrap();
        issue_from_json(&issue).unwrap().labels
    }
}

impl GithubPort for MockIssuePort {
    fn list_open(&self, repo: &str, limit: u32) -> anyhow::Result<Vec<GithubIssue>> {
        let (_, list) = self.call(
            Method::Get,
            &format!("/github/repos/{repo}/issues?state=open"),
            Value::Null,
        )?;
        Ok(list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(issue_from_json)
            .take(limit as usize)
            .collect())
    }

    fn view(&self, repo: &str, number: u64) -> anyhow::Result<Option<GithubIssue>> {
        let (code, issue) = self.call(
            Method::Get,
            &format!("/github/repos/{repo}/issues/{number}"),
            Value::Null,
        )?;
        Ok((code == 200).then(|| issue_from_json(&issue)).flatten())
    }

    fn blocked_by(&self, repo: &str, number: u64) -> anyhow::Result<Vec<IssueDependency>> {
        let (_, deps) = self.call(
            Method::Get,
            &format!("/github/repos/{repo}/issues/{number}/dependencies/blocked_by"),
            Value::Null,
        )?;
        Ok(deps
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|dep| {
                Some(IssueDependency {
                    repo: repo.to_string(),
                    number: dep["number"].as_u64()?,
                    state: IssueState::from_wire(dep["state"].as_str()?),
                })
            })
            .collect())
    }

    fn edit_labels(
        &self,
        repo: &str,
        number: u64,
        add: &[String],
        remove: &[String],
    ) -> anyhow::Result<()> {
        if !add.is_empty() {
            let (code, _) = self.call(
                Method::Post,
                &format!("/github/repos/{repo}/issues/{number}/labels"),
                json!({ "labels": add }),
            )?;
            anyhow::ensure!(code == 200, "add labels: HTTP {code}");
        }
        for label in remove {
            self.call(
                Method::Delete,
                &format!(
                    "/github/repos/{repo}/issues/{number}/labels/{}",
                    encode_segment(label)
                ),
                Value::Null,
            )?;
        }
        Ok(())
    }

    fn comment(&self, repo: &str, number: u64, body: &str) -> anyhow::Result<Option<String>> {
        let (code, reply) = self.call(
            Method::Post,
            &format!("/github/repos/{repo}/issues/{number}/comments"),
            json!({ "body": body }),
        )?;
        anyhow::ensure!(code == 201, "comment: HTTP {code}");
        Ok(reply["html_url"].as_str().map(str::to_string))
    }
}

fn vault() -> TempWorkspace {
    let workspace = TempWorkspace::new("mock-issues");
    let todo = workspace.path().join("TODO");
    std::fs::create_dir_all(&todo).unwrap();
    std::fs::write(
        todo.join("config.yaml"),
        "github:\n  repos: [Litybe/PixelHood]\n",
    )
    .unwrap();
    std::fs::write(
        todo.join("routing.yaml"),
        "rules:\n  - { match: { label: dev }, service: pixelhood-code }\nrepos:\n  pixelhood-code: { route: code, flow: game-code }\n",
    )
    .unwrap();
    workspace
}

#[test]
fn sync_then_label_and_comment_run_against_the_mock_issue_routes() {
    let port = MockIssuePort::new();
    port.seed(json!([
        { "number": 1, "title": "Dash", "body": "CA-1 : le dash dure 0,2 s", "labels": ["dev", "aidp:ready"] },
        { "number": 2, "title": "Wall jump", "body": "Dépend de #1\nCA-1 ok", "labels": ["dev", "aidp:ready"] },
        { "number": 3, "title": "Vague", "body": "pas de critères", "labels": ["dev"] }
    ]));
    let workspace = vault();
    let store = TicketStore::open(workspace.path()).unwrap();

    let report = sync(&store, &port, &SyncOptions::default()).unwrap();
    assert_eq!(report.created, 3, "{report:?}");
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let (one, _, _) = store.get("GH-1").unwrap();
    assert_eq!(one.route.as_deref(), Some("code"));
    assert_eq!(one.flow.as_deref(), Some("game-code"));
    let (two, _, _) = store.get("GH-2").unwrap();
    assert!(two
        .github
        .as_ref()
        .unwrap()
        .blocked_by
        .iter()
        .any(|link| link.key == "GH-1"));
    // The issue without criteria goes to triage on GitHub: the write-back swapped its label.
    assert!(port.labels_of(3).contains(&"aidp:triage".to_string()));

    // A flow phase: one state label at a time.
    let outcome = label_ticket(&store, &port, "GH-1", "review").unwrap();
    assert!(outcome.changed);
    let labels = port.labels_of(1);
    assert!(labels.contains(&"aidp:review".to_string()));
    assert!(!labels.contains(&"aidp:ready".to_string()));
    assert_eq!(
        labels.iter().filter(|l| l.starts_with("aidp:")).count(),
        1,
        "{labels:?}"
    );

    let url = comment_ticket(&store, &port, "GH-1", "Revue terminée").unwrap();
    assert!(url.unwrap().contains("issues/1#comment-1"));
}

#[test]
fn mock_issue_routes_answer_like_github() {
    let port = MockIssuePort::new();
    port.seed(json!([{ "number": 7, "title": "t", "body": "", "labels": ["a:b"] }]));

    let (code, _) = port
        .call(
            Method::Get,
            &format!("/github/repos/{REPO}/issues/99"),
            Value::Null,
        )
        .unwrap();
    assert_eq!(code, 404);

    let (code, _) = port
        .call(
            Method::Delete,
            &format!("/github/repos/{REPO}/issues/7/labels/zzz"),
            Value::Null,
        )
        .unwrap();
    assert_eq!(code, 404, "removing an absent label");

    let (code, _) = port
        .call(
            Method::Delete,
            &format!(
                "/github/repos/{REPO}/issues/7/labels/{}",
                encode_segment("a:b")
            ),
            Value::Null,
        )
        .unwrap();
    assert_eq!(code, 200);
    assert!(port.labels_of(7).is_empty());

    let (_, closed) = port
        .call(
            Method::Get,
            &format!("/github/repos/{REPO}/issues?state=closed"),
            Value::Null,
        )
        .unwrap();
    assert_eq!(closed, json!([]));
}
