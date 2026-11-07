use super::{longpaths_check, DoctorCheckName};

const FIX: &str = "git config --global core.longpaths true";

#[test]
fn true_value_is_ok_without_fix() {
    let check = longpaths_check(Some("true"), true).expect("windows yields a check");
    assert!(check.ok);
    assert!(check.fix.is_none());
    assert_eq!(check.name, DoctorCheckName::GitLongpaths);
}

#[test]
fn absent_value_warns_with_fix() {
    let check = longpaths_check(None, true).expect("windows yields a check");
    assert!(check.detail.starts_with("warning"));
    assert_eq!(check.fix.as_deref(), Some(FIX));
}

#[test]
fn false_value_warns_with_fix() {
    let check = longpaths_check(Some("false"), true).expect("windows yields a check");
    assert_eq!(check.fix.as_deref(), Some(FIX));
}

#[test]
fn warning_does_not_block_doctor() {
    let check = longpaths_check(None, true).expect("windows yields a check");
    assert!(check.ok);
}

#[test]
fn non_windows_yields_no_check() {
    assert!(longpaths_check(None, false).is_none());
    assert!(longpaths_check(Some("true"), false).is_none());
}
