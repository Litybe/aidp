use super::*;
use clap::Parser;

#[derive(Parser, Debug)]
struct Harness {
    #[command(subcommand)]
    op: Op,
}

fn parse(args: &[&str]) -> Op {
    let mut full = vec!["harness"];
    full.extend_from_slice(args);
    Harness::try_parse_from(full).unwrap().op
}

#[test]
fn list_parses_without_project() {
    match parse(&["worktrees", "list"]) {
        Op::Worktrees {
            op: WorktreesOp::List { project },
            ..
        } => assert_eq!(project, None),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn list_parses_project_flag() {
    match parse(&["worktrees", "list", "--project", "proj-a"]) {
        Op::Worktrees {
            op: WorktreesOp::List { project },
            ..
        } => assert_eq!(project.as_deref(), Some("proj-a")),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn prune_defaults_to_real_run() {
    match parse(&["worktrees", "prune"]) {
        Op::Worktrees {
            op: WorktreesOp::Prune { dry_run },
            ..
        } => assert!(!dry_run),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn prune_parses_dry_run() {
    match parse(&["worktrees", "prune", "--dry-run"]) {
        Op::Worktrees {
            op: WorktreesOp::Prune { dry_run },
            ..
        } => assert!(dry_run),
        other => panic!("unexpected {other:?}"),
    }
}

const TICKET_KEY: &str = "LOC-1";

/// `ensure` takes the ticket key as its positional argument.
#[test]
fn ensure_parses_the_ticket_key() {
    match parse(&["worktrees", "ensure", TICKET_KEY]) {
        Op::Worktrees {
            op: WorktreesOp::Ensure { key },
            ..
        } => assert_eq!(key, TICKET_KEY),
        other => panic!("unexpected {other:?}"),
    }
}

/// `remove` takes the ticket key as its positional argument.
#[test]
fn remove_parses_the_ticket_key() {
    match parse(&["worktrees", "remove", TICKET_KEY]) {
        Op::Worktrees {
            op: WorktreesOp::Remove { key },
            ..
        } => assert_eq!(key, TICKET_KEY),
        other => panic!("unexpected {other:?}"),
    }
}

/// `ensure` without a key is a usage error.
#[test]
fn ensure_requires_a_key() {
    let result = Harness::try_parse_from(["harness", "worktrees", "ensure"]);
    assert!(result.is_err());
}
