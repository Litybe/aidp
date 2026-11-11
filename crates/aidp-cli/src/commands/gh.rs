use super::CommandOutput;
use crate::args::gh::Op;
use crate::ctx::Ctx;
use crate::models::squad_pr_row::SquadPrRow;
use crate::models::squad_prs_envelope::SquadPrsEnvelope;
use crate::models::squad_repos_envelope::SquadReposEnvelope;
use aidp_core::{ExitStatus, Format, TypedExit};
use aidp_gateway::models::enums::api_status::ApiStatus;
use aidp_gateway::models::gateway_http_error::GatewayHttpError;
use aidp_gateway::models::gateway_unreachable::GatewayUnreachable;
use aidp_gateway::models::pr_fix_request::PrFixRequest;
use aidp_integrations::models::enums::bots_filter::BotsFilter;
use aidp_integrations::models::enums::comment_kind::CommentKind;
use aidp_integrations::models::enums::review_event::ReviewEvent;
use aidp_integrations::models::pr_created::PrCreated;
use aidp_integrations::models::review_draft::ReviewDraft;
use aidp_integrations::models::squad_config::SquadConfig;
use aidp_integrations::models::squad_degraded::SquadDegraded;
use aidp_integrations::models::thread_result::ThreadResult;
use aidp_integrations::{
    gh, github_labels, pr, pr_actions, pr_comments, pr_detail, pr_writes, review_post, squad,
    IntegrationError,
};
use serde_json::Value;
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

const STDIN_MARKER: &str = "-";
const NUMBER_KEY: &str = "number";
const REPO_KEY: &str = "repo";
const HEAD_SHA_KEY: &str = "head_sha";
const NO_HEAD_SHA: &str = "head commit of the pull request";
const NO_FALLBACK_REPOS: &[String] = &[];

/// Maps a daemon failure to its exit status: 403 and 409 are policy refusals.
pub(super) fn daemon_failure(error: anyhow::Error) -> anyhow::Error {
    let message = format!("{error}");
    let status = exit_status_for(&error);
    TypedExit { status, message }.into()
}

/// The exit status of a typed daemon error; any other error is internal.
fn exit_status_for(error: &anyhow::Error) -> ExitStatus {
    if let Some(http_error) = error.downcast_ref::<GatewayHttpError>() {
        return match ApiStatus::from_status(http_error.status) {
            ApiStatus::Forbidden | ApiStatus::Conflict => ExitStatus::PolicyRefusal,
            ApiStatus::NotFound => ExitStatus::NotFound,
            ApiStatus::BadRequest => ExitStatus::Usage,
            _ => ExitStatus::Internal,
        };
    }
    if error.downcast_ref::<GatewayUnreachable>().is_some() {
        return ExitStatus::UpstreamFailure;
    }
    ExitStatus::Internal
}

/// Where marks and the repos cache live: the gateway state directory.
fn state_dir() -> PathBuf {
    aidp_gateway::paths::state_dir().unwrap_or_default()
}

fn upstream_failure(message: String) -> anyhow::Error {
    aidp_core::Error::Upstream(message).into()
}

fn squad_repos_output(ctx: &Ctx) -> (SquadConfig, Vec<String>, Vec<SquadDegraded>) {
    let config = squad::load_config();
    let (repos, degraded) = squad::squad_repos(
        &config,
        NO_FALLBACK_REPOS,
        &state_dir(),
        ctx.clock.as_ref(),
        &squad::OsGhRunner,
    );
    (config, repos, degraded)
}

fn squad_repos_command(ctx: &Ctx) -> anyhow::Result<CommandOutput> {
    let (_, repos, degraded) = squad_repos_output(ctx);
    Ok(compact(serde_json::to_value(SquadReposEnvelope {
        repos,
        degraded,
    })?))
}

/// Whether `pr_author` passes the optional `--author` filter, ignoring case.
fn author_matches(filter: Option<&str>, pr_author: &str) -> bool {
    filter.is_none_or(|login| pr_author.eq_ignore_ascii_case(login))
}

