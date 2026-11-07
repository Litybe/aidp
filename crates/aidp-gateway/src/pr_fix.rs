//! `POST /api/prs/fix`: starts an autonomous session that fixes the selected review threads.

use crate::bus::Bus;
use crate::http::App;
use crate::models::constants::pr_fix::{PR_FIX_DEFAULT_MODEL, PR_FIX_EVENT};
use crate::models::constants::pr_fix_keys::{
    KEY_AUTHOR, KEY_BODY, KEY_COMMENTS, KEY_DIFF_HUNK, KEY_IS_RESOLVED, KEY_KIND, KEY_LINE,
    KEY_PATH, KEY_RESULT, KEY_TEXT, KEY_THREADS, KEY_THREAD_ID,
};
use crate::models::constants::pr_fix_limits::{TAIL_LINE_SEPARATOR, TRANSCRIPT_TAIL_BYTES};
use crate::models::constants::pr_fix_messages::{
    BAD_REPO_MESSAGE, BECAME_BUSY_MESSAGE, BUSY_MESSAGE_PREFIX, EMPTY_SELECTION_MESSAGE,
    NOT_MINE_MESSAGE, NO_OPEN_THREAD_MESSAGE, NO_PROJECT_MESSAGE, NO_SKILL_ROOT_MESSAGE,
    NO_VAULT_MESSAGE, PR_CLOSED_MESSAGE, PR_MERGED_MESSAGE, REPO_SEPARATOR, SESSION_FAILED_MESSAGE,
    TITLE_PREFIX, UNKNOWN_AUTHOR,
};
use crate::models::constants::pr_fix_permissions::PR_FIX_BASE_ALLOWED_TOOLS;
use crate::models::constants::session_profile::{AUTONOMOUS_ENABLED, AUTONOMOUS_ENV};
use crate::models::enums::pr_fix_phase::PrFixPhase;
use crate::models::enums::pr_fix_reply_check::PrFixReplyCheck;
use crate::models::enums::pr_refusal_reason::PrRefusalReason;
use crate::models::enums::{
    ApiStatus, Engine, OriginKind, PermissionMode, SessionEventKind, SessionProfile, WorkspaceKind,
};
use crate::models::fix_tracking::FixTracking;
use crate::models::fix_workspace::FixWorkspace;
pub use crate::models::pr_fix_ports::PrFixPorts;
pub use crate::models::pr_fix_pull::PrFixPull;
use crate::models::pr_fix_recipe_input::PrFixRecipeInput;
use crate::models::pr_fix_request::PrFixRequest;
use crate::models::pr_fix_response::PrFixResponse;
use crate::models::pr_fix_session_event::PrFixSessionEvent;
use crate::models::pr_fix_summary::PrFixSummary;
use crate::models::pr_fix_thread_input::PrFixThreadInput;
use crate::models::pr_workspace_holder::PrWorkspaceHolder;
use crate::models::selection_point::SelectionPoint;
pub use crate::models::session_outcome::SessionOutcome;
pub use crate::models::started_session::StartedSession;
use crate::models::ApiError;
use crate::pr_fix_recipe::{load_recipe, parse_summary, render};
use crate::pr_workspace_holder;
use crate::prs::{ticket_key_of_branch, PrRouteError};
use crate::session_error::SessionError;
use crate::sessions::{Origin, StartRequest, ALLOWED_PUSH_BRANCH_ENV};
use crate::worktrees::{self, EnsureError};
use aidp_core::clock::SystemClock;
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner};
use aidp_integrations::branch_key::ticket_key_from_branch;
use aidp_integrations::gh::pr_review_threads_with_handed;
use aidp_integrations::models::pr_thread_mark::PrThreadMark;
use aidp_integrations::pr::{pr_status, PrState};
use aidp_integrations::squad::lane::mark_key;
use aidp_integrations::{HasErrorKind, IntegrationError};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// A route error carrying `status` and `message`, without a refusal reason.
pub(crate) fn failure(status: ApiStatus, message: impl Into<String>) -> PrRouteError {
    PrRouteError {
        error: ApiError {
            status,
            message: message.into(),
        },
        reason: None,
    }
}

