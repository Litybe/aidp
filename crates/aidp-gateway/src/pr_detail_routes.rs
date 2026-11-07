//! Pull-request page routes: reads, comment and review writes, drafts and viewed marks.
//! Every write re-reads the PR state, writes through `pr_writes`, then refreshes caches and the bus.

use crate::models::constants::pr_detail_events::{
    PR_COMMENT_CREATED, PR_COMMENT_DELETED, PR_COMMENT_UPDATED, PR_REVIEW_SUBMITTED,
};
use crate::models::enums::pr_refusal_reason::PrRefusalReason;
use crate::models::enums::ApiStatus;
use crate::models::file_blob::FileBlob;
use crate::models::pr_comment_event::PrCommentEvent;
use crate::models::pr_comment_request::PrCommentRequest;
use crate::models::pr_detail_context::PrDetailContext;
use crate::models::pr_detail_query::PrDetailQuery;
use crate::models::pr_drafts_request::PrDraftsRequest;
use crate::models::pr_file_diff_response::PrFileDiffResponse;
use crate::models::pr_file_view::PrFileView;
use crate::models::pr_files_response::PrFilesResponse;
use crate::models::pr_review_comment_request::PrReviewCommentRequest;
use crate::models::pr_review_request::PrReviewRequest;
use crate::models::pr_review_submitted_event::PrReviewSubmittedEvent;
use crate::models::pr_viewed_request::PrViewedRequest;
use crate::models::pr_write_state::PrWriteState;
use crate::models::viewed_file_state::ViewedFileState;
use crate::models::ApiError;
use crate::prs::{handle_status, PrRouteError};
use aidp_integrations::models::enums::comment_kind::CommentKind;
use aidp_integrations::models::enums::diff_side::DiffSide;
use aidp_integrations::models::enums::review_event::ReviewEvent;
use aidp_integrations::models::pr_file::PrFile;
use aidp_integrations::models::review_draft::ReviewDraft;
use aidp_integrations::pr_detail::{line_in_diff, pr_detail, pr_file_diff, pr_files};
use aidp_integrations::{pr_writes, IntegrationError};
use serde::Serialize;
use serde_json::Value;

const FIRST_PAGE: u32 = 1;
const FILES_PER_PAGE: u32 = 100;
const MAX_FILE_PAGES: u32 = 30;
const NO_HEAD_MESSAGE: &str = "pull request status carries no head sha";
const NOT_YOUR_COMMENT_MESSAGE: &str = "only the author of a comment can change it";
const OWN_PR_MESSAGE: &str = "cannot approve or request changes on your own pull request";
const LINE_OUTSIDE_DIFF_MESSAGE: &str = "the line is outside the diff of the file";
const FILE_NOT_IN_DIFF_MESSAGE: &str = "the file is not part of the pull request diff";
const EMPTY_BODY_MESSAGE: &str = "the comment body is empty";
const EMPTY_REVIEW_MESSAGE: &str = "a comment review needs a summary or at least one draft";
const NO_BLOB_MESSAGE: &str = "the file has no blob sha to mark";
const BAD_COMMENT_ID_MESSAGE: &str = "comment id must be numeric";
const PATH_REQUIRED_MESSAGE: &str = "path is required";

fn refuse(status: ApiStatus, message: &str, reason: Option<PrRefusalReason>) -> PrRouteError {
    PrRouteError {
        error: ApiError {
            status,
            message: message.to_string(),
        },
        reason,
    }
}

fn integration(error: IntegrationError) -> PrRouteError {
    PrRouteError::from(error)
}

fn to_value(payload: impl Serialize) -> Result<Value, PrRouteError> {
    serde_json::to_value(payload)
        .map_err(|error| integration(IntegrationError::Upstream(error.to_string())))
}

/// Parses a comment id from a route segment; anything but digits is a 400.
pub fn parse_comment_id(raw: &str) -> Result<u64, PrRouteError> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(refuse(ApiStatus::BadRequest, BAD_COMMENT_ID_MESSAGE, None));
    }
    raw.parse()
        .map_err(|_| refuse(ApiStatus::BadRequest, BAD_COMMENT_ID_MESSAGE, None))
}

fn read_state(
    ctx: &PrDetailContext<'_>,
    repo: &str,
    number: u64,
) -> Result<PrWriteState, PrRouteError> {
    let status = handle_status(ctx.cache, repo, number, true)?;
    PrWriteState::from_status(&status)
        .ok_or_else(|| refuse(ApiStatus::Upstream, NO_HEAD_MESSAGE, None))
}

