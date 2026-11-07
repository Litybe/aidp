use super::*;

const PATCH: &str = "@@ -10,3 +10,4 @@\n ctx10\n-del11\n+add11\n+add12\n ctx12\n@@ -40,2 +41,2 @@\n c40\n c41\n\\ No newline at end of file";

#[test]
fn diff_hunks_collect_lines_per_side() {
    let hunks = parse_hunks(PATCH);
    assert_eq!(hunks.len(), 2);
    assert_eq!(hunks[0].right_lines, vec![10, 11, 12, 13]);
    assert_eq!(hunks[0].left_lines, vec![10, 11, 12]);
    assert_eq!(hunks[1].right_lines, vec![41, 42]);
}

#[test]
fn diff_hunks_accept_added_line_on_right() {
    assert!(anchor_in_hunks(
        &parse_hunks(PATCH),
        12,
        DiffSide::Right,
        None
    ));
}

#[test]
fn diff_hunks_accept_deleted_line_on_left_only() {
    let hunks = parse_hunks(PATCH);
    assert!(anchor_in_hunks(&hunks, 11, DiffSide::Left, None));
    assert!(!anchor_in_hunks(&hunks, 13, DiffSide::Left, None));
}

#[test]
fn diff_hunks_reject_line_outside_every_hunk() {
    assert!(!anchor_in_hunks(
        &parse_hunks(PATCH),
        30,
        DiffSide::Right,
        None
    ));
}

#[test]
fn diff_hunks_range_must_stay_in_one_hunk() {
    let hunks = parse_hunks(PATCH);
    assert!(anchor_in_hunks(&hunks, 12, DiffSide::Right, Some(10)));
    assert!(!anchor_in_hunks(&hunks, 41, DiffSide::Right, Some(12)));
    assert!(!anchor_in_hunks(&hunks, 10, DiffSide::Right, Some(12)));
}

#[test]
fn diff_hunks_of_empty_patch_hold_nothing() {
    assert!(parse_hunks("").is_empty());
}
