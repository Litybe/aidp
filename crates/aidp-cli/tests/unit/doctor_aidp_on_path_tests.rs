use super::{aidp_on_path_check, DoctorCheckName};
use aidp_core::aidp_binary::{AidpBinaryFacts, AidpBinaryIssue, AidpBinaryReport};
use std::path::PathBuf;

const INSTALLED: &str = "/opt/aidp/release/aidp";
const OTHER: &str = "/usr/local/bin/aidp";
const DEBUG_BUILD: &str = "/work/rust/target/debug/aidp";

fn report_with(issues: Vec<AidpBinaryIssue>, resolved: Option<&str>) -> AidpBinaryReport {
    AidpBinaryReport {
        resolved: resolved.map(PathBuf::from),
        running: Some(PathBuf::from(INSTALLED)),
        resolved_modified: None,
        running_modified: None,
        issues,
    }
}

#[test]
fn ok_report_passes_without_fix() {
    let check = aidp_on_path_check(&report_with(Vec::new(), Some(INSTALLED)));
    assert!(check.ok);
    assert!(check.fix.is_none());
    assert!(check.detail.contains("aidp"));
    assert_eq!(check.name, DoctorCheckName::AidpOnPath);
}

#[test]
fn ok_report_detail_includes_build_time_when_known() {
    let mut report = report_with(Vec::new(), Some(INSTALLED));
    report.resolved_modified = Some(std::time::UNIX_EPOCH);
    let check = aidp_on_path_check(&report);
    assert!(check.detail.contains("(built 1970-01-01T00:00:00"));
}

#[test]
fn absent_report_fails_with_path_fix() {
    let check = aidp_on_path_check(&report_with(vec![AidpBinaryIssue::Absent], None));
    assert!(!check.ok);
    assert!(check.detail.contains("absent"));
    assert!(check.detail.contains("aidp is not on PATH"));
    let fix = check.fix.expect("fix");
    assert!(fix.starts_with("Add '"));
    assert!(fix.contains("/pipeline-setup-v7"));
}

#[test]
fn target_build_report_fails_with_release_install_fix() {
    let report = aidp_core::aidp_binary::evaluate(AidpBinaryFacts {
        resolved: Some(PathBuf::from(DEBUG_BUILD)),
        running: Some(PathBuf::from(DEBUG_BUILD)),
        ..AidpBinaryFacts::default()
    });
    let check = aidp_on_path_check(&report);
    assert!(!check.ok);
    assert!(check.detail.contains("target_build"));
    assert!(check.fix.expect("fix").contains("release install"));
}

#[test]
fn mismatch_report_names_both_paths() {
    let check = aidp_on_path_check(&report_with(vec![AidpBinaryIssue::Mismatch], Some(OTHER)));
    assert!(!check.ok);
    assert!(check.detail.contains(OTHER));
    assert!(check.detail.contains(&format!("(running: {INSTALLED})")));
    assert!(check.fix.expect("fix").contains("different aidp"));
}

#[test]
fn older_than_sources_report_suggests_rebuild() {
    let check = aidp_on_path_check(&report_with(
        vec![AidpBinaryIssue::OlderThanSources],
        Some(INSTALLED),
    ));
    assert!(!check.ok);
    assert!(check.fix.expect("fix").contains("rebuild"));
}
