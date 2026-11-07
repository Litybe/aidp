use super::*;

/// A leading front matter block is dropped and the body after it is kept.
#[test]
fn github_body_strips_leading_front_matter() {
    let body = "---\naidp: artifact\ntags: [a]\n---\n# Title\ntext\n";
    assert_eq!(github_body(body), "# Title\ntext\n");
}

/// A body without front matter is unchanged.
#[test]
fn github_body_keeps_a_body_without_front_matter() {
    assert_eq!(github_body("# Title\ntext\n"), "# Title\ntext\n");
}

/// A horizontal rule after the first line is content, not front matter.
#[test]
fn github_body_keeps_a_later_horizontal_rule() {
    let body = "# Title\n---\nmore\n";
    assert_eq!(github_body(body), body);
}

/// An opening fence without a closing one is left alone.
#[test]
fn github_body_keeps_an_unclosed_front_matter() {
    let body = "---\nnot closed\n";
    assert_eq!(github_body(body), body);
}

/// Both wikilink forms become plain text.
#[test]
fn github_body_unlinks_both_wikilink_forms() {
    let body = "See [[Pipelines/svc/plan/reply|plan]] and [[Pipelines/svc/plan/reply.md]].";
    assert_eq!(github_body(body), "See plan and reply.");
}

/// Text without links is unchanged, including a lone `[[`.
#[test]
fn github_body_keeps_text_without_links() {
    let body = "a [b](c) and [[ open\n";
    assert_eq!(github_body(body), body);
}

/// An unclosed `[[` stays as text, and a later link is still converted.
#[test]
fn github_body_converts_links_after_an_unclosed_one() {
    let body = "Range [[1, 2 is open.\nSee [[a/b/plan.md|plan]].";
    assert_eq!(github_body(body), "Range [[1, 2 is open.\nSee plan.");
}