/// The registered project whose checkout folder is named `name`, case ignored.
pub(crate) fn project_for_repo(app: &App, name: &str) -> Option<crate::projects::Project> {
    app.projects.list().into_iter().find(|project| {
        aidp_core::vault::repo_segment(Path::new(&project.path)).eq_ignore_ascii_case(name)
    })
}

/// Callback run once when a fix session ended.
pub type SessionEndCallback = Box<dyn FnOnce(SessionOutcome) + Send>;

/// Pause between worktree removal attempts, injectable so a test never waits.
pub type Sleeper = Arc<dyn Fn(Duration) + Send + Sync>;

/// GitHub reads made at click time, never served from the status cache.
pub trait PrFixGithub: Send + Sync {
    fn pull(&self, repo: &str, number: u64) -> Result<PrFixPull, IntegrationError>;

    /// The review-thread document of the PR, resolved threads included.
    fn threads(&self, repo: &str, number: u64) -> Result<Value, IntegrationError>;
}

/// The session manager as the fix flow sees it.
pub trait PrFixHost: Send + Sync {
    /// Names what occupies the workspace, `None` when it is free.
    fn busy_holder(&self, workspace_key: &str) -> Option<String>;

    fn start(&self, request: StartRequest) -> anyhow::Result<StartedSession>;

    fn cancel_queued(&self, queue_id: &str);

    /// Runs `hook` once, when the session ended.
    fn on_end(&self, session_id: &str, hook: SessionEndCallback);
}

struct GhFixGithub;

impl PrFixGithub for GhFixGithub {
    fn pull(&self, repo: &str, number: u64) -> Result<PrFixPull, IntegrationError> {
        let status = pr_status(Some(repo), Some(number), None)?;
        Ok(PrFixPull {
            author: status.author,
            viewer: status.viewer,
            state: status.state,
            head_branch: status.head.name,
        })
    }

    fn threads(&self, repo: &str, number: u64) -> Result<Value, IntegrationError> {
        pr_review_threads_with_handed(Some(&number.to_string()), Some(repo), true, &HashSet::new())
            .map_err(|error| IntegrationError::Upstream(error.to_string()))
    }
}

struct SessionsHost {
    sessions: Arc<crate::sessions::SessionManager>,
}

impl PrFixHost for SessionsHost {
    fn busy_holder(&self, workspace_key: &str) -> Option<String> {
        self.sessions.workspace_holder(workspace_key)
    }

    fn start(&self, request: StartRequest) -> anyhow::Result<StartedSession> {
        let launch = self.sessions.start(request)?;
        Ok(StartedSession {
            id: launch.id().to_string(),
            queued: matches!(launch, crate::sessions::Launch::Queued(_)),
        })
    }

    fn cancel_queued(&self, queue_id: &str) {
        self.sessions.queue_cancel(queue_id);
    }

    fn on_end(&self, session_id: &str, hook: SessionEndCallback) {
        let sessions = self.sessions.clone();
        let id = session_id.to_string();
        self.sessions.on_end(
            session_id,
            Box::new(move |record| {
                let succeeded = record.status == crate::models::enums::SessionStatus::Done;
                let error = (!succeeded).then(|| {
                    record
                        .last_text
                        .clone()
                        .unwrap_or_else(|| SESSION_FAILED_MESSAGE.to_string())
                });
                hook(SessionOutcome {
                    succeeded,
                    error,
                    transcript_tail: transcript_tail(&sessions.events(&id)),
                });
            }),
        );
    }
}

