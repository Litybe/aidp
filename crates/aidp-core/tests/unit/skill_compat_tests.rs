use super::{check, check_manifest, min_aidp_version};
use crate::models::constants::skill_compat::{BYTE_ORDER_MARK, SKILL_MANIFEST_FILE};
use crate::models::enums::SkillCompat;

const RUNNING: &str = "7.1.0";
const SAME: &str = "7.1.0";
const OLDER: &str = "7.0.9";
const NEWER: &str = "7.2.0";
const SHORT_OLDER: &str = "7.1";
const PRERELEASE_RUNNING: &str = "7.1.0-rc.1";
const NOT_A_VERSION: &str = "seven";
const PLAIN_BODY: &str = "# A skill\n\nNo frontmatter here.\n";
const NO_KEY_MANIFEST: &str = "---\nname: a-skill\n---\nbody\n";
const UNCLOSED_MANIFEST: &str = "---\nmin_aidp_version: 9.0.0\nbody\n";
const INVALID_YAML_MANIFEST: &str = "---\nmin_aidp_version: [unclosed\n---\n";
const LINE_FEED: char = '\n';
const CARRIAGE_RETURN_LINE_FEED: &str = "\r\n";

fn manifest(min_version: &str) -> String {
    format!("---\nname: a-skill\ndescription: Something.\nmin_aidp_version: {min_version}\n---\n\n# A skill\n")
}

fn compatible(required: &str) -> SkillCompat {
    SkillCompat::Compatible {
        required: required.to_string(),
    }
}

fn too_old(required: &str) -> SkillCompat {
    SkillCompat::TooOld {
        required: required.to_string(),
    }
}

#[test]
fn reads_the_declared_version() {
    assert_eq!(
        min_aidp_version(&manifest(SAME)).unwrap().as_deref(),
        Some(SAME)
    );
}

#[test]
fn reads_the_version_after_a_byte_order_mark_and_crlf_lines() {
    let crlf = manifest(SAME).replace(LINE_FEED, CARRIAGE_RETURN_LINE_FEED);
    let text = format!("{BYTE_ORDER_MARK}{crlf}");
    assert_eq!(min_aidp_version(&text).unwrap().as_deref(), Some(SAME));
}

#[test]
fn a_manifest_without_frontmatter_declares_nothing() {
    assert_eq!(min_aidp_version(PLAIN_BODY).unwrap(), None);
    assert_eq!(check_manifest(PLAIN_BODY, RUNNING), SkillCompat::Undeclared);
}

#[test]
fn a_frontmatter_without_the_key_declares_nothing() {
    assert_eq!(
        check_manifest(NO_KEY_MANIFEST, RUNNING),
        SkillCompat::Undeclared
    );
}

#[test]
fn an_unclosed_frontmatter_declares_nothing() {
    assert_eq!(
        check_manifest(UNCLOSED_MANIFEST, RUNNING),
        SkillCompat::Undeclared
    );
}

#[test]
fn equal_and_older_requirements_are_compatible() {
    assert_eq!(check_manifest(&manifest(SAME), RUNNING), compatible(SAME));
    assert_eq!(check_manifest(&manifest(OLDER), RUNNING), compatible(OLDER));
    assert_eq!(
        check_manifest(&manifest(SHORT_OLDER), RUNNING),
        compatible(SHORT_OLDER)
    );
}

#[test]
fn a_newer_requirement_reports_the_binary_too_old() {
    assert_eq!(check_manifest(&manifest(NEWER), RUNNING), too_old(NEWER));
}

#[test]
fn a_prerelease_suffix_is_ignored() {
    assert_eq!(
        check_manifest(&manifest(SAME), PRERELEASE_RUNNING),
        compatible(SAME)
    );
}

#[test]
fn an_unparsable_version_is_unreadable() {
    let verdict = check_manifest(&manifest(NOT_A_VERSION), RUNNING);
    assert!(
        matches!(verdict, SkillCompat::Unreadable { .. }),
        "{verdict:?}"
    );
}

#[test]
fn invalid_yaml_is_unreadable() {
    assert!(matches!(
        check_manifest(INVALID_YAML_MANIFEST, RUNNING),
        SkillCompat::Unreadable { .. }
    ));
}

#[test]
fn a_skill_root_without_manifest_declares_nothing() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(check(root.path(), RUNNING), SkillCompat::Undeclared);
}

#[test]
fn a_skill_root_manifest_is_read_from_disk() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(SKILL_MANIFEST_FILE), manifest(NEWER)).unwrap();
    assert_eq!(check(root.path(), RUNNING), too_old(NEWER));
}