fn squad_prs_command(
    ctx: &Ctx,
    repo: Option<String>,
    author: Option<String>,
    bots: BotsFilter,
) -> anyhow::Result<CommandOutput> {
    let runner = squad::OsGhRunner;
    let me = squad::me_login(&runner).map_err(upstream_failure)?;
    let (config, mut repos, mut degraded) = squad_repos_output(ctx);
    if let Some(only) = &repo {
        repos.retain(|candidate| candidate.eq_ignore_ascii_case(only));
    }
    let fetched = squad::squad_prs(&repos, &me, &runner, ctx.clock.as_ref(), None);
    degraded.extend(fetched.degraded);
    let marks =
        aidp_gateway::pr_marks::read_marks(&state_dir().join(aidp_gateway::pr_marks::MARKS_FILE));
    let prs = fetched
        .prs
        .into_iter()
        .filter(|pr| author_matches(author.as_deref(), &pr.author))
        .filter(|pr| squad::accepts(bots, pr, &config))
        .filter_map(|pr| {
            let mark = marks.get(&squad::mark_key(&pr.repo, pr.number));
            let lanes = squad::classify(&pr, &me, mark);
            if lanes.is_empty() {
                return None;
            }
            Some(SquadPrRow {
                lanes,
                my_review_state: squad::my_review_state(&pr, &me),
                pr,
            })
        })
        .collect();
    Ok(compact(serde_json::to_value(SquadPrsEnvelope {
        fetched_at: fetched.fetched_at,
        me: fetched.me,
        repos: fetched.repos,
        prs,
        degraded,
    })?))
}

/// Thread ids already handed to a run for this PR; empty when nothing is stored.
fn handed_threads(pr: Option<&str>, repo: Option<&str>) -> HashSet<String> {
    let Some(resolved) = gh::pr_resolve(pr, repo) else {
        return HashSet::new();
    };
    let number = resolved.get(NUMBER_KEY).and_then(Value::as_u64);
    let slug = resolved.get(REPO_KEY).and_then(Value::as_str);
    let (Some(number), Some(slug), Ok(dir)) = (number, slug, aidp_gateway::paths::state_dir())
    else {
        return HashSet::new();
    };
    aidp_gateway::pr_comment_state::load(&dir, slug, number)
        .handed
        .into_iter()
        .collect()
}

fn review_threads_command(
    pr: Option<&str>,
    repo: Option<&str>,
    include_resolved: bool,
) -> anyhow::Result<CommandOutput> {
    let handed = handed_threads(pr, repo);
    Ok(compact(gh::pr_review_threads_with_handed(
        pr,
        repo,
        include_resolved,
        &handed,
    )?))
}

fn pr_comments_command(pr: u64, repo: Option<&str>) -> anyhow::Result<CommandOutput> {
    let comments = pr_comments::pr_comments(repo, pr, &[])
        .map_err(|error| upstream_failure(error.to_string()))?;
    Ok(compact(serde_json::to_value(comments)?))
}

fn read_body(body_file: &str) -> anyhow::Result<String> {
    if body_file == STDIN_MARKER {
        let mut body = String::new();
        std::io::stdin().read_to_string(&mut body)?;
        return Ok(body);
    }
    Ok(std::fs::read_to_string(body_file)?)
}

