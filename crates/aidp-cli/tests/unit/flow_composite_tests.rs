use super::*;

const BUG_TYPE: &str = "Bug";
const STORY_TYPE: &str = "Story";
const BUG_KEY: &str = "PIXEL-1299";
const STORY_KEY: &str = "PIXEL-1287";
const OWN_EMAIL: &str = "me@example.com";
const OTHER_EMAIL: &str = "other@example.com";
const SUMMARY: &str = "Retry the payment";

#[test]
fn flow_composite_bug_gets_the_fix_prefix() {
    assert_eq!(branch_name(BUG_KEY, BUG_TYPE), "fix/PIXEL-1299");
}

#[test]
fn flow_composite_story_gets_the_feat_prefix() {
    assert_eq!(branch_name(STORY_KEY, STORY_TYPE), "feat/PIXEL-1287");
}

#[test]
fn flow_composite_unknown_type_gets_the_feat_prefix() {
    assert_eq!(branch_prefix(""), FEATURE_BRANCH_PREFIX);
}

#[test]
fn flow_composite_bug_type_ignores_case() {
    assert_eq!(branch_prefix("bug"), FIX_BRANCH_PREFIX);
}

/// A pull request title is `<type>(<KEY>): <summary>`, `fix` for a bug and `feat` otherwise.
#[test]
fn flow_composite_pr_title_follows_the_ticket_type() {
    let heading = |issue_type: &str, key: &str| TicketHeading {
        issue_type: parse_issue_type(issue_type),
        key: key.to_string(),
        summary: SUMMARY.to_string(),
    };
    assert_eq!(
        pr_title(&heading(BUG_TYPE, BUG_KEY)),
        format!("fix({BUG_KEY}): {SUMMARY}")
    );
    assert_eq!(
        pr_title(&heading(STORY_TYPE, STORY_KEY)),
        format!("feat({STORY_KEY}): {SUMMARY}")
    );
}

#[test]
fn flow_composite_foreign_authors_excludes_the_current_user() {
    let emails = format!("{OWN_EMAIL}\n{OTHER_EMAIL}\n{OTHER_EMAIL}\n\n");
    assert_eq!(foreign_authors(&emails, OWN_EMAIL), vec![OTHER_EMAIL]);
    assert!(foreign_authors(OWN_EMAIL, OWN_EMAIL).is_empty());
}

const GITHUB_NOREPLY_AUTHOR: &str = "123+me@users.noreply.github.com";

#[test]
fn flow_composite_foreign_authors_accepts_commits_committed_by_github() {
    let published = format!("{GITHUB_NOREPLY_AUTHOR}\tGitHub\tnoreply@github.com");
    assert!(foreign_authors(&published, OWN_EMAIL).is_empty());
}

#[test]
fn flow_composite_foreign_authors_still_refuses_a_foreign_committer() {
    let foreign = format!("{OTHER_EMAIL}\tOther\t{OTHER_EMAIL}");
    let spoofed = format!("{OTHER_EMAIL}\tGitHub\t{OTHER_EMAIL}");
    assert_eq!(foreign_authors(&foreign, OWN_EMAIL), vec![OTHER_EMAIL]);
    assert_eq!(foreign_authors(&spoofed, OWN_EMAIL), vec![OTHER_EMAIL]);
}

#[test]
fn flow_composite_missing_pr_body_is_a_usage_error() {
    let error = read_pr_body(Path::new("does-not-exist-body.md")).unwrap_err();
    let exit = error.downcast_ref::<TypedExit>().unwrap();
    assert_eq!(exit.status, ExitStatus::Usage);
}
