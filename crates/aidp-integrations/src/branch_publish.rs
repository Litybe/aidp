//! Publishes a ticket branch without `git push`: the committed worktree becomes one commit that
//! GitHub creates and signs through its GraphQL API, then the worktree lands on that commit.

use crate::models::commit_message::CommitMessage;
use crate::models::committable_branch::CommittableBranch;
use crate::models::constants::branch_publish::*;
use crate::models::constants::gh_flow::{API_COMMAND, INPUT_FLAG, METHOD_FLAG, POST_METHOD};
use crate::models::constants::process::{GH_PROGRAM, GIT_PROGRAM, REV_PARSE};
use crate::models::create_commit_on_branch_input::CreateCommitOnBranchInput;
use crate::models::create_ref_input::CreateRefInput;
use crate::models::file_addition::FileAddition;
use crate::models::file_changes::FileChanges;
use crate::models::file_deletion::FileDeletion;
use crate::models::flow_push_report::FlowPushReport;
use crate::models::github_repository::GithubRepository;
use crate::models::graphql_input::GraphqlInput;
use crate::models::graphql_request::GraphqlRequest;
use crate::models::publish_context::PublishContext;
use crate::IntegrationError;
use aidp_core::models::enums::ExitStatus;
use aidp_core::models::TypedExit;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec};
use base64::Engine as _;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::process::Output;

/// [`publish`] with a private scratch directory for the request bodies, removed afterwards.
pub fn publish_in(
    runner: &dyn ProcessRunner,
    workspace: &Path,
    expected_branch: Option<&str>,
) -> anyhow::Result<FlowPushReport> {
    let scratch = tempfile::tempdir()?;
    let context = PublishContext {
        runner,
        workspace,
        scratch: scratch.path(),
    };
    publish(&context, expected_branch)
}

/// Publishes the committed state of the worktree's branch and lands the worktree on the result.
/// Refuses a detached HEAD, another branch than `expected_branch`, and the default branch.
pub fn publish(
    context: &PublishContext<'_>,
    expected_branch: Option<&str>,
) -> anyhow::Result<FlowPushReport> {
    let branch = current_branch(context)?;
    if let Some(expected) = expected_branch.filter(|expected| *expected != branch) {
        return Err(conflict(format!("{WRONG_BRANCH} `{expected}`: `{branch}`")));
    }
    let repository = origin_repository(context)?;
    let (repository_id, default_branch) = repository_target(context, &repository)?;
    if branch == default_branch {
        return Err(conflict(format!("{DEFAULT_BRANCH_REFUSED} `{branch}`")));
    }
    git(context, &[FETCH, ORIGIN, PRUNE_FLAG])?;
    let remote_ref = format!("{REMOTE_REF_PREFIX}{branch}");
    let remote_head = git_optional(context, &[REV_PARSE, VERIFY_FLAG, QUIET_FLAG, &remote_ref])?;
    let expected_head = match &remote_head {
        Some(head) => head.clone(),
        None => {
            let default_ref = format!("{REMOTE_REF_PREFIX}{default_branch}");
            git_text(context, &[MERGE_BASE, HEAD, &default_ref])?
        }
    };
    let changes = collect_changes(context, &expected_head)?;
    if changes.is_empty() {
        if remote_head.is_none() {
            return Err(nothing_to_publish(&default_branch));
        }
        git(context, &[RESET, SOFT_FLAG, &remote_ref])?;
        return Ok(FlowPushReport {
            branch,
            sha: expected_head,
            changed: false,
        });
    }
    if remote_head.is_some()
        && !git_succeeds(
            context,
            &[MERGE_BASE, IS_ANCESTOR_FLAG, &expected_head, HEAD],
        )?
    {
        return Err(conflict(format!(
            "{ORIGIN}{REMOTE_BRANCH_SEPARATOR}{branch} {REMOTE_MOVED}"
        )));
    }
    let last_message = git_text(context, &[LOG, LAST_COMMIT_FLAG, MESSAGE_FORMAT, HEAD])?;
    if remote_head.is_none() {
        create_ref(context, &repository_id, &branch, &expected_head)?;
    }
    let request = create_commit_request(
        &repository,
        &branch,
        &expected_head,
        commit_message(&last_message, &branch),
        changes,
    );
    let reply = graphql(context, CREATE_COMMIT_REQUEST_FILE, &request)?;
    let sha = text_at(&reply, COMMIT_OID_POINTER, NO_COMMIT_OID)?;
    git(context, &[FETCH, ORIGIN, &branch])?;
    git(context, &[RESET, SOFT_FLAG, &sha])?;
    Ok(FlowPushReport {
        branch,
        sha,
        changed: true,
    })
}