fn finish(
    ctx: &PrDetailContext<'_>,
    event_name: &str,
    repo: &str,
    number: u64,
    event: impl Serialize,
    response: impl Serialize,
) -> Result<Value, PrRouteError> {
    ctx.cache.invalidate(ctx.bus, repo, number);
    ctx.squad.invalidate(ctx.bus);
    ctx.bus.emit(event_name, to_value(event)?);
    to_value(response)
}

fn find_file(
    ctx: &PrDetailContext<'_>,
    repo: &str,
    number: u64,
    path: &str,
) -> Result<PrFile, PrRouteError> {
    for page in FIRST_PAGE..=MAX_FILE_PAGES {
        let listed =
            pr_files(ctx.runner, repo, number, page, FILES_PER_PAGE).map_err(integration)?;
        if let Some(file) = listed.files.into_iter().find(|file| file.filename == path) {
            return Ok(file);
        }
        if !listed.has_more {
            break;
        }
    }
    Err(refuse(
        ApiStatus::BadRequest,
        FILE_NOT_IN_DIFF_MESSAGE,
        None,
    ))
}

fn ensure_in_diff(
    ctx: &PrDetailContext<'_>,
    repo: &str,
    number: u64,
    path: &str,
    line: u32,
    side: DiffSide,
    start_line: Option<u32>,
) -> Result<(), PrRouteError> {
    let file = find_file(ctx, repo, number, path)?;
    let patch = match file.patch {
        Some(patch) => patch,
        None => pr_file_diff(ctx.runner, repo, number, path).map_err(integration)?,
    };
    if line_in_diff(&patch, u64::from(line), side, start_line.map(u64::from)) {
        Ok(())
    } else {
        Err(refuse(
            ApiStatus::BadRequest,
            LINE_OUTSIDE_DIFF_MESSAGE,
            None,
        ))
    }
}

fn ensure_draft_in_diff(
    ctx: &PrDetailContext<'_>,
    repo: &str,
    number: u64,
    draft: &ReviewDraft,
) -> Result<(), PrRouteError> {
    let (path, line, side) = (&draft.path, draft.line, draft.side);
    ensure_in_diff(ctx, repo, number, path, line, side, draft.start_line)
}

fn ensure_own_comment(
    ctx: &PrDetailContext<'_>,
    state: &PrWriteState,
    repo: &str,
    kind: CommentKind,
    id: u64,
) -> Result<(), PrRouteError> {
    let author = pr_writes::comment_author(ctx.runner, repo, kind, id).map_err(integration)?;
    match &state.viewer {
        Some(viewer) if viewer.eq_ignore_ascii_case(&author) => Ok(()),
        _ => Err(refuse(ApiStatus::Forbidden, NOT_YOUR_COMMENT_MESSAGE, None)),
    }
}

fn ensure_body(body: &str) -> Result<(), PrRouteError> {
    if body.trim().is_empty() {
        return Err(refuse(ApiStatus::BadRequest, EMPTY_BODY_MESSAGE, None));
    }
    Ok(())
}

/// Reads the whole PR page: description, commits, reviews, comments, threads, reviewers.
pub fn handle_detail(
    ctx: &PrDetailContext<'_>,
    query: &PrDetailQuery,
) -> Result<Value, PrRouteError> {
    let detail = pr_detail(ctx.runner, &query.repo, query.number).map_err(integration)?;
    to_value(detail)
}

/// Reads one page of the PR's files.
pub fn handle_files(
    ctx: &PrDetailContext<'_>,
    query: &PrDetailQuery,
) -> Result<Value, PrRouteError> {
    let page = query.page.unwrap_or(FIRST_PAGE);
    let listed = pr_files(ctx.runner, &query.repo, query.number, page, FILES_PER_PAGE)
        .map_err(integration)?;
    let now: Vec<FileBlob> = listed
        .files
        .iter()
        .filter_map(|file| {
            file.sha.as_ref().map(|sha| FileBlob {
                path: file.filename.clone(),
                sha: sha.clone(),
            })
        })
        .collect();
    let since_last_review = ctx
        .viewed
        .since_last_review(&query.repo, query.number, &now);
    let states = ctx.viewed.viewed_state(&query.repo, query.number, &now);
    let files = listed
        .files
        .into_iter()
        .map(|file| {
            let viewed = states
                .iter()
                .any(|state| state.viewed && state.path == file.filename);
            PrFileView { file, viewed }
        })
        .collect();
    to_value(PrFilesResponse {
        page: listed.page,
        per_page: listed.per_page,
        has_more: listed.has_more,
        files,
        since_last_review,
    })
}

