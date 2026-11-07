use super::*;
use crate::github_issues::tests::RoutedRunner;

const EXISTING: &str = r#"[{"name":"bug"},{"name":"dev"},{"name":"aidp:done"},{"name":"aidp:need-fix"},
  {"name":"aidp:ready"},{"name":"Code"}]"#;

fn runner() -> Arc<RoutedRunner> {
    Arc::new(
        RoutedRunner::new()
            .ok("label list", EXISTING)
            .ok(
                "--label aidp:need-fix",
                r#"[{"number":4,"title":"Review KO"}]"#,
            )
            .ok(
                "--label aidp:done",
                r#"[{"number":10,"title":"PR ouverte"},{"number":11,"title":"PR mergée"}]"#,
            )
            .ok(
                "pr list",
                r#"[{"number":50,"body":"Closes #10","closingIssuesReferences":[]},
                   {"number":51,"body":"","closingIssuesReferences":[{"number":99}]}]"#,
            ),
    )
}

#[test]
fn plan_creates_only_missing_labels_and_relabels_two_cases() {
    let plan = plan(runner().as_ref(), "o/r", "aidp:").unwrap();
    let created: Vec<&str> = plan.create.iter().map(|spec| spec.name.as_str()).collect();
    assert!(created.contains(&"aidp:triage"));
    assert!(created.contains(&"aidp:awaiting-merge"));
    assert!(created.contains(&"prio:high"));
    assert!(!created.contains(&"aidp:ready"), "already there");
    assert!(!created.contains(&"aidp:done"));
    assert!(!created.contains(&"code"), "case-insensitive match");
    assert_eq!(plan.relabel.len(), 2);
    assert_eq!(plan.relabel[0].number, 4);
    assert_eq!(plan.relabel[0].add, vec!["aidp:fixing"]);
    assert_eq!(plan.relabel[0].remove, vec!["aidp:need-fix"]);
    assert_eq!(plan.relabel[1].number, 10);
    assert_eq!(plan.relabel[1].add, vec!["aidp:awaiting-merge"]);
    assert_eq!(plan.relabel[1].remove, vec!["aidp:done"]);
    assert_eq!(plan.obsolete, vec!["aidp:need-fix"]);
    assert!(!plan.applied);
}

#[test]
fn plan_writes_nothing() {
    let runner = runner();
    plan(runner.as_ref(), "o/r", "aidp:").unwrap();
    for call in runner.calls() {
        assert!(
            call.starts_with("label list")
                || call.starts_with("issue list")
                || call.starts_with("pr list"),
            "write call in a plan: {call}"
        );
    }
}

#[test]
fn apply_creates_labels_then_relabels() {
    let runner = runner();
    let planned = plan(runner.as_ref(), "o/r", "aidp:").unwrap();
    let before = runner.calls().len();
    let done = apply(runner.clone(), planned.clone());
    assert!(done.applied);
    assert!(done.errors.is_empty(), "{:?}", done.errors);
    let writes = runner.calls()[before..].to_vec();
    assert_eq!(
        writes
            .iter()
            .filter(|c| c.starts_with("label create"))
            .count(),
        planned.create.len()
    );
    let first_relabel = writes.iter().position(|c| c.starts_with("api")).unwrap();
    assert!(writes[..first_relabel]
        .iter()
        .all(|c| c.starts_with("label create")));
    assert!(writes
        .iter()
        .any(|c| c.contains("repos/o/r/issues/4/labels/aidp%3Aneed-fix")));
    assert!(writes.iter().any(|c| c.contains("--input")));
}