/// The `owner/name` an `origin` URL names, HTTPS (with or without a credential) or SSH.
pub fn parse_owner_repo(remote_url: &str) -> Option<GithubRepository> {
    let trimmed = remote_url.trim();
    let trimmed = trimmed.strip_suffix(GIT_SUFFIX).unwrap_or(trimmed);
    let (rest, name) = trimmed.rsplit_once(URL_SEPARATORS)?;
    let (_, owner) = rest.rsplit_once(URL_SEPARATORS)?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(GithubRepository {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// The files `HEAD` changes against `base`: whole committed contents, base64 encoded, and deletions.
/// A rename is a deletion plus an addition; a submodule path has no blob and is left out.
pub fn collect_changes(context: &PublishContext<'_>, base: &str) -> anyhow::Result<FileChanges> {
    let listing = git_bytes(
        context,
        &[
            CONFIG_FLAG,
            LITERAL_PATHS,
            DIFF,
            NO_RENAMES_FLAG,
            NAME_STATUS_FLAG,
            NUL_FLAG,
            base,
            HEAD,
        ],
    )?;
    let mut changes = FileChanges::default();
    let mut payload = 0usize;
    for (status, path) in name_status_entries(&listing) {
        if status.starts_with(DELETED_STATUS) {
            changes.deletions.push(FileDeletion { path });
            continue;
        }
        let object = format!("{HEAD}{OBJECT_SEPARATOR}{path}");
        let size = git_optional(context, &[CAT_FILE, SIZE_FLAG, &object])?
            .and_then(|text| text.parse::<u64>().ok());
        let Some(size) = size else {
            continue;
        };
        if size > MAX_FILE_BYTES {
            return Err(conflict(format!(
                "`{path}` ({} MiB) {FILE_TOO_LARGE}",
                size / BYTES_PER_MEBIBYTE
            )));
        }
        let contents = base64::engine::general_purpose::STANDARD
            .encode(git_bytes(context, &[CAT_FILE, BLOB, &object])?);
        payload += contents.len();
        if payload > MAX_TOTAL_BASE64 {
            return Err(conflict(format!("{PAYLOAD_TOO_LARGE} `{path}`")));
        }
        changes.additions.push(FileAddition { path, contents });
    }
    Ok(changes)
}

/// The `(status, path)` records of a `git diff --name-status -z` listing.
pub fn name_status_entries(listing: &[u8]) -> Vec<(String, String)> {
    let tokens: Vec<String> = listing
        .split(|byte| *byte == NUL)
        .filter(|token| !token.is_empty())
        .map(|token| String::from_utf8_lossy(token).to_string())
        .collect();
    tokens
        .chunks_exact(2)
        .map(|record| (record[0].clone(), record[1].clone()))
        .collect()
}

/// The commit message of a publish: the last local message, or a default naming the branch.
pub fn commit_message(text: &str, branch: &str) -> CommitMessage {
    let text = text.trim();
    if text.is_empty() {
        return CommitMessage {
            headline: format!("{DEFAULT_MESSAGE_PREFIX}{branch}"),
            body: None,
        };
    }
    let (headline, body) = text.split_once(HEADLINE_SEPARATOR).unwrap_or((text, ""));
    let body = body.trim();
    CommitMessage {
        headline: headline.trim().to_string(),
        body: (!body.is_empty()).then(|| body.to_string()),
    }
}

/// The `createCommitOnBranch` request of one publish.
pub fn create_commit_request(
    repository: &GithubRepository,
    branch: &str,
    expected_head: &str,
    message: CommitMessage,
    changes: FileChanges,
) -> GraphqlRequest<GraphqlInput<CreateCommitOnBranchInput>> {
    GraphqlRequest {
        query: CREATE_COMMIT_MUTATION,
        variables: GraphqlInput {
            input: CreateCommitOnBranchInput {
                branch: CommittableBranch {
                    repository_name_with_owner: repository.name_with_owner(),
                    branch_name: branch.to_string(),
                },
                message,
                expected_head_oid: expected_head.to_string(),
                file_changes: changes,
            },
        },
    }
}

/// The `createRef` request creating `branch` at `oid`.
pub fn create_ref_request(
    repository_id: &str,
    branch: &str,
    oid: &str,
) -> GraphqlRequest<GraphqlInput<CreateRefInput>> {
    GraphqlRequest {
        query: CREATE_REF_MUTATION,
        variables: GraphqlInput {
            input: CreateRefInput {
                repository_id: repository_id.to_string(),
                name: format!("{BRANCH_REF_PREFIX}{branch}"),
                oid: oid.to_string(),
            },
        },
    }
}

fn conflict(message: String) -> anyhow::Error {
    IntegrationError::Conflict(message).into()
}

/// A negative verdict (exit 6): the flow escalates with this reason instead of retrying.
fn nothing_to_publish(default_branch: &str) -> anyhow::Error {
    TypedExit {
        status: ExitStatus::NegativeVerdict,
        message: format!("{NOTHING_TO_PUBLISH} {default_branch}"),
    }
    .into()
}

fn upstream(message: String) -> anyhow::Error {
    IntegrationError::Upstream(message).into()
}

fn current_branch(context: &PublishContext<'_>) -> anyhow::Result<String> {
    git_optional(context, &[SYMBOLIC_REF, QUIET_FLAG, SHORT_FLAG, HEAD])?
        .ok_or_else(|| conflict(DETACHED_HEAD.to_string()))
}

fn origin_repository(context: &PublishContext<'_>) -> anyhow::Result<GithubRepository> {
    let url = git_optional(context, &[CONFIG, GET_FLAG, ORIGIN_URL_KEY])?
        .ok_or_else(|| IntegrationError::NotFound(NO_ORIGIN.to_string()))?;
    parse_owner_repo(&url)
        .ok_or_else(|| IntegrationError::NotFound(UNPARSABLE_ORIGIN.to_string()).into())
}

/// The node id and the default branch of `repository`.
fn repository_target(
    context: &PublishContext<'_>,
    repository: &GithubRepository,
) -> anyhow::Result<(String, String)> {
    let request = GraphqlRequest {
        query: REPOSITORY_QUERY,
        variables: repository.clone(),
    };
    let reply = graphql(context, REPOSITORY_REQUEST_FILE, &request)?;
    Ok((
        text_at(&reply, REPOSITORY_ID_POINTER, NO_REPOSITORY_ID)?,
        text_at(&reply, DEFAULT_BRANCH_POINTER, NO_DEFAULT_BRANCH)?,
    ))
}

fn create_ref(
    context: &PublishContext<'_>,
    repository_id: &str,
    branch: &str,
    oid: &str,
) -> anyhow::Result<()> {
    graphql(
        context,
        CREATE_REF_REQUEST_FILE,
        &create_ref_request(repository_id, branch, oid),
    )
    .map(|_| ())
}

/// Sends `request` with `gh api graphql`, its body in a scratch file; GraphQL errors are upstream failures.
fn graphql(
    context: &PublishContext<'_>,
    file_name: &str,
    request: &impl Serialize,
) -> anyhow::Result<Value> {
    let body_path = context.scratch.join(file_name);
    std::fs::write(&body_path, serde_json::to_vec(request)?).map_err(IntegrationError::Io)?;
    let body_argument = body_path.to_string_lossy().to_string();
    let output = run(
        context,
        GH_PROGRAM,
        &[
            API_COMMAND,
            GRAPHQL_ENDPOINT,
            METHOD_FLAG,
            POST_METHOD,
            INPUT_FLAG,
            &body_argument,
        ],
    );
    let _ = std::fs::remove_file(&body_path);
    let output = output?;
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let errors = graphql_errors(&reply);
    if !errors.is_empty() {
        return Err(upstream(format!(
            "{GRAPHQL_FAILED}: {}",
            errors.join(ERROR_SEPARATOR)
        )));
    }
    if !output.status.success() {
        return Err(upstream(format!(
            "{GH_FAILED}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(reply)
}

/// The messages of the `errors` a GraphQL reply carries.
fn graphql_errors(reply: &Value) -> Vec<String> {
    reply
        .get(ERRORS_KEY)
        .and_then(Value::as_array)
        .map(|errors| {
            errors
                .iter()
                .map(|error| {
                    error
                        .get(MESSAGE_KEY)
                        .and_then(Value::as_str)
                        .map_or_else(|| error.to_string(), str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn text_at(reply: &Value, pointer: &str, missing: &str) -> anyhow::Result<String> {
    reply
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| upstream(missing.to_string()))
}

fn run(context: &PublishContext<'_>, program: &str, args: &[&str]) -> anyhow::Result<Output> {
    let spec = ProcessSpec::new(program)
        .args(args.iter().copied())
        .cwd(context.workspace);
    context
        .runner
        .output(&spec)
        .map_err(|error| IntegrationError::Io(error).into())
}

fn git_output(context: &PublishContext<'_>, args: &[&str]) -> anyhow::Result<Output> {
    run(context, GIT_PROGRAM, args)
}

/// The stdout of a git call that must succeed.
fn git_bytes(context: &PublishContext<'_>, args: &[&str]) -> anyhow::Result<Vec<u8>> {
    let output = git_output(context, args)?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(upstream(format!(
        "{GIT_FAILED}: {GIT_PROGRAM} {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn git(context: &PublishContext<'_>, args: &[&str]) -> anyhow::Result<()> {
    git_bytes(context, args).map(|_| ())
}

fn git_text(context: &PublishContext<'_>, args: &[&str]) -> anyhow::Result<String> {
    Ok(String::from_utf8_lossy(&git_bytes(context, args)?)
        .trim()
        .to_string())
}

/// The trimmed stdout of a git call, `None` when it exits non-zero.
fn git_optional(context: &PublishContext<'_>, args: &[&str]) -> anyhow::Result<Option<String>> {
    let output = git_output(context, args)?;
    Ok(output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
}

fn git_succeeds(context: &PublishContext<'_>, args: &[&str]) -> anyhow::Result<bool> {
    Ok(git_output(context, args)?.status.success())
}

#[cfg(test)]
#[path = "../tests/unit/branch_publish_tests.rs"]
mod tests;
