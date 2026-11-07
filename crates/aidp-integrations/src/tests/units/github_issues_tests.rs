use super::*;
use aidp_core::models::process::{ProcessSpec, ScriptedOutput};
use aidp_core::process_runner::{ScriptedProcessRunner, SpawnedChild};
use std::sync::Mutex;

/// A `gh` stand-in: the first route whose key is contained in the joined argv answers.
pub(crate) struct RoutedRunner {
    routes: Vec<(String, i32, String, String)>,
    calls: Mutex<Vec<String>>,
}

impl RoutedRunner {
    pub(crate) fn new() -> Self {
        Self {
            routes: Vec::new(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn ok(mut self, key: &str, stdout: &str) -> Self {
        self.routes
            .push((key.to_string(), 0, stdout.to_string(), String::new()));
        self
    }

    pub(crate) fn fail(mut self, key: &str, stderr: &str) -> Self {
        self.routes
            .push((key.to_string(), 1, String::new(), stderr.to_string()));
        self
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl ProcessRunner for RoutedRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        let line = spec.args.join(" ");
        self.calls.lock().unwrap().push(line.clone());
        let (code, stdout, stderr) = self
            .routes
            .iter()
            .find(|(key, ..)| line.contains(key.as_str()))
            .map(|(_, code, out, err)| (*code, out.clone(), err.clone()))
            .unwrap_or((0, String::new(), String::new()));
        ScriptedProcessRunner::new()
            .with_output(ScriptedOutput {
                status_code: code,
                stdout: stdout.into_bytes(),
                stderr: stderr.into_bytes(),
            })
            .output(spec)
    }

    fn spawn(&self, _spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        Err(std::io::Error::other("spawn is not scripted"))
    }
}

fn port(runner: &Arc<RoutedRunner>) -> GhIssuePort {
    GhIssuePort::with_runner(runner.clone())
}

const LIST: &str = r#"[
  {"number":12,"title":"Dash","body":"Dépend de #3\n- CA-1 ok","state":"OPEN","url":"https://github.com/o/r/issues/12",
   "labels":[{"name":"dev"},{"name":"aidp:ready"}],
   "milestone":{"number":2,"title":"v0.2","dueOn":"2026-12-01T00:00:00Z"},
   "createdAt":"2026-11-01T10:00:00Z","updatedAt":"2026-11-02T10:00:00Z"},
  {"number":13,"title":"Sans milestone","body":"","state":"OPEN","url":"u","labels":[],"milestone":null}
]"#;

#[test]
fn list_open_reads_gh_json_and_asks_for_the_limit() {
    let runner = Arc::new(RoutedRunner::new().ok("issue list", LIST));
    let issues = port(&runner).list_open("o/r", 50).unwrap();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].number, 12);
    assert_eq!(issues[0].labels, vec!["dev", "aidp:ready"]);
    assert_eq!(issues[0].state, IssueState::Open);
    let milestone = issues[0].milestone.as_ref().unwrap();
    assert_eq!(milestone.title, "v0.2");
    assert!(milestone.due_on.is_some());
    assert!(issues[1].milestone.is_none());
    let call = &runner.calls()[0];
    assert!(call.contains("--repo o/r"), "{call}");
    assert!(call.contains("--limit 50"), "{call}");
    assert!(
        call.contains("--json number,title,body,labels,milestone,state"),
        "{call}"
    );
}

#[test]
fn view_reads_rest_json_and_maps_not_found_to_none() {
    let rest = r#"{"number":3,"title":"t","body":"b","state":"closed","html_url":"h",
        "labels":[{"name":"bug"}],"milestone":{"title":"m","due_on":null}}"#;
    let runner = Arc::new(
        RoutedRunner::new()
            .ok("issues/3", rest)
            .fail("issues/4", "gh: Not Found (HTTP 404)"),
    );
    let issue = port(&runner).view("o/r", 3).unwrap().unwrap();
    assert_eq!(issue.state, IssueState::Closed);
    assert_eq!(issue.url, "h");
    assert!(port(&runner).view("o/r", 4).unwrap().is_none());
}

#[test]
fn blocked_by_reads_native_dependencies_with_their_repository() {
    let deps = r#"[{"number":7,"state":"open","repository_url":"https://api.github.com/repos/o/other"},
                   {"number":8,"state":"closed"}]"#;
    let runner = Arc::new(RoutedRunner::new().ok("dependencies/blocked_by", deps));
    let found = port(&runner).blocked_by("o/r", 1).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!((found[0].repo.as_str(), found[0].number), ("o/other", 7));
    assert_eq!(found[0].state, IssueState::Open);
    assert_eq!(found[1].repo, "o/r");
    assert_eq!(found[1].state, IssueState::Closed);
}

#[test]
fn blocked_by_is_empty_when_the_route_does_not_exist() {
    let runner = Arc::new(RoutedRunner::new().fail("blocked_by", "HTTP 404"));
    assert!(port(&runner).blocked_by("o/r", 1).unwrap().is_empty());
}

#[test]
fn edit_labels_posts_the_added_and_deletes_each_removed_label_encoded() {
    let runner = Arc::new(RoutedRunner::new().fail("labels/aidp%3Aold", "Not Found (HTTP 404)"));
    port(&runner)
        .edit_labels(
            "o/r",
            9,
            &["aidp:review".to_string()],
            &["aidp:old".to_string(), "aidp:other".to_string()],
        )
        .unwrap();
    let calls = runner.calls();
    assert!(
        calls[0].starts_with("api -X POST repos/o/r/issues/9/labels --input"),
        "{calls:?}"
    );
    assert!(
        calls[1].contains("-X DELETE repos/o/r/issues/9/labels/aidp%3Aold"),
        "{calls:?}"
    );
    assert!(calls[2].contains("labels/aidp%3Aother"), "{calls:?}");
}

#[test]
fn edit_labels_surfaces_a_real_failure() {
    let runner = Arc::new(RoutedRunner::new().fail("-X POST", "HTTP 403 forbidden"));
    let result = port(&runner).edit_labels("o/r", 9, &["x".to_string()], &[]);
    assert!(result.is_err());
}

#[test]
fn comment_posts_a_body_file_and_returns_the_url() {
    let runner = Arc::new(
        RoutedRunner::new().ok("issues/5/comments", r#"{"id":1,"html_url":"https://c/1"}"#),
    );
    let url = port(&runner).comment("o/r", 5, "rapport").unwrap();
    assert_eq!(url.as_deref(), Some("https://c/1"));
    assert!(runner.calls()[0].contains("--input"));
}

#[test]
fn encode_segment_escapes_everything_but_unreserved() {
    assert_eq!(encode_segment("aidp:ready"), "aidp%3Aready");
    assert_eq!(encode_segment("good first issue"), "good%20first%20issue");
    assert_eq!(encode_segment("a-b_c.d~e"), "a-b_c.d~e");
}
