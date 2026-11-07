use super::*;
use crate::models::constants::skill_rules::{FINDING_ID_PREFIX, RUST_DEV_SKILL};

const ADDED_LINE_COMMENT: &str = include_str!("../fixtures/skill_rules/added_line_comment.diff");
const ADDED_DOC_COMMENT: &str = include_str!("../fixtures/skill_rules/added_doc_comment.diff");
const ADDED_TICKET_REF: &str = include_str!("../fixtures/skill_rules/added_ticket_ref.diff");
const NON_RUST_COMMENT: &str = include_str!("../fixtures/skill_rules/non_rust_comment.diff");
const REMOVED_COMMENT: &str = include_str!("../fixtures/skill_rules/removed_comment.diff");
const RUST_FILE: &str = "src/a.rs";
const RUN_ID: &str = "run";
const GENERATED_AT: &str = "2026-01-01T00:00:00Z";
const OTHER_SKILL: &str = "other-skill";

#[test]
fn line_comment_is_reported_with_file_and_line() {
    let violations = scan_diff(ADDED_LINE_COMMENT, &[]);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].rule, SkillRule::NoLineComment);
    assert_eq!(violations[0].file, RUST_FILE);
    assert_eq!(violations[0].line, 2);
}

#[test]
fn doc_comments_are_allowed() {
    assert!(scan_diff(ADDED_DOC_COMMENT, &[]).is_empty());
}

#[test]
fn ticket_reference_is_reported() {
    let violations = scan_diff(ADDED_TICKET_REF, &[]);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].rule, SkillRule::NoTicketReference);
    assert_eq!(violations[0].line, 2);
}

#[test]
fn non_rust_files_and_removed_lines_are_ignored() {
    assert!(scan_diff(NON_RUST_COMMENT, &[]).is_empty());
    assert!(scan_diff(REMOVED_COMMENT, &[]).is_empty());
}

#[test]
fn skills_filter_selects_rules_by_owner() {
    let other = vec![OTHER_SKILL.to_string()];
    assert!(scan_diff(ADDED_LINE_COMMENT, &other).is_empty());
    let owner = vec![RUST_DEV_SKILL.to_string()];
    assert_eq!(scan_diff(ADDED_LINE_COMMENT, &owner).len(), 1);
}

#[test]
fn findings_are_blocking_with_exact_location() {
    let violations = scan_diff(ADDED_LINE_COMMENT, &[]);
    let findings = to_findings(&violations, RUN_ID, GENERATED_AT);
    assert!(findings.has_blocking_findings);
    assert_eq!(findings.severity_counts.blocking, 1);
    assert_eq!(findings.findings[0].file.as_deref(), Some(RUST_FILE));
    assert_eq!(findings.findings[0].line, Some(2));
    assert_eq!(findings.findings[0].id, format!("{FINDING_ID_PREFIX}1"));
    let failed: Vec<&Check> = findings
        .checks
        .iter()
        .filter(|check| check.status == CheckStatus::Fail)
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].finding_ids, vec![findings.findings[0].id.clone()]);
}

#[test]
fn empty_violations_pass_every_check() {
    let findings = to_findings(&[], RUN_ID, GENERATED_AT);
    assert!(!findings.has_blocking_findings);
    assert_eq!(findings.severity_counts, SeverityCounts::default());
    assert_eq!(findings.checks.len(), SkillRule::all().len());
    assert!(findings
        .checks
        .iter()
        .all(|check| check.status == CheckStatus::Pass));
}