/// Reads the full diff of one file, however large.
pub fn handle_file_diff(
    ctx: &PrDetailContext<'_>,
    query: &PrDetailQuery,
) -> Result<Value, PrRouteError> {
    let path = query
        .path
        .as_deref()
        .ok_or_else(|| refuse(ApiStatus::BadRequest, PATH_REQUIRED_MESSAGE, None))?;
    let diff = pr_file_diff(ctx.runner, &query.repo, query.number, path).map_err(integration)?;
    to_value(PrFileDiffResponse {
        path: path.to_string(),
        diff,
    })
}

/// Posts a conversation comment.
pub fn handle_comment_create(
    ctx: &PrDetailContext<'_>,
    request: PrCommentRequest,
) -> Result<Value, PrRouteError> {
    ensure_body(&request.body)?;
    let outcome =
        pr_writes::comment_create(ctx.runner, &request.repo, request.number, &request.body)
            .map_err(integration)?;
    let event = PrCommentEvent {
        repo: request.repo.clone(),
        number: request.number,
        comment_id: outcome.id,
        kind: CommentKind::Conversation,
    };
    finish(
        ctx,
        PR_COMMENT_CREATED,
        &request.repo,
        request.number,
        event,
        outcome,
    )
}

/// Edits a comment of the viewer; someone else's comment is a 403 with no write.
pub fn handle_comment_edit(
    ctx: &PrDetailContext<'_>,
    kind: CommentKind,
    id: u64,
    request: PrCommentRequest,
) -> Result<Value, PrRouteError> {
    ensure_body(&request.body)?;
    let state = read_state(ctx, &request.repo, request.number)?;
    comment_edit_with_state(ctx, &state, kind, id, request)
}

fn comment_edit_with_state(
    ctx: &PrDetailContext<'_>,
    state: &PrWriteState,
    kind: CommentKind,
    id: u64,
    request: PrCommentRequest,
) -> Result<Value, PrRouteError> {
    ensure_own_comment(ctx, state, &request.repo, kind, id)?;
    let outcome = pr_writes::comment_edit(ctx.runner, &request.repo, kind, id, &request.body)
        .map_err(integration)?;
    let event = PrCommentEvent {
        repo: request.repo.clone(),
        number: request.number,
        comment_id: id,
        kind,
    };
    finish(
        ctx,
        PR_COMMENT_UPDATED,
        &request.repo,
        request.number,
        event,
        outcome,
    )
}

/// Deletes a comment of the viewer; someone else's comment is a 403 with no write.
pub fn handle_comment_delete(
    ctx: &PrDetailContext<'_>,
    kind: CommentKind,
    id: u64,
    query: &PrDetailQuery,
) -> Result<Value, PrRouteError> {
    let state = read_state(ctx, &query.repo, query.number)?;
    comment_delete_with_state(ctx, &state, kind, id, query)
}

fn comment_delete_with_state(
    ctx: &PrDetailContext<'_>,
    state: &PrWriteState,
    kind: CommentKind,
    id: u64,
    query: &PrDetailQuery,
) -> Result<Value, PrRouteError> {
    ensure_own_comment(ctx, state, &query.repo, kind, id)?;
    pr_writes::comment_delete(ctx.runner, &query.repo, kind, id).map_err(integration)?;
    let event = PrCommentEvent {
        repo: query.repo.clone(),
        number: query.number,
        comment_id: id,
        kind,
    };
    finish(
        ctx,
        PR_COMMENT_DELETED,
        &query.repo,
        query.number,
        &event,
        &event,
    )
}

/// Posts an inline comment on a changed line; a line outside the diff is a 400 with no write.
pub fn handle_review_comment_create(
    ctx: &PrDetailContext<'_>,
    request: PrReviewCommentRequest,
) -> Result<Value, PrRouteError> {
    ensure_body(&request.body)?;
    let state = read_state(ctx, &request.repo, request.number)?;
    review_comment_create_with_state(ctx, &state, request)
}

fn review_comment_create_with_state(
    ctx: &PrDetailContext<'_>,
    state: &PrWriteState,
    request: PrReviewCommentRequest,
) -> Result<Value, PrRouteError> {
    let (repo, number, path) = (&request.repo, request.number, &request.path);
    ensure_in_diff(
        ctx,
        repo,
        number,
        path,
        request.line,
        request.side,
        request.start_line,
    )?;
    let draft = ReviewDraft {
        id: format!("{}:{}", request.path, request.line),
        path: request.path.clone(),
        line: request.line,
        side: request.side,
        start_line: request.start_line,
        start_side: request.start_side,
        body: request.body.clone(),
    };
    let outcome = pr_writes::inline_comment_create(
        ctx.runner,
        &request.repo,
        request.number,
        &state.head_sha,
        &draft,
    )
    .map_err(integration)?;
    let event = PrCommentEvent {
        repo: request.repo.clone(),
        number: request.number,
        comment_id: outcome.id,
        kind: CommentKind::Review,
    };
    finish(
        ctx,
        PR_COMMENT_CREATED,
        &request.repo,
        request.number,
        event,
        outcome,
    )
}

