use super::*;

const KNOWN_STATUSES: [(&str, TaskStatus); 5] = [
    ("pending", TaskStatus::Pending),
    ("success", TaskStatus::Success),
    ("failed", TaskStatus::Failed),
    ("skipped", TaskStatus::Skipped),
    ("cancelled", TaskStatus::Cancelled),
];
const UNKNOWN_STATUS: &str = "weird";
const SLASH_PATH: &str = "src/foo.cs";
const BACKSLASH_PATH: &str = "src\\foo.cs";
const DOT_SLASH_PATH: &str = "./src/foo.cs";
const REPEATED_DOT_SLASH_PATH: &str = "./././src/foo.cs";
const MIXED_CASE_PATH: &str = "Src/Foo.CS";
const PADDED_PATH: &str = "  /src/foo.cs/  ";

/// Each known status serializes to its lowercase wire string and parses back.
#[test]
fn known_task_statuses_round_trip_as_wire_strings() {
    for (wire, status) in KNOWN_STATUSES {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{wire}\""));
        let parsed: TaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
        assert_eq!(status.to_string(), wire);
    }
}

/// A status the runtime writes that is not a known variant survives verbatim.
#[test]
fn unknown_task_status_round_trips_verbatim() {
    let parsed: TaskStatus = serde_json::from_str(&format!("\"{UNKNOWN_STATUS}\"")).unwrap();
    assert_eq!(parsed, TaskStatus::Other(UNKNOWN_STATUS.to_string()));
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        format!("\"{UNKNOWN_STATUS}\"")
    );
}

/// A freshly defaulted status is pending.
#[test]
fn default_task_status_is_pending() {
    assert_eq!(TaskStatus::default(), TaskStatus::Pending);
}

/// Backslash and slash spellings normalize to the same path.
#[test]
fn normalize_plan_path_unifies_separators() {
    assert_eq!(normalize_plan_path(BACKSLASH_PATH), SLASH_PATH);
    assert_eq!(normalize_plan_path(SLASH_PATH), SLASH_PATH);
}

/// Leading `./` prefixes are dropped, however many there are.
#[test]
fn normalize_plan_path_drops_leading_dot_slash() {
    assert_eq!(normalize_plan_path(DOT_SLASH_PATH), SLASH_PATH);
    assert_eq!(normalize_plan_path(REPEATED_DOT_SLASH_PATH), SLASH_PATH);
}

/// Paths compare case-insensitively.
#[test]
fn normalize_plan_path_lowercases() {
    assert_eq!(normalize_plan_path(MIXED_CASE_PATH), SLASH_PATH);
}

/// Surrounding whitespace and edge slashes are trimmed.
#[test]
fn normalize_plan_path_trims_whitespace_and_edge_slashes() {
    assert_eq!(normalize_plan_path(PADDED_PATH), SLASH_PATH);
}