/// Head sha given by the caller, else the one GitHub reports for the PR.
fn head_sha_of(given: Option<String>, repo: &str, number: u64) -> anyhow::Result<String> {
    if let Some(sha) = given {
        return Ok(sha);
    }
    let number_text = number.to_string();
    gh::pr_resolve(Some(&number_text), Some(repo))
        .and_then(|resolved| {
            resolved
                .get(HEAD_SHA_KEY)
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| IntegrationError::NotFound(NO_HEAD_SHA.to_string()).into())
}

fn deleted(id: u64) -> anyhow::Result<CommandOutput> {
    Ok(compact(serde_json::to_value(ThreadResult {
        id: id.to_string(),
        ok: true,
        error: None,
    })?))
}

fn review_comment_create_command(
    ctx: &Ctx,
    repo: &str,
    number: u64,
    draft: ReviewDraft,
    head_sha: Option<String>,
) -> anyhow::Result<CommandOutput> {
    let sha = head_sha_of(head_sha, repo, number)?;
    let outcome =
        pr_writes::inline_comment_create(ctx.runner.as_ref(), repo, number, &sha, &draft)?;
    Ok(compact(serde_json::to_value(outcome)?))
}

fn review_submit_command(
    ctx: &Ctx,
    repo: &str,
    number: u64,
    event: ReviewEvent,
    body_file: &str,
    comments_file: &str,
    head_sha: Option<String>,
) -> anyhow::Result<CommandOutput> {
    let body = read_body(body_file)?;
    let drafts: Vec<ReviewDraft> = serde_json::from_str(&std::fs::read_to_string(comments_file)?)?;
    let sha = head_sha_of(head_sha, repo, number)?;
    let outcome = pr_writes::review_submit(
        ctx.runner.as_ref(),
        repo,
        number,
        &sha,
        event,
        &body,
        &drafts,
    )?;
    Ok(compact(serde_json::to_value(outcome)?))
}

fn compact(value: Value) -> CommandOutput {
    CommandOutput::json(Some(Format::JsonCompact), value)
}

/// Text the command used to write without a final newline of its own.
/// `CommandOutput` adds one, so a single trailing newline is dropped here.
fn unterminated(text: String) -> CommandOutput {
    let body = text.strip_suffix('\n').unwrap_or(&text);
    CommandOutput::text(body)
}

pub fn run(ctx: &Ctx, op: Op) -> anyhow::Result<CommandOutput> {
    Ok(match op {
        Op::PrCreate {
            title,
            body,
            base,
            draft,
        } => compact(serde_json::to_value(PrCreated {
            url: gh::pr_create_draft(&title, &body, base.as_deref(), draft)?,
        })?),
        Op::PrReady { pr, repo, undo } => compact(serde_json::to_value(pr_actions::pr_ready(
            pr,
            repo.as_deref(),
            undo,
        )?)?),
        Op::PrUpdateBranch { pr, repo } => compact(serde_json::to_value(
            pr_actions::pr_update_branch(pr, repo.as_deref())?,
        )?),
        Op::PrMerge {
            pr,
            expected_head_sha,
            repo,
            keep_branch,
            report_conflict,
        } => {
            let merge = if report_conflict {
                pr_actions::pr_merge_reporting_conflict
            } else {
                pr_actions::pr_merge
            };
            compact(serde_json::to_value(merge(
                pr,
                repo.as_deref(),
                &expected_head_sha,
                keep_branch,
            )?)?)
        }
        Op::LabelsMigrate {
            repo,
            apply,
            prefix,
        } => {
            let runner: std::sync::Arc<dyn aidp_core::process_runner::ProcessRunner> =
                std::sync::Arc::new(aidp_core::process_runner::OsProcessRunner);
            let planned = github_labels::plan(runner.as_ref(), &repo, &prefix)?;
            let outcome = if apply {
                github_labels::apply(runner, planned)
            } else {
                planned
            };
            compact(serde_json::to_value(outcome)?)
        }
        Op::ThreadReply {
            thread_id,
            body_file,
        } => {
            let body = read_body(&body_file)?;
            pr_actions::thread_reply(&thread_id, &body)?;
            compact(serde_json::to_value(ThreadResult {
                id: thread_id,
                ok: true,
                error: None,
            })?)
        }
        Op::PrFix {
            repo,
            number,
            threads,
            points,
            note_file,
            model,
            effort,
            url,
        } => {
            let note = note_file.as_deref().map(read_body).transpose()?;
            let request = PrFixRequest {
                repo,
                number,
                threads,
                points,
                note,
                model,
                effort,
            };
            let answer = aidp_gateway::client::pr_fix(&url, &request).map_err(daemon_failure)?;
            compact(answer)
        }
        Op::ThreadResolve {
            thread_ids,
            unresolve,
        } => {
            let results = pr_actions::thread_resolve(&thread_ids, unresolve);
            let any_failed = results.iter().any(|result| !result.ok);
            let output = compact(serde_json::to_value(results)?);
            if any_failed {
                output.with_exit(ExitStatus::NegativeVerdict)
            } else {
                output
            }
        }
        Op::PrView { pr } => unterminated(gh::pr_view(&pr)?),
        Op::PrDiff { pr } => unterminated(gh::pr_diff(&pr)?),
        Op::PrResolve { pr, repo } => {
            compact(gh::pr_resolve_or_missing(pr.as_deref(), repo.as_deref()))
        }
        Op::PrChecks { pr, repo } => compact(gh::pr_checks(pr.as_deref(), repo.as_deref())?),
        Op::PrStatus { pr: number, repo } => compact(serde_json::to_value(pr::pr_status(
            repo.as_deref(),
            number,
            None,
        )?)?),
        Op::SquadRepos => squad_repos_command(ctx)?,
        Op::SquadPrs { repo, author, bots } => squad_prs_command(ctx, repo, author, bots.into())?,
        Op::PrReviewThreads { pr, repo, all } => {
            review_threads_command(pr.as_deref(), repo.as_deref(), all)?
        }
        Op::PrComments { pr, repo } => pr_comments_command(pr, repo.as_deref())?,
        Op::PrDetail { repo, number } => compact(serde_json::to_value(pr_detail::pr_detail(
            ctx.runner.as_ref(),
            &repo,
            number,
        )?)?),
        Op::PrFiles {
            repo,
            number,
            page,
            per_page,
        } => compact(serde_json::to_value(pr_detail::pr_files(
            ctx.runner.as_ref(),
            &repo,
            number,
            page,
            per_page,
        )?)?),
        Op::PrFileDiff { repo, number, path } => unterminated(pr_detail::pr_file_diff(
            ctx.runner.as_ref(),
            &repo,
            number,
            &path,
        )?),
        Op::CommentCreate {
            repo,
            number,
            body_file,
        }
        | Op::PrComment {
            repo,
            number,
            body_file,
        } => compact(serde_json::to_value(pr_writes::comment_create(
            ctx.runner.as_ref(),
            &repo,
            number,
            &read_body(&body_file)?,
        )?)?),
        Op::CommentEdit {
            repo,
            id,
            body_file,
        } => compact(serde_json::to_value(pr_writes::comment_edit(
            ctx.runner.as_ref(),
            &repo,
            CommentKind::Conversation,
            id,
            &read_body(&body_file)?,
        )?)?),
        Op::CommentDelete { repo, id } => {
            pr_writes::comment_delete(ctx.runner.as_ref(), &repo, CommentKind::Conversation, id)?;
            deleted(id)?
        }
        Op::ReviewCommentCreate {
            repo,
            number,
            path,
            line,
            side,
            start_line,
            start_side,
            head_sha,
            body_file,
        } => {
            let draft = ReviewDraft {
                id: String::new(),
                path,
                line,
                side: side.into(),
                start_line,
                start_side: start_side.map(Into::into),
                body: read_body(&body_file)?,
            };
            review_comment_create_command(ctx, &repo, number, draft, head_sha)?
        }
        Op::ReviewCommentEdit {
            repo,
            id,
            body_file,
        } => compact(serde_json::to_value(pr_writes::comment_edit(
            ctx.runner.as_ref(),
            &repo,
            CommentKind::Review,
            id,
            &read_body(&body_file)?,
        )?)?),
        Op::ReviewCommentDelete { repo, id } => {
            pr_writes::comment_delete(ctx.runner.as_ref(), &repo, CommentKind::Review, id)?;
            deleted(id)?
        }
        Op::ReviewSubmit {
            repo,
            number,
            event,
            body_file,
            comments_file,
            head_sha,
        } => review_submit_command(
            ctx,
            &repo,
            number,
            event.into(),
            &body_file,
            &comments_file,
            head_sha,
        )?,
        Op::ReviewPost {
            repo,
            number,
            findings,
            pass,
            run,
        } => compact(serde_json::to_value(review_post::review_post(
            ctx.runner.as_ref(),
            &repo,
            number,
            &findings,
            pass,
            &run,
        )?)?),
        Op::ThreadReopen {
            thread_id,
            body_file,
        } => compact(serde_json::to_value(review_post::thread_reopen(
            &thread_id,
            &read_body(&body_file)?,
        )?)?),
    })
}

#[cfg(test)]
#[path = "../../tests/unit/gh_tests.rs"]
mod tests;
