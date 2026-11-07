use super::*;
use serde_json::json;

const CONFIG_PATH: &str = "/repo/.aidp/dev-target.json";
const CHECK_NAME: &str = "dev_target_config";
const CHECK_DETAIL: &str = "found /repo/.aidp/dev-target.json";
const CHECK_FIX: &str = "Create .aidp/dev-target.json (see the dev-target-config skill).";

/// A check with no `fix` omits the key entirely rather than serializing null.
#[test]
fn passing_check_omits_fix_key() {
    let check = DevTargetCheck::new(CHECK_NAME, true, CHECK_DETAIL.to_string(), None);
    let value = serde_json::to_value(&check).unwrap();
    assert_eq!(
        value,
        json!({"name": CHECK_NAME, "ok": true, "detail": CHECK_DETAIL})
    );
}

/// A failing check carries its `fix` under the same key `aidp dev check`
/// already emits on the wire.
#[test]
fn failing_check_carries_fix_key() {
    let check = DevTargetCheck::new(CHECK_NAME, false, CHECK_DETAIL.to_string(), Some(CHECK_FIX));
    let value = serde_json::to_value(&check).unwrap();
    assert_eq!(
        value,
        json!({"name": CHECK_NAME, "ok": false, "detail": CHECK_DETAIL, "fix": CHECK_FIX})
    );
}

/// `DevTargetCheckReport::new` derives its own `ok` as the conjunction of
/// every check, and serializes to the exact `ok`/`config_path`/`checks`
/// envelope shape `aidp dev check` already emits.
#[test]
fn report_ok_is_conjunction_of_checks() {
    let checks = vec![
        DevTargetCheck::new(CHECK_NAME, true, CHECK_DETAIL.to_string(), None),
        DevTargetCheck::new("cli_gh", false, "gh NOT found".to_string(), Some(CHECK_FIX)),
    ];
    let report = DevTargetCheckReport::new(CONFIG_PATH.to_string(), checks);
    assert!(!report.ok);
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        value,
        json!({
            "ok": false,
            "config_path": CONFIG_PATH,
            "checks": [
                {"name": CHECK_NAME, "ok": true, "detail": CHECK_DETAIL},
                {"name": "cli_gh", "ok": false, "detail": "gh NOT found", "fix": CHECK_FIX},
            ]
        })
    );
}
