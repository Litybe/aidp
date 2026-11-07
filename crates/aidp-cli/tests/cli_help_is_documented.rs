//! Every option and subcommand listed by `--help` carries a description.

use crate::common;
use common::{aidp, workspace, SUBCOMMANDS};

const COMMANDS_HEADING: &str = "Commands:";
const OPTIONS_HEADING: &str = "Options:";
const ARGUMENTS_HEADING: &str = "Arguments:";
const OBSOLETE_ABOUT: &str = "ai-dev-pipeline v4 helper";
const OBSOLETE_VERSION: &str = "aidp 4.0.0";
const ITEM_INDENT_MAX: usize = 6;
const COMMAND_INDENT: usize = 2;

#[derive(PartialEq)]
enum Section {
    Other,
    Commands,
    Options,
}

/// Removes trailing bracketed annotations such as `[default: x]` or `[possible values: a, b]`.
fn strip_annotations(text: &str) -> &str {
    let mut rest = text.trim();
    while rest.ends_with(']') {
        match rest.rfind('[') {
            Some(start) => rest = rest[..start].trim_end(),
            None => break,
        }
    }
    rest
}

/// Splits a help line into the item name and its description at the first run of two spaces.
fn split_item(line: &str) -> (&str, &str) {
    let trimmed = line.trim_start();
    match trimmed.find("  ") {
        Some(gap) => (&trimmed[..gap], strip_annotations(&trimmed[gap..])),
        None => (trimmed, ""),
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Returns the items of a help text whose description is empty.
fn undocumented_items(help: &str) -> Vec<String> {
    let mut section = Section::Other;
    let mut missing = Vec::new();
    for line in help.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if indent_of(line) == 0 {
            section = match line.trim() {
                COMMANDS_HEADING => Section::Commands,
                OPTIONS_HEADING => Section::Options,
                ARGUMENTS_HEADING => Section::Other,
                _ => Section::Other,
            };
            continue;
        }
        let indent = indent_of(line);
        let is_item = match section {
            Section::Commands => indent == COMMAND_INDENT,
            Section::Options => indent <= ITEM_INDENT_MAX && line.trim_start().starts_with('-'),
            Section::Other => false,
        };
        if !is_item {
            continue;
        }
        let (name, description) = split_item(line);
        if description.is_empty() {
            missing.push(name.to_string());
        }
    }
    missing
}

fn help_of(path: &[&str], flag: &str) -> String {
    let ws = workspace();
    let output = aidp(&ws)
        .args(path)
        .arg(flag)
        .output()
        .expect("help should execute");
    assert_eq!(output.status.code(), Some(0), "{path:?} {flag} failed");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn snapshotted_help_paths() -> Vec<Vec<&'static str>> {
    let mut paths: Vec<Vec<&'static str>> = vec![vec![]];
    paths.extend(SUBCOMMANDS.iter().map(|name| vec![*name]));
    paths
}

#[test]
fn no_option_or_subcommand_has_an_empty_description() {
    let mut failures = Vec::new();
    for path in snapshotted_help_paths() {
        for name in undocumented_items(&help_of(&path, "-h")) {
            failures.push(format!("aidp {} -h: `{name}`", path.join(" ")));
        }
    }

    assert!(
        failures.is_empty(),
        "items without a description:\n{}",
        failures.join("\n")
    );
}

#[test]
fn undocumented_items_are_detected_by_the_parser() {
    let help = "Usage: x\n\nCommands:\n  documented  Does a thing\n  silent\n\nOptions:\n      --format <FORMAT>  [possible values: a, b]\n  -h, --help             Print help\n";

    assert_eq!(
        undocumented_items(help),
        vec!["silent".to_string(), "--format <FORMAT>".to_string()]
    );
}

#[test]
fn version_and_about_no_longer_carry_the_v4_identity() {
    let ws = workspace();

    let version = aidp(&ws)
        .arg("--version")
        .output()
        .expect("version should execute");

    assert_ne!(
        String::from_utf8_lossy(&version.stdout).trim(),
        OBSOLETE_VERSION
    );
    assert!(!help_of(&[], "--help").contains(OBSOLETE_ABOUT));
}
