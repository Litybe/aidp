//! Global PR comments: conversation comments and non-empty review bodies.

use crate::gh::{gh, repo_slug};
use crate::models::comment_point::CommentPoint;
use crate::models::enums::comment_kind::CommentKind;
use crate::models::global_comment::GlobalComment;
use crate::models::raw_comment::RawComment;
use crate::points::split_points;
use crate::IntegrationError;

/// First line of the summary comment the pipeline posts; such comments are never read back.
pub const SUMMARY_MARKER: &str = "<!-- aidp:pr-comments-summary -->";

const GH_API: &str = "api";
const REPOS_SEGMENT: &str = "repos";
const ISSUES_SEGMENT: &str = "issues";
const PULLS_SEGMENT: &str = "pulls";
const COMMENTS_SEGMENT: &str = "comments";
const REVIEWS_SEGMENT: &str = "reviews";
const BOT_SUFFIX: &str = "[bot]";
const NO_REPOSITORY: &str = "cannot resolve the repository";
const PAGINATE_FLAG: &str = "--paginate";
const SLURP_FLAG: &str = "--slurp";

/// True for `[bot]` accounts and configured bot logins.
pub fn is_bot(author: &str, bot_authors: &[String]) -> bool {
    author.ends_with(BOT_SUFFIX) || bot_authors.iter().any(|bot| bot == author)
}

fn parse_raw(text: &str) -> Result<Vec<RawComment>, IntegrationError> {
    let pages: Vec<Vec<RawComment>> = serde_json::from_str(text)
        .map_err(|error| IntegrationError::Upstream(error.to_string()))?;
    Ok(pages.into_iter().flatten().collect())
}

fn attach_ids(comment_id: u64, points: Vec<CommentPoint>) -> Vec<CommentPoint> {
    points
        .into_iter()
        .map(|point| CommentPoint {
            id: Some(format!("{comment_id}#{}", point.index)),
            ..point
        })
        .collect()
}

fn convert(raw: Vec<RawComment>, kind: CommentKind, bot_authors: &[String]) -> Vec<GlobalComment> {
    raw.into_iter()
        .filter_map(|comment| {
            let body = comment.body.unwrap_or_default();
            if body.trim().is_empty() || body.trim_start().starts_with(SUMMARY_MARKER) {
                return None;
            }
            let author = comment.user.map(|user| user.login).unwrap_or_default();
            Some(GlobalComment {
                id: comment.id,
                bot: is_bot(&author, bot_authors),
                author,
                created_at: comment.created_at,
                url: comment.html_url,
                kind,
                points: attach_ids(comment.id, split_points(&body)),
            })
        })
        .collect()
}

/// Builds the comments from the two raw GitHub payloads (arrays of pages).
pub fn parse_comments(
    conversation_json: &str,
    reviews_json: &str,
    bot_authors: &[String],
) -> Result<Vec<GlobalComment>, IntegrationError> {
    let mut comments = convert(
        parse_raw(conversation_json)?,
        CommentKind::Conversation,
        bot_authors,
    );
    comments.extend(convert(
        parse_raw(reviews_json)?,
        CommentKind::Review,
        bot_authors,
    ));
    Ok(comments)
}

/// Builds the REST path `repos/{slug}/{resource}/{number}/{leaf}`.
fn pr_path(slug: &str, resource: &str, number: u64, leaf: &str) -> String {
    format!("{REPOS_SEGMENT}/{slug}/{resource}/{number}/{leaf}")
}

fn fetch(path: &str) -> Result<String, IntegrationError> {
    gh(&[GH_API, PAGINATE_FLAG, SLURP_FLAG, path], None)
        .map_err(|error| IntegrationError::Upstream(error.to_string()))
}

/// Reads the PR's conversation comments and non-empty review bodies, split into points.
pub fn pr_comments(
    repo: Option<&str>,
    number: u64,
    bot_authors: &[String],
) -> Result<Vec<GlobalComment>, IntegrationError> {
    let slug =
        repo_slug(repo).ok_or_else(|| IntegrationError::NotFound(NO_REPOSITORY.to_string()))?;
    let conversation = fetch(&pr_path(&slug, ISSUES_SEGMENT, number, COMMENTS_SEGMENT))?;
    let reviews = fetch(&pr_path(&slug, PULLS_SEGMENT, number, REVIEWS_SEGMENT))?;
    parse_comments(&conversation, &reviews, bot_authors)
}

#[cfg(test)]
#[path = "../tests/unit/pr_comments_tests.rs"]
mod tests;
