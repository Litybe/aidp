use super::{skill_compat_check, DoctorCheckName, SKILL_COMPAT_TOO_OLD_FIX};
use aidp_core::models::enums::SkillCompat;

const RUNNING: &str = "7.1.0";
const REQUIRED_NEWER: &str = "7.2.0";
const REQUIRED_OLDER: &str = "7.0.0";
const UNREADABLE_REASON: &str = "invalid frontmatter";

#[test]
fn a_compatible_binary_passes_and_names_both_versions() {
    let verdict = SkillCompat::Compatible {
        required: REQUIRED_OLDER.to_string(),
    };
    let check = skill_compat_check(&verdict, RUNNING);
    assert_eq!(check.name, DoctorCheckName::SkillCompat);
    assert!(check.ok);
    assert!(check.detail.contains(RUNNING) && check.detail.contains(REQUIRED_OLDER));
    assert!(check.fix.is_none());
}

#[test]
fn an_undeclared_requirement_passes() {
    let check = skill_compat_check(&SkillCompat::Undeclared, RUNNING);
    assert!(check.ok);
    assert!(check.fix.is_none());
}

#[test]
fn a_too_old_binary_fails_with_the_setup_fix() {
    let verdict = SkillCompat::TooOld {
        required: REQUIRED_NEWER.to_string(),
    };
    let check = skill_compat_check(&verdict, RUNNING);
    assert!(!check.ok);
    assert!(check.detail.contains(REQUIRED_NEWER));
    assert_eq!(check.fix.as_deref(), Some(SKILL_COMPAT_TOO_OLD_FIX));
}

#[test]
fn an_unreadable_requirement_fails_with_its_reason() {
    let verdict = SkillCompat::Unreadable {
        reason: UNREADABLE_REASON.to_string(),
    };
    let check = skill_compat_check(&verdict, RUNNING);
    assert!(!check.ok);
    assert_eq!(check.detail, UNREADABLE_REASON);
    assert!(check.fix.is_some());
}