/// The assistant text and turn results of a session, last `TRANSCRIPT_TAIL_BYTES` bytes at most.
pub fn transcript_tail(events: &[Value]) -> String {
    let mut pieces: Vec<&str> = Vec::new();
    for event in events {
        let kind = event.get(KEY_KIND).and_then(Value::as_str);
        let field = match kind {
            Some(kind) if kind == SessionEventKind::Text.as_str() => KEY_TEXT,
            Some(kind) if kind == SessionEventKind::TurnEnd.as_str() => KEY_RESULT,
            _ => continue,
        };
        if let Some(text) = event.get(field).and_then(Value::as_str) {
            pieces.push(text);
        }
    }
    let joined = pieces.join(TAIL_LINE_SEPARATOR);
    let mut start = joined.len().saturating_sub(TRANSCRIPT_TAIL_BYTES);
    while !joined.is_char_boundary(start) {
        start += 1;
    }
    joined[start..].to_string()
}

fn not_mine() -> PrRouteError {
    PrRouteError {
        error: ApiError {
            status: ApiStatus::Forbidden,
            message: NOT_MINE_MESSAGE.to_string(),
        },
        reason: Some(PrRefusalReason::NotMine),
    }
}

fn is_authored_by_viewer(pull: &PrFixPull) -> bool {
    match (pull.author.as_deref(), pull.viewer.as_deref()) {
        (Some(author), Some(viewer)) => author.eq_ignore_ascii_case(viewer),
        _ => false,
    }
}

fn check_open(pull: &PrFixPull) -> Result<(), PrRouteError> {
    match pull.state {
        PrState::Open => Ok(()),
        PrState::Merged => Err(failure(ApiStatus::Conflict, PR_MERGED_MESSAGE)),
        _ => Err(failure(ApiStatus::Conflict, PR_CLOSED_MESSAGE)),
    }
}

fn thread_text(node: &Value, key: &str) -> Option<String> {
    node.get(key).and_then(Value::as_str).map(str::to_string)
}

