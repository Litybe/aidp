use crate::args::{gh, jira};
use clap::Parser;

const BIN: &str = "aidp";
const KEY: &str = "PROJ-1";
const STATUS: &str = "In Review";

#[derive(Parser)]
struct JiraCli {
    #[command(subcommand)]
    op: jira::Op,
}

#[derive(Parser)]
struct GhCli {
    #[command(subcommand)]
    op: gh::Op,
}

fn jira_op(args: &[&str]) -> Result<jira::Op, clap::Error> {
    let mut argv = vec![BIN];
    argv.extend_from_slice(args);
    JiraCli::try_parse_from(argv).map(|cli| cli.op)
}


#[test]
fn flow_verbs_issue_assign_requires_me() {
    assert!(matches!(
        jira_op(&["issue-assign", KEY, "--me"]),
        Ok(jira::Op::IssueAssign { .. })
    ));
    assert!(jira_op(&["issue-assign", KEY]).is_err());
}

#[test]
fn flow_verbs_transition_accepts_to_status() {
    match jira_op(&["issue-transition", "--key", KEY, "--to-status", STATUS]) {
        Ok(jira::Op::IssueTransition {
            transition_id,
            to_status,
            ..
        }) => {
            assert_eq!(transition_id, None);
            assert_eq!(to_status.as_deref(), Some(STATUS));
        }
        _ => panic!("expected IssueTransition"),
    }
}

#[test]
fn flow_verbs_transition_accepts_id_alone() {
    assert!(jira_op(&["issue-transition", "--key", KEY, "--transition-id", "31"]).is_ok());
}

#[test]
fn flow_verbs_transition_refuses_id_with_status() {
    let args = [
        "issue-transition",
        "--key",
        KEY,
        "--transition-id",
        "31",
        "--to-status",
        STATUS,
    ];
    assert!(jira_op(&args).is_err());
}

#[test]
fn flow_verbs_transition_requires_a_target() {
    assert!(jira_op(&["issue-transition", "--key", KEY]).is_err());
}


