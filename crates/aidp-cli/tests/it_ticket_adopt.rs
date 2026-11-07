//! `aidp ticket adopt` help and the epic block appended by `aidp ticket render`.

use crate::common;
use aidp_testkit::TempWorkspace;
use aidp_tickets::model::Source;
use common::*;
use std::fs;
use std::path::Path;

const TICKET_TEMPLATE: &str = include_str!("fixtures/ticket_adopt/ticket.md");
const KEY_PLACEHOLDER: &str = "{key}";
const SOURCE_PLACEHOLDER: &str = "{source}";
const EXTRA_FRONTMATTER_PLACEHOLDER: &str = "{extra_frontmatter}";

/// Writes a ticket note from the fixture template; Jira-sourced when extra frontmatter is given.
fn write_ticket(vault: &Path, key: &str, extra_frontmatter: &str) {
    let dir = vault.join("TODO").join("demo");
    fs::create_dir_all(&dir).expect("create TODO/demo");
    let source = if extra_frontmatter.is_empty() {
        Source::Local
    } else {
        Source::Github
    };
    let content = TICKET_TEMPLATE
        .replace(KEY_PLACEHOLDER, key)
        .replace(SOURCE_PLACEHOLDER, source.as_str())
        .replace(EXTRA_FRONTMATTER_PLACEHOLDER, extra_frontmatter);
    fs::write(dir.join(format!("{key}.md")), content).expect("write ticket");
}

fn render(ws: &TempWorkspace, key: &str) -> String {
    let output = aidp(ws)
        .arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("render")
        .arg(key)
        .output()
        .expect("run ticket render");
    assert_eq!(output.status.code(), Some(0));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn adopt_help_lists_the_pr_flag() {
    let ws = workspace();
    let output = aidp(&ws)
        .args(["ticket", "adopt", "--help"])
        .output()
        .expect("run adopt --help");
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("--pr"));
}

#[test]
fn render_prints_no_block_for_a_ticket_without_epic() {
    let ws = workspace();
    let vault = ws.child("vault");
    write_ticket(&vault, "LOC-3", "");

    assert_eq!(render(&ws, "LOC-3"), "Body.\n");
}