fn thread_input(node: &Value) -> Option<PrFixThreadInput> {
    let comments = node.get(KEY_COMMENTS).and_then(Value::as_array);
    let diff_hunk = comments
        .and_then(|comments| comments.first())
        .and_then(|first| thread_text(first, KEY_DIFF_HUNK))
        .unwrap_or_default();
    let conversation = comments
        .map(|comments| {
            comments
                .iter()
                .map(|comment| {
                    (
                        thread_text(comment, KEY_AUTHOR)
                            .unwrap_or_else(|| UNKNOWN_AUTHOR.to_string()),
                        thread_text(comment, KEY_BODY).unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PrFixThreadInput {
        id: thread_text(node, KEY_THREAD_ID)?,
        path: thread_text(node, KEY_PATH).unwrap_or_default(),
        line: node.get(KEY_LINE).and_then(Value::as_u64),
        diff_hunk,
        conversation,
    })
}

fn thread_nodes(document: &Value) -> &[Value] {
    document
        .get(KEY_THREADS)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn thread_comments(node: &Value) -> &[Value] {
    node.get(KEY_COMMENTS)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// Comment count of each listed thread present in the document.
fn comment_counts(document: &Value, ids: &[String]) -> BTreeMap<String, usize> {
    thread_nodes(document)
        .iter()
        .filter_map(|node| {
            let id = node.get(KEY_THREAD_ID).and_then(Value::as_str)?;
            ids.iter()
                .any(|chosen| chosen == id)
                .then(|| (id.to_string(), thread_comments(node).len()))
        })
        .collect()
}

/// Whether the thread gained a comment by `viewer` after `previous_count` comments.
fn has_new_reply_by(node: &Value, previous_count: usize, viewer: &str) -> bool {
    thread_comments(node)
        .iter()
        .skip(previous_count)
        .any(|comment| {
            thread_text(comment, KEY_AUTHOR)
                .is_some_and(|author| author.eq_ignore_ascii_case(viewer))
        })
}

/// Chosen threads with no new comment by the PR author since the session started.
fn unanswered_threads(document: &Value, tracking: &FixTracking) -> Vec<String> {
    tracking
        .threads
        .iter()
        .filter(|id| {
            let previous = tracking
                .comment_counts
                .get(*id)
                .copied()
                .unwrap_or_default();
            !thread_nodes(document)
                .iter()
                .filter(|node| node.get(KEY_THREAD_ID).and_then(Value::as_str) == Some(id.as_str()))
                .any(|node| has_new_reply_by(node, previous, &tracking.viewer))
        })
        .cloned()
        .collect()
}

/// Re-reads the threads on GitHub and names the chosen ones that got no reply from the author.
fn check_replies(
    github: &Arc<dyn PrFixGithub>,
    tracking: &FixTracking,
) -> (PrFixReplyCheck, Option<Vec<String>>) {
    match github.threads(&tracking.repo, tracking.number) {
        Ok(document) => {
            let unanswered = unanswered_threads(&document, tracking);
            if unanswered.is_empty() {
                (PrFixReplyCheck::AllAnswered, None)
            } else {
                (PrFixReplyCheck::Unanswered, Some(unanswered))
            }
        }
        Err(error) => {
            warn!("pr fix replies not verified: {error}");
            (PrFixReplyCheck::Unverified, None)
        }
    }
}

/// Splits the selected ids into the threads still open (in selection order) and the dropped ones.
fn split_selection(document: &Value, selected: &[String]) -> (Vec<PrFixThreadInput>, Vec<String>) {
    let nodes = thread_nodes(document);
    let mut open = Vec::new();
    let mut dropped = Vec::new();
    for id in selected {
        let found = nodes
            .iter()
            .find(|node| node.get(KEY_THREAD_ID).and_then(Value::as_str) == Some(id.as_str()));
        let input = found
            .filter(|node| {
                !node
                    .get(KEY_IS_RESOLVED)
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .and_then(thread_input);
        match input {
            Some(thread)
                if !open
                    .iter()
                    .any(|kept: &PrFixThreadInput| kept.id == thread.id) =>
            {
                open.push(thread)
            }
            Some(_) => {}
            None => dropped.push(id.clone()),
        }
    }
    (open, dropped)
}

fn ensure_failure(error: EnsureError) -> PrRouteError {
    let status = match error {
        EnsureError::BranchCheckedOutElsewhere { .. } => ApiStatus::Conflict,
        EnsureError::InvalidKey(_) => ApiStatus::BadRequest,
        EnsureError::Git(_) => ApiStatus::Upstream,
        EnsureError::Io(_) => ApiStatus::Io,
    };
    failure(status, error.to_string())
}

fn start_failure(error: anyhow::Error) -> PrRouteError {
    let status = error
        .downcast_ref::<SessionError>()
        .map(|typed| ApiStatus::from(typed.kind()))
        .unwrap_or(ApiStatus::Conflict);
    failure(status, error.to_string())
}

/// The ticket worktree when the head is `feat|fix/<KEY>` and it exists, else the PR worktree path.
fn plan_workspace(ports: &PrFixPorts<'_>, number: u64, branch: &str) -> FixWorkspace {
    let project_path = Path::new(&ports.project.path);
    let ticket_key = ticket_key_from_branch(branch)
        .filter(|key| ticket_key_of_branch(branch) == Some(key.as_str()));
    if let Some(request) = ticket_key
        .and_then(|key| worktrees::ticket_path_request(ports.vault_root, project_path, &key))
    {
        let path = request.path.clone().map(PathBuf::from).unwrap_or_default();
        return FixWorkspace {
            request,
            path,
            kind: WorkspaceKind::Ticket,
            owned: false,
        };
    }
    let segment = aidp_core::vault::repo_segment(project_path);
    let path = worktrees::pr_worktree_path(ports.vault_root, &segment, number);
    FixWorkspace {
        request: worktrees::path_workspace_request(&path),
        path,
        kind: WorkspaceKind::PrWorktree,
        owned: true,
    }
}

fn event_for(tracking: &FixTracking, phase: PrFixPhase) -> PrFixSessionEvent {
    PrFixSessionEvent {
        repo: tracking.repo.clone(),
        number: tracking.number,
        session_id: tracking.session_id.clone(),
        phase,
        threads: tracking.threads.clone(),
        fixed: None,
        needs_reviewer: None,
        not_relevant: None,
        unreported: None,
        assumptions: None,
        pushed_sha: None,
        worktree_kept: None,
        error: None,
        summary_missing: false,
        unanswered: None,
        reply_check: PrFixReplyCheck::NotChecked,
    }
}

fn publish(bus: &Bus, event: &PrFixSessionEvent) {
    match serde_json::to_value(event) {
        Ok(payload) => {
            bus.emit(PR_FIX_EVENT, payload);
        }
        Err(error) => warn!("pr fix event not published: {error}"),
    }
}

fn parsed_summary(transcript_tail: &str) -> Option<PrFixSummary> {
    let value = parse_summary(transcript_tail)?;
    serde_json::from_value(value).ok()
}

/// Publishes the `ended` event and removes the PR worktree once the work is pushed.
fn finish_fix(
    runner: &Arc<dyn ProcessRunner>,
    sleeper: &Sleeper,
    bus: &Bus,
    github: &Arc<dyn PrFixGithub>,
    tracking: &FixTracking,
    outcome: SessionOutcome,
) {
    let mut event = event_for(tracking, PrFixPhase::Ended);
    let summary = parsed_summary(&outcome.transcript_tail);
    let pushed = outcome.succeeded
        && summary
            .as_ref()
            .is_some_and(|summary| summary.pushed_sha.is_some());
    match summary {
        Some(summary) => {
            event.unreported = Some(summary.unreported(&tracking.threads));
            event.fixed = Some(summary.fixed);
            event.needs_reviewer = Some(summary.needs_reviewer);
            event.not_relevant = Some(summary.not_relevant);
            event.assumptions = Some(summary.assumptions);
            event.pushed_sha = summary.pushed_sha;
        }
        None => event.summary_missing = true,
    }
    event.error = outcome.error;
    (event.reply_check, event.unanswered) = check_replies(github, tracking);
    if tracking.owned_worktree {
        let removed = pushed
            && worktrees::remove_pr_worktree(
                runner.as_ref(),
                sleeper.as_ref(),
                &tracking.project_path,
                &tracking.worktree,
                tracking.number,
            )
            .map_err(|error| warn!("pr worktree kept: {error}"))
            .is_ok();
        if !removed {
            event.worktree_kept = Some(tracking.worktree.display().to_string());
        }
    }
    publish(bus, &event);
}

/// Tool rules the fix session may use: the base set plus pushes to the head branch only.
pub fn allowed_tools(head_branch: &str) -> anyhow::Result<Vec<String>> {
    let mut rules: Vec<String> = PR_FIX_BASE_ALLOWED_TOOLS
        .iter()
        .map(|rule| rule.to_string())
        .collect();
    rules.extend(SessionProfile::push_branch_allow_rules(head_branch)?);
    Ok(rules)
}

fn point_inputs(points: &[String]) -> Vec<SelectionPoint> {
    points
        .iter()
        .map(|point| SelectionPoint {
            id: point.clone(),
            body: point.clone(),
        })
        .collect()
}

/// Whether what the recorded holder names still exists: its session, else its native run.
fn holder_is_live(ports: &PrFixPorts<'_>, holder: &PrWorkspaceHolder, workspace_key: &str) -> bool {
    match (&holder.session_id, &holder.run_id) {
        (Some(_), _) => ports.host.busy_holder(workspace_key).is_some(),
        (None, Some(run_id)) => (ports.run_is_live)(run_id),
        (None, None) => false,
    }
}

/// Who holds the PR branch: a live stage recorded in the vault first, else the session host's answer.
fn busy_holder_of(ports: &PrFixPorts<'_>, number: u64, workspace_key: &str) -> Option<String> {
    let segment = aidp_core::vault::repo_segment(Path::new(&ports.project.path));
    if let Some(holder) = pr_workspace_holder::read(ports.vault_root, &segment, number) {
        if holder_is_live(ports, &holder, workspace_key) {
            return Some(holder.describe());
        }
        pr_workspace_holder::remove(ports.vault_root, &segment, number);
    }
    ports.host.busy_holder(workspace_key)
}

/// Admits the request, prepares the workspace, starts the session and registers its end handler.
pub fn fix_pull_request(
    ports: &PrFixPorts<'_>,
    request: PrFixRequest,
) -> Result<Value, PrRouteError> {
    let PrFixRequest {
        repo,
        number,
        threads,
        points,
        note,
        model,
        effort,
    } = request;
    if !repo.contains(REPO_SEPARATOR) {
        return Err(failure(ApiStatus::BadRequest, BAD_REPO_MESSAGE));
    }
    if threads.is_empty() && points.is_empty() {
        return Err(failure(ApiStatus::BadRequest, EMPTY_SELECTION_MESSAGE));
    }
    let pull = ports.github.pull(&repo, number)?;
    if !is_authored_by_viewer(&pull) {
        return Err(not_mine());
    }
    check_open(&pull)?;
    SessionProfile::validate_push_branch(&pull.head_branch)
        .map_err(|error| failure(ApiStatus::BadRequest, error.to_string()))?;
    let document = ports.github.threads(&repo, number)?;
    let (open_threads, dropped_threads) = split_selection(&document, &threads);
    if open_threads.is_empty() && points.is_empty() {
        return Err(failure(ApiStatus::Conflict, NO_OPEN_THREAD_MESSAGE));
    }

    let workspace = plan_workspace(ports, number, &pull.head_branch);
    let workspace_key = worktrees::normalize_path(&workspace.path);
    if let Some(holder) = busy_holder_of(ports, number, &workspace_key) {
        return Err(failure(
            ApiStatus::Conflict,
            format!("{BUSY_MESSAGE_PREFIX}{holder}"),
        ));
    }
    if workspace.owned {
        let segment = aidp_core::vault::repo_segment(Path::new(&ports.project.path));
        worktrees::ensure_pr_worktree(
            ports.runner.as_ref(),
            Path::new(&ports.project.path),
            ports.vault_root,
            &segment,
            number,
            &pull.head_branch,
        )
        .map_err(ensure_failure)?;
    }

    let recipe = load_recipe(ports.skill_root)
        .map_err(|error| failure(ApiStatus::Config, error.to_string()))?;
    let thread_ids: Vec<String> = open_threads
        .iter()
        .map(|thread| thread.id.clone())
        .collect();
    let prompt = render(
        &recipe,
        &PrFixRecipeInput {
            repo: repo.clone(),
            pr_number: number,
            head_branch: pull.head_branch.clone(),
            threads: open_threads,
            points: point_inputs(&points),
            note,
        },
    );
    let start = StartRequest {
        project_id: ports.project.id.clone(),
        engine: Some(Engine::Prompt.as_str().to_string()),
        prompt: Some(prompt),
        model: Some(model.unwrap_or_else(|| PR_FIX_DEFAULT_MODEL.to_string())),
        effort,
        workspace: Some(workspace.request.clone()),
        auto_close: Some(true),
        origin: Some(Origin {
            kind: OriginKind::PrFix.as_str().to_string(),
            r#ref: Some(format!("{repo}#{number}")),
            label: Some(format!("{repo}#{number}")),
        }),
        title: Some(format!("{TITLE_PREFIX}{repo}#{number}")),
        env: BTreeMap::from([
            (AUTONOMOUS_ENV.to_string(), AUTONOMOUS_ENABLED.to_string()),
            (
                ALLOWED_PUSH_BRANCH_ENV.to_string(),
                pull.head_branch.clone(),
            ),
        ]),
        permission_mode: Some(PermissionMode::DontAsk.as_str().to_string()),
        allowed_tools: allowed_tools(&pull.head_branch).map_err(start_failure)?,
        ..Default::default()
    };
    let started = ports.host.start(start).map_err(start_failure)?;
    if started.queued {
        ports.host.cancel_queued(&started.id);
        return Err(failure(ApiStatus::Conflict, BECAME_BUSY_MESSAGE));
    }

    record_fix(ports, &repo, number, &started.id, &thread_ids, &points);
    let tracking = FixTracking {
        repo: repo.clone(),
        number,
        session_id: started.id.clone(),
        threads: thread_ids.clone(),
        worktree: workspace.path.clone(),
        owned_worktree: workspace.owned,
        project_path: PathBuf::from(&ports.project.path),
        viewer: pull.viewer.clone().unwrap_or_default(),
        comment_counts: comment_counts(&document, &thread_ids),
    };
    publish(&ports.bus, &event_for(&tracking, PrFixPhase::Started));
    let (runner, sleeper, bus, github) = (
        ports.runner.clone(),
        ports.sleeper.clone(),
        ports.bus.clone(),
        ports.github.clone(),
    );
    ports.host.on_end(
        &started.id,
        Box::new(move |outcome| finish_fix(&runner, &sleeper, &bus, &github, &tracking, outcome)),
    );
    serde_json::to_value(PrFixResponse {
        session_id: started.id,
        workspace: workspace.path.display().to_string(),
        workspace_kind: workspace.kind,
        threads: thread_ids,
        dropped_threads,
    })
    .map_err(|error| failure(ApiStatus::Internal, error.to_string()))
}

/// Marks the PR and each selected thread as handed to a fix session; failures are logged only.
fn record_fix(
    ports: &PrFixPorts<'_>,
    repo: &str,
    number: u64,
    session_id: &str,
    thread_ids: &[String],
    points: &[String],
) {
    let at = ports.clock.now_rfc3339();
    let marked = ports.marks.update(&mark_key(repo, number), |mark| {
        mark.fix_session = Some(session_id.to_string());
        for thread_id in thread_ids {
            mark.threads.insert(
                thread_id.clone(),
                PrThreadMark {
                    fix_session: session_id.to_string(),
                    at: at.clone(),
                },
            );
        }
        mark.fix_at = Some(at);
    });
    if let Err(error) = marked {
        warn!("pr fix mark not written: {error}");
    }
    let handed: Vec<String> = thread_ids.iter().chain(points).cloned().collect();
    if let Err(error) = ports.comment_state.add_handed(repo, number, &handed) {
        warn!("pr fix handed ids not written: {error}");
    }
}

/// Starts the fix session of `request` on the real GitHub, session manager and vault.
pub fn handle_fix(app: &App, request: PrFixRequest) -> Result<Value, PrRouteError> {
    let name = request
        .repo
        .split_once(REPO_SEPARATOR)
        .map(|(_, name)| name)
        .ok_or_else(|| failure(ApiStatus::BadRequest, BAD_REPO_MESSAGE))?;
    let project = project_for_repo(app, name).ok_or_else(|| {
        failure(
            ApiStatus::NotFound,
            format!("{NO_PROJECT_MESSAGE}{}", request.repo),
        )
    })?;
    if app.vault_root.as_os_str().is_empty() {
        return Err(failure(ApiStatus::Config, NO_VAULT_MESSAGE));
    }
    let skill_root = app
        .skill_root
        .clone()
        .ok_or_else(|| failure(ApiStatus::Config, NO_SKILL_ROOT_MESSAGE))?;
    let host = SessionsHost {
        sessions: app.sessions.clone(),
    };
    let native_runs = app.native_runs.clone();
    let ports = PrFixPorts {
        github: Arc::new(GhFixGithub),
        run_is_live: Arc::new(move |run_id| {
            native_runs
                .get(run_id)
                .is_some_and(|view| !view.status.is_terminal())
        }),
        host: &host,
        runner: Arc::new(OsProcessRunner),
        sleeper: Arc::new(std::thread::sleep),
        bus: app.bus.clone(),
        clock: &SystemClock,
        marks: &app.pr_marks,
        comment_state: &app.pr_comment_state,
        vault_root: &app.vault_root,
        skill_root: &skill_root,
        project: &project,
    };
    fix_pull_request(&ports, request)
}

#[cfg(test)]
#[path = "../tests/unit/pr_fix_tests.rs"]
mod tests;
