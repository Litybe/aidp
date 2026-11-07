use super::*;
use crate::args::{Cli, Cmd};
use clap::Parser;

const FIELD_NAME: &str = "Package name & Build name";
const ISSUE_KEY: &str = "PIXEL-1";
const FIELD_VALUE: &str = "1.12.0";

fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
    let mut full = vec!["aidp", "jira"];
    full.extend_from_slice(arguments);
    Cli::try_parse_from(full)
}

#[test]
fn issue_edit_parses_key_field_and_value() {
    let cli = parse(&[
        "issue-edit",
        ISSUE_KEY,
        "--field",
        FIELD_NAME,
        "--value",
        FIELD_VALUE,
    ])
    .unwrap();
    match cli.cmd {
        Cmd::Jira {
            op: Op::IssueEdit { key, field, value },
        } => {
            assert_eq!(key, ISSUE_KEY);
            assert_eq!(field, FIELD_NAME);
            assert_eq!(value, FIELD_VALUE);
        }
        _ => panic!("expected jira issue-edit"),
    }
}

#[test]
fn issue_edit_without_value_is_a_clap_error() {
    let error = parse(&["issue-edit", ISSUE_KEY, "--field", FIELD_NAME])
        .err()
        .unwrap();
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}