/// Reads the local drafts of a PR.
pub fn handle_drafts_get(
    ctx: &PrDetailContext<'_>,
    query: &PrDetailQuery,
) -> Result<Value, PrRouteError> {
    to_value(ctx.drafts.get(&query.repo, query.number))
}

/// Replaces the local drafts of a PR.
pub fn handle_drafts_put(
    ctx: &PrDetailContext<'_>,
    request: PrDraftsRequest,
) -> Result<Value, PrRouteError> {
    ctx.drafts
        .put(&request.repo, request.number, request.drafts.clone())
        .map_err(|error| integration(IntegrationError::Upstream(error.to_string())))?;
    to_value(request.drafts)
}

fn ensure_review_allowed(
    state: &PrWriteState,
    request: &PrReviewRequest,
    draft_count: usize,
) -> Result<(), PrRouteError> {
    let judges = matches!(
        request.event,
        ReviewEvent::Approve | ReviewEvent::RequestChanges
    );
    if judges && state.is_own_pr() {
        return Err(refuse(
            ApiStatus::Conflict,
            OWN_PR_MESSAGE,
            Some(PrRefusalReason::OwnPr),
        ));
    }
    if request.event == ReviewEvent::Comment && request.body.trim().is_empty() && draft_count == 0 {
        return Err(refuse(ApiStatus::BadRequest, EMPTY_REVIEW_MESSAGE, None));
    }
    Ok(())
}

/// Submits the stored drafts as one review; the drafts are cleared only when GitHub accepted it.
pub fn handle_review_submit(
    ctx: &PrDetailContext<'_>,
    request: PrReviewRequest,
) -> Result<Value, PrRouteError> {
    let state = read_state(ctx, &request.repo, request.number)?;
    review_submit_with_state(ctx, &state, request)
}

fn review_submit_with_state(
    ctx: &PrDetailContext<'_>,
    state: &PrWriteState,
    request: PrReviewRequest,
) -> Result<Value, PrRouteError> {
    let drafts = ctx.drafts.get(&request.repo, request.number);
    ensure_review_allowed(state, &request, drafts.len())?;
    for draft in &drafts {
        ensure_draft_in_diff(ctx, &request.repo, request.number, draft)?;
    }
    let outcome = pr_writes::review_submit(
        ctx.runner,
        &request.repo,
        request.number,
        &state.head_sha,
        request.event,
        &request.body,
        &drafts,
    )
    .map_err(integration)?;
    if let Err(error) = ctx.drafts.clear(&request.repo, request.number) {
        tracing::warn!(error = %error, "review submitted but its drafts could not be cleared");
    }
    let event = PrReviewSubmittedEvent {
        repo: request.repo.clone(),
        number: request.number,
        review_id: outcome.id,
        event: request.event,
        comment_count: outcome.comment_count,
    };
    finish(
        ctx,
        PR_REVIEW_SUBMITTED,
        &request.repo,
        request.number,
        event,
        outcome,
    )
}

/// Marks or unmarks a file as viewed, keyed on its current blob sha.
pub fn handle_viewed(
    ctx: &PrDetailContext<'_>,
    request: PrViewedRequest,
) -> Result<Value, PrRouteError> {
    let store_error =
        |error: anyhow::Error| integration(IntegrationError::Upstream(error.to_string()));
    if request.viewed {
        let file = find_file(ctx, &request.repo, request.number, &request.path)?;
        let sha = file
            .sha
            .ok_or_else(|| refuse(ApiStatus::BadRequest, NO_BLOB_MESSAGE, None))?;
        ctx.viewed
            .mark(&request.repo, request.number, &request.path, &sha)
            .map_err(store_error)?;
    } else {
        ctx.viewed
            .unmark(&request.repo, request.number, &request.path)
            .map_err(store_error)?;
    }
    to_value(ViewedFileState {
        path: request.path,
        viewed: request.viewed,
    })
}

#[cfg(test)]
#[path = "tests/units/pr_detail_routes_tests.rs"]
mod pr_detail_routes_tests;
