//! Pure checker of the added lines of a unified diff against the skill rule catalogue.

use crate::findings::{Check, CheckStatus, Finding, Findings, Severity, SeverityCounts};
use crate::models::constants::skill_rules::{
    ADDED_PREFIX, CHECK_ID_PREFIX, CONTEXT_PREFIX, DEV_NULL_PATH, DOC_COMMENT_CHARS,
    FILE_HEADER_PREFIX, FINDING_CATEGORY, FINDING_ID_PREFIX, GENERATED_ITERATION,
    GENERATED_SCHEMA_VERSION, HUNK_NEW_RANGE_PREFIX, HUNK_PREFIX, HUNK_RANGE_SEPARATOR,
    LINE_COMMENT_MARKER, NEW_FILE_PREFIX, REMOVED_PREFIX, TICKET_PATTERN, TITLE_SKILL_END,
    TITLE_SKILL_SEPARATOR,
};
use crate::models::enums::skill_rule::SkillRule;
use crate::models::rule_violation::RuleViolation;
use regex::Regex;
use std::sync::LazyLock;

static TICKET_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(TICKET_PATTERN).expect("TICKET_PATTERN is a valid regex"));

/// Whether `rule` fires on the added line `text`.
fn rule_fires(rule: SkillRule, text: &str) -> bool {
    match rule {
        SkillRule::NoLineComment => {
            let trimmed = text.trim();
            trimmed.starts_with(LINE_COMMENT_MARKER)
                && !trimmed
                    .chars()
                    .nth(LINE_COMMENT_MARKER.len())
                    .is_some_and(|third| DOC_COMMENT_CHARS.contains(&third))
        }
        SkillRule::NoTicketReference => TICKET_REGEX.is_match(text),
    }
}

/// Start line of the new-file range of a hunk header, if it parses.
fn hunk_new_start(header: &str) -> Option<u32> {
    let range = header
        .split_whitespace()
        .find(|token| token.starts_with(HUNK_NEW_RANGE_PREFIX))?;
    let start = range.trim_start_matches(HUNK_NEW_RANGE_PREFIX);
    start.split(HUNK_RANGE_SEPARATOR).next()?.parse().ok()
}

/// Violations of the rules owned by `skills` (all rules when empty) on the added lines of `diff`.
pub fn scan_diff(diff: &str, skills: &[String]) -> Vec<RuleViolation> {
    let rules: Vec<SkillRule> = SkillRule::all()
        .iter()
        .copied()
        .filter(|rule| skills.is_empty() || skills.iter().any(|skill| skill == rule.skill()))
        .collect();
    let mut violations = Vec::new();
    let mut current_file: Option<String> = None;
    let mut line_number: u32 = 0;
    for diff_line in diff.lines() {
        if let Some(path) = diff_line.strip_prefix(NEW_FILE_PREFIX) {
            current_file = Some(path.to_string());
            continue;
        }
        if diff_line.starts_with(FILE_HEADER_PREFIX) {
            if diff_line.ends_with(DEV_NULL_PATH) {
                current_file = None;
            }
            continue;
        }
        if diff_line.starts_with(HUNK_PREFIX) {
            line_number = hunk_new_start(diff_line).unwrap_or(0);
            continue;
        }
        if diff_line.starts_with(REMOVED_PREFIX) {
            continue;
        }
        let Some(file) = current_file.as_deref() else {
            continue;
        };
        if let Some(text) = diff_line.strip_prefix(ADDED_PREFIX) {
            for rule in rules.iter().filter(|rule| rule.applies_to(file)) {
                if rule_fires(*rule, text) {
                    violations.push(RuleViolation {
                        rule: *rule,
                        file: file.to_string(),
                        line: line_number,
                        excerpt: text.trim().to_string(),
                    });
                }
            }
            line_number += 1;
        } else if diff_line.is_empty() || diff_line.starts_with(CONTEXT_PREFIX) {
            line_number += 1;
        }
    }
    violations
}

fn finding_title(rule: SkillRule) -> String {
    format!(
        "{}{}{}{}",
        rule.id(),
        TITLE_SKILL_SEPARATOR,
        rule.skill(),
        TITLE_SKILL_END
    )
}

fn check_for(rule: SkillRule, violated_ids: Vec<String>) -> Check {
    let status = if violated_ids.is_empty() {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    Check {
        id: format!("{CHECK_ID_PREFIX}{}", rule.id()),
        name: rule.message().to_string(),
        status,
        harness_gap: None,
        evidence: None,
        finding_ids: violated_ids,
    }
}

/// Findings document with one blocking finding per violation and one check per rule.
pub fn to_findings(violations: &[RuleViolation], run_id: &str, generated_at: &str) -> Findings {
    let findings: Vec<Finding> = violations
        .iter()
        .enumerate()
        .map(|(index, violation)| Finding {
            id: format!("{FINDING_ID_PREFIX}{}", index + 1),
            severity: Severity::Blocking,
            category: FINDING_CATEGORY.to_string(),
            file: Some(violation.file.clone()),
            line: Some(i64::from(violation.line)),
            title: finding_title(violation.rule),
            description: Some(format!(
                "{}: {}",
                violation.rule.message(),
                violation.excerpt
            )),
            suggestion: None,
            related_ids: Vec::new(),
            location: None,
        })
        .collect();
    let checks: Vec<Check> = SkillRule::all()
        .iter()
        .map(|rule| {
            let ids = violations
                .iter()
                .zip(&findings)
                .filter(|(violation, _)| violation.rule == *rule)
                .map(|(_, finding)| finding.id.clone())
                .collect();
            check_for(*rule, ids)
        })
        .collect();
    Findings {
        schema_version: GENERATED_SCHEMA_VERSION,
        run_id: run_id.to_string(),
        iter: GENERATED_ITERATION,
        generated_at: generated_at.to_string(),
        severity_counts: SeverityCounts {
            blocking: u32::try_from(findings.len()).unwrap_or(u32::MAX),
            ..SeverityCounts::default()
        },
        has_blocking_findings: !findings.is_empty(),
        findings,
        checks,
        scores: Default::default(),
    }
}

#[cfg(test)]
#[path = "../tests/unit/skill_rules_tests.rs"]
mod tests;
