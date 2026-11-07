use super::*;

const SONAR: &str = include_str!("fixtures/comment_sonarqube_report.md");
const BULLET_SUMMARY: &str = include_str!("fixtures/comment_bullet_summary.md");
const BULLETS_CODE_BLOCK: &str = include_str!("fixtures/comment_bullets_code_block.md");
const HEADINGS: &str = include_str!("fixtures/comment_headings.md");

#[test]
fn two_headings_make_two_points_with_titles() {
    let points = split_points(HEADINGS);
    assert_eq!(points.len(), 2);
    assert_eq!(
        points[0].title.as_deref(),
        Some("🔴 1 — Missing validation")
    );
    assert_eq!(points[1].title.as_deref(), Some("🟠 2 — Naming"));
    assert_eq!(points[1].index, 1);
}

#[test]
fn bullets_with_code_block_keep_the_block_intact() {
    let points = split_points(BULLETS_CODE_BLOCK);
    assert_eq!(points.len(), 2);
    assert!(points[0].body.contains("## not a heading"));
    assert!(points[0].body.contains("- not an item"));
    assert!(points[1].body.contains("unused import"));
}

#[test]
fn sonar_report_splits_on_headings() {
    assert_eq!(split_points(SONAR).len(), 2);
}

#[test]
fn bullet_summary_keeps_preamble_and_one_point_per_item() {
    let points = split_points(BULLET_SUMMARY);
    assert_eq!(points.len(), 4);
    assert!(points[0].body.starts_with("Overall"));
}

#[test]
fn plain_text_is_one_point() {
    let points = split_points("just a note");
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].body, "just a note");
}

#[test]
fn h1_and_h5_are_not_split_headings() {
    assert_eq!(split_points("# t\n\n##### deep\n\ntext").len(), 1);
}
