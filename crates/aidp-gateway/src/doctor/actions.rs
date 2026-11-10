//! The allowed actions, executed by the daemon after it checked their preconditions.
//! None merges, approves, force-pushes, touches the game or the harness, or opens Unity.

use crate::doctor::converge::{self, Git};
use crate::doctor::ports::{DoctorRuns, HarnessRepos};
use crate::flows::executor::step_at_cursor;
use crate::flows::facts;
use crate::flows::registry::{FlowAction, SubprocessAction};
use crate::flows::FlowEngine;
use crate::models::constants::doctor::{
    AIDP_LABEL, AIDP_TICKET, DOCTOR_ACTOR, FORBIDDEN_BRANCH_PREFIX, GH_PROGRAM, GH_REPO_FLAG,
    GH_UPDATE_BRANCH, HARNESS_AUTHOR, HARNESS_BASE, HARNESS_BRANCH_PREFIX, HARNESS_REMOTE,
    HARNESS_REPOS, PARAM_CRITERION, PARAM_DEFECT, PARAM_DEFECT_FILE, PARAM_FILES,
    PARAM_HARNESS_REPO, PARAM_PR, PARAM_REPO, PARAM_RESOURCE, PARAM_STEP, PARAM_SUMMARY,
    PROTECTED_BRANCHES, REBASE_PARAMS, RESTORE_COMMIT_PREFIX, STATE_READY, STATE_TO_TEST,
    WORKTREES_DIR,
};
use crate::models::constants::flow_actions::{
    ACTION_GH_MERGE, PLACEHOLDER_KEY_ARG, VERB_FLOW, VERB_FLOW_BUG_REARM, WITH_ALLOW_EXTRA,
};
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::{DoctorAction, DoctorActionResult, DoctorCategory, DoctorTrigger};
use crate::models::{ActionContext, DoctorDiagnosis, DoctorIncident};
use crate::unity_lock::resource_key;
use aidp_core::process_runner::ProcessSpec;
use aidp_flows::models::constants::resource::UNITY_EDITOR;
use aidp_flows::{ActionOutcome, StepKind, StepStatus};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const COMMAND_POLL: Duration = Duration::from_millis(50);
const KEY_PLACEHOLDER: &str = "{key}";

/// What an action needs.
pub struct ActionTarget<'a> {
    pub engine: &'a Arc<FlowEngine>,
    pub runs: &'a Arc<dyn DoctorRuns>,
    pub incident: &'a DoctorIncident,
    pub diagnosis: &'a DoctorDiagnosis,
    /// Local clone of each harness repository, by `owner/name`.
    pub harness_repos: &'a HarnessRepos,
    /// The daemon's state dir: temporary worktrees of harness patches live under it.
    pub state_dir: &'a Path,
    /// Key of the dedicated harness issue (`doctor.harness_ticket`), when one is set.
    pub harness_ticket: Option<&'a str>,
    /// False when the bug cap of the ticket was already re-armed in the last 24 hours.
    pub rearm_allowed: bool,
}

use DoctorActionResult::{Done, Failed, Refused};

/// Runs `action` once its precondition holds.
pub fn execute(target: &ActionTarget<'_>, action: DoctorAction) -> DoctorActionResult {
    match action {
        DoctorAction::RetryStep => retry_step(target),
        DoctorAction::RequeueToTest => requeue(target, STATE_TO_TEST),
        DoctorAction::RequeueDev => requeue(target, STATE_READY),
        DoctorAction::AbortDeadFlowAndRequeue => abort_dead_flow(target),
        DoctorAction::UpdatePrBranch => update_pr_branch(target),
        DoctorAction::ReleaseUnityLock => release_unity_lock(target),
        DoctorAction::CreateBugTicket => create_bug_ticket(target),
        DoctorAction::OpenHarnessPr => open_harness_pr(target),
        DoctorAction::RestoreTestInfra => restore_test_infra(target),
        DoctorAction::None => Done("nothing to do".to_string()),
    }
}

fn param<'a>(target: &'a ActionTarget<'_>, name: &str) -> Option<&'a Value> {
    target
        .diagnosis
        .params
        .get(name)
        .filter(|value| !value.is_null())
}

fn text_param(target: &ActionTarget<'_>, name: &str) -> Option<String> {
    param(target, name).and_then(|value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

/// Retries the step of the incident (or the one named), never a human step. A step still
/// running is reconciled first, so a run that ended is read before the retry.
fn retry_step(target: &ActionTarget<'_>) -> DoctorActionResult {
    let key = target.incident.candidate.key.as_str();
    let Some(step) =
        text_param(target, PARAM_STEP).or_else(|| target.incident.candidate.step.clone())
    else {
        return resume_run(target);
    };
    let executor = target.engine.executor();
    let Ok((_, state)) = executor.load(key) else {
        return Refused(format!("no flow run for `{key}`"));
    };
    if state.current.as_deref() != Some(step.as_str()) {
        return Refused(format!("`{step}` is not the current step of `{key}`"));
    }
    let definition = step_at_cursor(&state);
    if definition
        .as_ref()
        .is_some_and(|definition| definition.kind == StepKind::Human)
    {
        return Refused(format!(
            "`{step}` is a human step: the doctor never decides one"
        ));
    }
    if definition
        .as_ref()
        .and_then(|definition| definition.use_.as_deref())
        == Some(ACTION_GH_MERGE)
    {
        return Refused(format!(
            "`{step}` merges a pull request: the doctor never merges"
        ));
    }
    if state.steps.get(&step).map(|row| row.status) == Some(StepStatus::Running) {
        if let Err(error) = executor.reconcile(key) {
            return Failed(format!("reconcile of `{key}` failed: {error}"));
        }
        let Ok((_, after)) = executor.load(key) else {
            return Failed(format!("`{key}` disappeared"));
        };
        let status = after.steps.get(&step).map(|row| row.status);
        if status == Some(StepStatus::Running) {
            return Failed(format!("`{step}` is still running after a reconcile"));
        }
        if after.current.as_deref() != Some(step.as_str())
            || !matches!(status, Some(StepStatus::Failed | StepStatus::Escalated))
        {
            return Done(format!("reconciled `{key}`: `{step}` is {status:?} now"));
        }
    }
    match target.engine.retry(key, &step, DOCTOR_ACTOR) {
        Ok(_) => Done(format!("retried `{step}`")),
        Err(error) => Failed(format!("retry of `{step}` refused: {error}")),
    }
}

/// A paused run outside any flow: resumed where it stopped, its finished stages kept.
fn resume_run(target: &ActionTarget<'_>) -> DoctorActionResult {
    let Some(run_id) = target.incident.candidate.run_id.as_deref() else {
        return Refused("no step to retry".to_string());
    };
    match target.runs.get(run_id) {
        Some(view) if view.status == NativeRunStatus::Paused => {}
        Some(_) => return Refused(format!("run `{run_id}` is not paused")),
        None => return Refused(format!("run `{run_id}` is gone")),
    }
    match target.runs.resume(run_id) {
        Ok(()) => Done(format!("run `{run_id}` resumed where it stopped")),
        Err(error) => Failed(format!("resume of run `{run_id}` failed: {error}")),
    }
}

/// Moves the ticket's state label, which puts it back in a lane; never while a flow owns it.
fn requeue(target: &ActionTarget<'_>, state: &str) -> DoctorActionResult {
    let ticket = target.incident.candidate.ticket.as_str();
    if target.engine.has_active_run(ticket)
        || target.engine.has_active_run(&target.incident.candidate.key)
    {
        return Refused(format!("`{ticket}` still has an active flow"));
    }
    label(target, ticket, state)
}

fn label(target: &ActionTarget<'_>, ticket: &str, state: &str) -> DoctorActionResult {
    let action = SubprocessAction::new([AIDP_TICKET, AIDP_LABEL, KEY_PLACEHOLDER, state]);
    let runner = target.engine.executor().ports().runner.clone();
    let context = ActionContext {
        key: ticket,
        workspace: None,
        runner: runner.as_ref(),
    };
    match action.run(&context, &BTreeMap::new()) {
        ActionOutcome::Ok(_) => Done(format!("`{ticket}` labelled {state}")),
        other => Failed(format!("label {state} on `{ticket}` failed: {other:?}")),
    }
}

/// Aborts a flow that already ended, so its ticket is no longer owned by it, then puts the
/// ticket back in the lane of that flow: the test lane for a project-checkout flow, else dev.
fn abort_dead_flow(target: &ActionTarget<'_>) -> DoctorActionResult {
    let key = target.incident.candidate.key.as_str();
    let Some(entry) = target.engine.executor().store().index_get(key) else {
        return Refused(format!("no flow run for `{key}`"));
    };
    if !entry.status.is_terminal() {
        return Refused(format!(
            "`{key}` is {}: only a terminal flow is aborted",
            entry.status.as_wire()
        ));
    }
    let test_lane = target.engine.executor().load(key).is_ok_and(|(_, state)| {
        state.def.workspace == Some(aidp_flows::models::enums::WorkspaceMode::Project)
    });
    if entry.status != aidp_flows::FlowStatus::Aborted {
        if let Err(error) = target.engine.abort(key, DOCTOR_ACTOR) {
            return Failed(format!("abort of `{key}` failed: {error}"));
        }
    }
    let lane = if test_lane {
        STATE_TO_TEST
    } else {
        STATE_READY
    };
    label(target, &target.incident.candidate.ticket, lane)
}

/// `gh pr update-branch` by merge, the GitHub default; a rebase or force is refused.
fn update_pr_branch(target: &ActionTarget<'_>) -> DoctorActionResult {
    if REBASE_PARAMS
        .iter()
        .any(|name| param(target, name).is_some_and(|value| value != &Value::Bool(false)))
    {
        return Refused("only a merge update is allowed: no rebase, no force".to_string());
    }
    let state = target
        .engine
        .executor()
        .load(&target.incident.candidate.key)
        .ok()
        .map(|(_, state)| state);
    let known = state.as_ref().and_then(facts::pull_request);
    let number =
        text_param(target, PARAM_PR).or_else(|| known.as_ref().map(|pr| pr.number.to_string()));
    let repo = text_param(target, PARAM_REPO).or_else(|| known.and_then(|pr| pr.repository));
    let (Some(number), Some(repo)) = (number, repo) else {
        return Refused("no pull request number and repository".to_string());
    };
    if number.parse::<u64>().is_err() {
        return Refused(format!("`{number}` is not a pull request number"));
    }
    let mut spec = ProcessSpec::new(GH_PROGRAM)
        .args(GH_UPDATE_BRANCH.iter().map(|part| part.to_string()))
        .args([number.clone(), GH_REPO_FLAG.to_string(), repo.clone()]);
    spec.piped_stdout = true;
    spec.piped_stderr = true;
    match run_command(target, &spec) {
        Ok(0) => Done(format!("{repo}#{number} updated by merge")),
        Ok(code) => Failed(format!("gh pr update-branch exited {code}")),
        Err(error) => Failed(format!("gh pr update-branch failed: {error}")),
    }
}

fn run_command(target: &ActionTarget<'_>, spec: &ProcessSpec) -> std::io::Result<i32> {
    let runner = target.engine.executor().ports().runner.clone();
    let mut child = runner.spawn(spec)?;
    let drains: Vec<_> = [child.take_stdout(), child.take_stderr()]
        .into_iter()
        .flatten()
        .map(|mut stream| {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            })
        })
        .collect();
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let code = loop {
        if let Some(code) = child.try_wait()? {
            break code;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            break -1;
        }
        std::thread::sleep(COMMAND_POLL);
    };
    for drain in drains {
        let _ = drain.join();
    }
    Ok(code)
}

/// Frees the Unity editor lock, only when its holder's flow is no longer active.
fn release_unity_lock(target: &ActionTarget<'_>) -> DoctorActionResult {
    let executor = target.engine.executor();
    let resource = text_param(target, PARAM_RESOURCE).or_else(|| {
        let project = match &target.incident.candidate.project_id {
            Some(id) => id.clone(),
            None => {
                let (_, state) = executor.load(&target.incident.candidate.key).ok()?;
                executor.ports().sessions.project_for(&state.ticket)?.id
            }
        };
        Some(resource_key(UNITY_EDITOR, &project))
    });
    let Some(resource) = resource else {
        return Refused("no Unity lock to name".to_string());
    };
    let (holder, _) = executor.resources().snapshot(&resource);
    let Some(holder) = holder else {
        return Refused(format!("nobody holds {resource}"));
    };
    if let Some(entry) = executor.store().index_get(&holder.flow_key) {
        if !entry.status.is_terminal() {
            return Refused(format!(
                "the holder `{}` is {}, not terminal",
                holder.flow_key,
                entry.status.as_wire()
            ));
        }
    }
    match executor.release_resource(&resource) {
        Some(released) => Done(format!("{resource} released from `{}`", released.flow_key)),
        None => Done(format!("{resource} was already free")),
    }
}

/// The local checkout of the project of flow `key`: where git reads and where temporary worktrees are cut.
pub fn checkout_for(engine: &Arc<FlowEngine>, key: &str) -> Option<PathBuf> {
    let executor = engine.executor();
    let (_, state) = executor.load(key).ok()?;
    let sessions = &executor.ports().sessions;
    sessions
        .project_for(&state.ticket)
        .map(|project| project.path)
        .or_else(|| sessions.test_checkout_for(&state.ticket))
}

/// The strings of parameter `name`: a list, or one comma-separated text.
fn list_param(target: &ActionTarget<'_>, name: &str) -> Vec<String> {
    let items: Vec<String> = match param(target, name) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(text)) => text.split(',').map(str::to_string).collect(),
        _ => Vec::new(),
    };
    items
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Checks that a `real_defect` diagnosis proves a cause the earlier bugs did not fix: it names the
/// defect, the file and the criterion, quotes evidence, and differs from every earlier bug.
fn prove_real_defect(target: &ActionTarget<'_>) -> Result<(), String> {
    let candidate = &target.incident.candidate;
    if candidate.trigger != DoctorTrigger::BugsNotConverging {
        return Err("a bug past the cap is raised only once the cap was reached".to_string());
    }
    let named = |name: &str| text_param(target, name).filter(|text| !text.trim().is_empty());
    let (defect, file, criterion) = (
        named(PARAM_DEFECT),
        named(PARAM_DEFECT_FILE),
        named(PARAM_CRITERION),
    );
    let missing: Vec<&str> = [
        (PARAM_DEFECT, defect.is_some()),
        (PARAM_DEFECT_FILE, file.is_some()),
        (PARAM_CRITERION, criterion.is_some()),
    ]
    .into_iter()
    .filter(|(_, present)| !present)
    .map(|(name, _)| name)
    .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the cause is not proven: {} missing",
            missing.join(", ")
        ));
    }
    if target.diagnosis.evidence.is_empty() {
        return Err("the cause is not proven: no evidence".to_string());
    }
    let (Some(defect), Some(file)) = (defect, file) else {
        return Err("the cause is not proven".to_string());
    };
    let Some(clone) = checkout_for(target.engine, &candidate.key) else {
        return Err("no checkout to read the earlier bugs from".to_string());
    };
    let runner = target.engine.executor().ports().runner.clone();
    let git = Git {
        runner: runner.as_ref(),
        cwd: &clone,
    };
    let gathered = converge::gather(&git, &candidate.ticket, None)?;
    if gathered.bugs.is_empty() {
        return Err(
            "no earlier bug in the history of the branch: cannot show the cause differs"
                .to_string(),
        );
    }
    let cause = format!("{defect} {}", target.diagnosis.cause);
    if converge::differs_from_previous(&file, &cause, &gathered.bugs) {
        Ok(())
    } else {
        Err(format!(
            "the cause matches an earlier bug that already touched `{file}`"
        ))
    }
}

/// Raises a bug issue through the `bug.create` action (`aidp flow bug-create`): same marker, same
/// cap of bugs per parent, and the parent waits while the bug is open. Three kinds only: a product
/// bug under the ticket, a real defect proven after the cap (one more bug), a shared harness
/// failure under the dedicated harness ticket.
fn create_bug_ticket(target: &ActionTarget<'_>) -> DoctorActionResult {
    let candidate = &target.incident.candidate;
    let diagnosis = target.diagnosis;
    let (ticket, past_cap) = match diagnosis.category {
        DoctorCategory::ProductBug => (candidate.ticket.clone(), false),
        DoctorCategory::RealDefect => match prove_real_defect(target) {
            Ok(()) => (candidate.ticket.clone(), true),
            Err(reason) => return Refused(reason),
        },
        DoctorCategory::SharedHarnessFailure => match target.harness_ticket {
            Some(key) => (key.to_string(), false),
            None => return Refused("no harness ticket: set `doctor.harness_ticket`".to_string()),
        },
        _ => {
            return Refused(
                "a bug ticket is raised for a product bug, a real defect or a shared harness failure only"
                    .to_string(),
            )
        }
    };
    let ticket = ticket.as_str();
    if ticket.is_empty() || candidate.run_id.as_deref() == Some(ticket) {
        return Refused("the incident has no ticket to raise a bug under".to_string());
    }
    let mut report = format!(
        "{}\n\n{}\n",
        diagnosis.cause.trim(),
        diagnosis.summary.trim()
    );
    for item in &diagnosis.evidence {
        report.push_str(&format!("\n- {item}"));
    }
    if !target.incident.affected.is_empty() {
        report.push_str(&format!(
            "\n\nTickets touchés : {}.",
            target.incident.affected.join(", ")
        ));
    }
    report.push_str(&format!("\n\nIncident du docteur : `{}`.\n", candidate.id));
    let summary = text_param(target, PARAM_SUMMARY).unwrap_or_else(|| {
        diagnosis
            .cause
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    });
    let mut with = BTreeMap::new();
    with.insert("report".to_string(), Value::String(report));
    with.insert("summary".to_string(), Value::String(summary));
    if past_cap {
        with.insert(WITH_ALLOW_EXTRA.to_string(), Value::Bool(true));
    }
    let runner = target.engine.executor().ports().runner.clone();
    let context = ActionContext {
        key: ticket,
        workspace: None,
        runner: runner.as_ref(),
    };
    match crate::flows::actions::BugCreateAction.run(&context, &with) {
        ActionOutcome::Ok(outputs) => {
            if outputs.get("capped").and_then(Value::as_bool) == Some(true) {
                return Refused(format!(
                    "`{ticket}` already holds the most bugs: none created"
                ));
            }
            let number = outputs
                .get("number")
                .map(Value::to_string)
                .unwrap_or_default();
            Done(format!(
                "bug #{number} raised under `{ticket}`; the ticket waits for it"
            ))
        }
        other => Failed(format!("bug-create on `{ticket}` failed: {other:?}")),
    }
}

/// Puts the test-infrastructure files a branch broke back from `origin/main`: a temporary
/// detached worktree cut from the branch on `origin`, the files checked out from `origin/main`,
/// one commit, a normal push to the branch. Never `main`, never a force-push, never `gamedev/*`.
fn restore_test_infra(target: &ActionTarget<'_>) -> DoctorActionResult {
    let candidate = &target.incident.candidate;
    if target.diagnosis.category != DoctorCategory::TestInfraRegression {
        return Refused(
            "test infrastructure is restored for a test_infra_regression only".to_string(),
        );
    }
    let ticket = candidate.ticket.as_str();
    if ticket.is_empty() {
        return Refused("the incident has no ticket".to_string());
    }
    let Some(clone) = checkout_for(target.engine, &candidate.key) else {
        return Refused("no checkout for the project of the ticket".to_string());
    };
    let branch = converge::parent_branch(ticket);
    if PROTECTED_BRANCHES.contains(&branch.as_str()) || branch.starts_with(FORBIDDEN_BRANCH_PREFIX)
    {
        return Refused(format!("branch `{branch}` is forbidden"));
    }
    let runner = target.engine.executor().ports().runner.clone();
    let git = Git {
        runner: runner.as_ref(),
        cwd: &clone,
    };
    converge::fetch(&git, &branch);
    let remote_tip = format!("{HARNESS_REMOTE}/{branch}");
    let base = format!("{HARNESS_REMOTE}/{HARNESS_BASE}");
    if !git.resolves(&remote_tip) || !git.resolves(&base) {
        return Refused(format!("`{branch}` or `{base}` is not on {HARNESS_REMOTE}"));
    }
    let changed = converge::infra_changes(&git, &base, &remote_tip);
    let mut wanted = list_param(target, PARAM_FILES);
    if wanted.is_empty() {
        wanted = changed
            .iter()
            .filter(|file| file.suspect && file.status != converge::STATUS_ADDED)
            .map(|file| file.path.clone())
            .collect();
    }
    if wanted.is_empty() {
        return Refused("no test-infrastructure file to restore".to_string());
    }
    for path in &wanted {
        if !converge::is_infra_path(path) {
            return Refused(format!("`{path}` is not a test-infrastructure file"));
        }
        match changed.iter().find(|file| file.path == *path) {
            None => return Refused(format!("`{path}` is not changed by `{branch}`")),
            Some(file) if file.status == converge::STATUS_ADDED => {
                return Refused(format!(
                    "`{path}` was added by `{branch}`: it is not on {base}"
                ))
            }
            Some(_) => {}
        }
    }
    let name = format!("{}-infra", crate::doctor::bundle_name(&candidate.id));
    let worktree: PathBuf = target.state_dir.join(WORKTREES_DIR).join(&name);
    let outcome = push_restoration(
        target,
        &clone,
        &worktree,
        &Restoration {
            branch: &branch,
            base: &base,
            remote_tip: &remote_tip,
            files: &wanted,
        },
    );
    let _ = run_in(
        target,
        &clone,
        "git",
        &["worktree", "remove", "--force", &worktree.to_string_lossy()],
    );
    let _ = std::fs::remove_dir_all(&worktree);
    outcome
}

/// The branch to repair, the ref the files come from, and the files.
struct Restoration<'a> {
    branch: &'a str,
    base: &'a str,
    remote_tip: &'a str,
    files: &'a [String],
}

fn push_restoration(
    target: &ActionTarget<'_>,
    clone: &Path,
    worktree: &Path,
    restoration: &Restoration<'_>,
) -> DoctorActionResult {
    let Restoration {
        branch,
        base,
        remote_tip,
        files,
    } = restoration;
    if let Some(parent) = worktree.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tree = worktree.to_string_lossy().to_string();
    if let Err(error) = run_in(
        target,
        clone,
        "git",
        &["worktree", "add", "--detach", &tree, remote_tip],
    ) {
        return Failed(error);
    }
    let mut checkout = vec!["checkout", base, "--"];
    checkout.extend(files.iter().map(String::as_str));
    if let Err(error) = run_in(target, worktree, "git", &checkout) {
        return Failed(error);
    }
    let mut status = vec!["status", "--porcelain", "--"];
    status.extend(files.iter().map(String::as_str));
    match run_in(target, worktree, "git", &status) {
        Ok(changes) if changes.is_empty() => {
            return Refused(format!(
                "the files already equal {base}: nothing to restore"
            ))
        }
        Ok(_) => {}
        Err(error) => return Failed(error),
    }
    let title = format!("{RESTORE_COMMIT_PREFIX} ({})", target.incident.candidate.id);
    let message = format!("{title}\n\n- {}", files.join("\n- "));
    let mut add = vec!["add", "-A", "--"];
    add.extend(files.iter().map(String::as_str));
    let mut commit = HARNESS_AUTHOR.to_vec();
    commit.extend(["commit", "-m", &message]);
    let committed = run_in(target, worktree, "git", &add)
        .and_then(|_| run_in(target, worktree, "git", &commit));
    if let Err(error) = committed {
        return Failed(error);
    }
    let destination = format!("HEAD:refs/heads/{branch}");
    if let Err(error) = run_in(
        target,
        worktree,
        "git",
        &["push", HARNESS_REMOTE, &destination],
    ) {
        return Failed(error);
    }
    Done(format!(
        "{} test-infrastructure files restored from {base} on {branch}: {}",
        files.len(),
        files.join(", ")
    ))
}

/// True when `action` cured an infrastructure cause for this category: the bugs closed before
/// stop counting for the ticket.
pub fn cures_infrastructure(action: DoctorAction, category: DoctorCategory) -> bool {
    matches!(
        (action, category),
        (
            DoctorAction::RestoreTestInfra,
            DoctorCategory::TestInfraRegression
        ) | (
            DoctorAction::UpdatePrBranch,
            DoctorCategory::BranchBehindMain
        ) | (
            DoctorAction::OpenHarnessPr,
            DoctorCategory::SharedHarnessFailure
        )
    )
}

/// What came after an infrastructure cure.
pub struct FollowUp {
    pub note: String,
    /// True when the bug cap of the ticket was re-armed by this follow-up.
    pub rearmed: bool,
}

/// Re-arms the bug cap, then reruns the test of the ticket: the dead flow is aborted and the
/// ticket goes back to the test lane. A harness patch only re-arms: a human merges the patch
/// before any test can pass. The cap is re-armed once per ticket over 24 hours.
pub fn follow_up(target: &ActionTarget<'_>, action: DoctorAction) -> FollowUp {
    let candidate = &target.incident.candidate;
    let relaunch = action != DoctorAction::OpenHarnessPr;
    if relaunch {
        let terminal = target
            .engine
            .executor()
            .store()
            .index_get(&candidate.key)
            .is_none_or(|entry| entry.status.is_terminal());
        if !terminal {
            return FollowUp {
                note: format!(
                    "`{}` is still going: the test is not relaunched",
                    candidate.key
                ),
                rearmed: false,
            };
        }
    }
    let mut tickets = vec![candidate.ticket.clone()];
    if !relaunch {
        for affected in &target.incident.affected {
            if !tickets.contains(affected) {
                tickets.push(affected.clone());
            }
        }
    }
    let mut notes = Vec::new();
    let mut rearmed = false;
    if target.rearm_allowed {
        for ticket in &tickets {
            match rearm_cap(target, ticket) {
                Ok(true) => {
                    rearmed = true;
                    notes.push(format!("bug cap of `{ticket}` re-armed"));
                }
                Ok(false) => notes.push(format!("bug cap of `{ticket}` was already re-armed")),
                Err(error) => notes.push(format!("bug cap of `{ticket}` not re-armed: {error}")),
            }
        }
    } else {
        notes.push("bug cap already re-armed in the last 24 h".to_string());
    }
    if relaunch {
        notes.push(match abort_dead_flow(target) {
            Done(detail) => format!("test relaunched: {detail}"),
            Failed(detail) | Refused(detail) => format!("test not relaunched: {detail}"),
        });
    }
    FollowUp {
        note: notes.join("; "),
        rearmed,
    }
}

/// `aidp flow bug-rearm KEY`: true when the cap was re-armed, false when the guard window refused.
fn rearm_cap(target: &ActionTarget<'_>, ticket: &str) -> Result<bool, String> {
    let action = SubprocessAction::new([VERB_FLOW, VERB_FLOW_BUG_REARM, PLACEHOLDER_KEY_ARG]);
    let runner = target.engine.executor().ports().runner.clone();
    let context = ActionContext {
        key: ticket,
        workspace: None,
        runner: runner.as_ref(),
    };
    match action.run(&context, &BTreeMap::new()) {
        ActionOutcome::Ok(outputs) => {
            Ok(outputs.get("rearmed").and_then(Value::as_bool) != Some(false))
        }
        other => Err(format!("{other:?}")),
    }
}

/// The repository a patch targets: the diagnosis names it, else its paths tell.
fn harness_repo(target: &ActionTarget<'_>, patch: &str) -> Option<String> {
    if let Some(named) = text_param(target, PARAM_HARNESS_REPO) {
        return HARNESS_REPOS.contains(&named.as_str()).then_some(named);
    }
    let paths = patch_paths(patch);
    let all = |prefix: &str| !paths.is_empty() && paths.iter().all(|path| path.starts_with(prefix));
    if all("crates/") || all("docs/") {
        return Some(HARNESS_REPOS[0].to_string());
    }
    if all("templates/") || all("agents/") || all("skills/") {
        return Some(HARNESS_REPOS[1].to_string());
    }
    None
}

/// The files a unified diff touches.
pub fn patch_paths(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ "))
        .map(|path| path.split('\t').next().unwrap_or(path).trim())
        .filter(|path| *path != "/dev/null")
        .map(|path| path.strip_prefix("b/").unwrap_or(path).to_string())
        .collect()
}

fn unsafe_patch_path(path: &str) -> bool {
    path.starts_with('/')
        || path.contains(':')
        || path
            .split(['/', '\\'])
            .any(|part| part == ".." || part == ".git")
}

/// Runs `program args` in `cwd`; the trimmed stdout, or the error with stderr.
fn run_in(
    target: &ActionTarget<'_>,
    cwd: &Path,
    program: &str,
    args: &[&str],
) -> Result<String, String> {
    let mut spec = ProcessSpec::new(program).args(args.iter().map(|part| part.to_string()));
    spec.cwd = Some(cwd.to_path_buf());
    let runner = target.engine.executor().ports().runner.clone();
    let output = runner
        .output(&spec)
        .map_err(|error| format!("{program} {}: {error}", args.join(" ")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "{program} {} exited {:?}: {}",
            args.join(" "),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Opens a pull request for a harness patch: a temporary worktree cut from `origin/main` of the
/// repository, the patch applied, the branch `doctor/<incident>` pushed, the PR opened. Never a
/// merge, never a force-push, never a `gamedev/*` branch. A patch that does not apply opens
/// nothing: the caller turns the refusal into a note and a human.
fn open_harness_pr(target: &ActionTarget<'_>) -> DoctorActionResult {
    if target.incident.pr_url.is_some() {
        return Refused("a pull request is already open for this incident".to_string());
    }
    let Some(patch) = target
        .diagnosis
        .proposed_patch
        .as_deref()
        .map(str::trim)
        .filter(|patch| !patch.is_empty())
    else {
        return Refused("no proposed patch".to_string());
    };
    let paths = patch_paths(patch);
    if paths.is_empty() || paths.iter().any(|path| unsafe_patch_path(path)) {
        return Refused("the patch names no file, or a file outside the repository".to_string());
    }
    let Some(repo) = harness_repo(target, patch) else {
        return Refused("the patch targets no known harness repository".to_string());
    };
    let Some(clone) = (target.harness_repos)(&repo) else {
        return Refused(format!("no local clone of `{repo}`"));
    };
    let name = crate::doctor::bundle_name(&target.incident.candidate.id);
    let branch = format!("{HARNESS_BRANCH_PREFIX}{name}");
    if branch.starts_with(FORBIDDEN_BRANCH_PREFIX) {
        return Refused(format!("branch `{branch}` is forbidden"));
    }
    let worktree: PathBuf = target.state_dir.join(WORKTREES_DIR).join(&name);
    let base = format!("{HARNESS_REMOTE}/{HARNESS_BASE}");
    let outcome = push_patch(target, &clone, &worktree, &branch, &base, &repo, patch);
    // The temporary worktree never outlives the action.
    let _ = run_in(
        target,
        &clone,
        "git",
        &["worktree", "remove", "--force", &worktree.to_string_lossy()],
    );
    let _ = std::fs::remove_dir_all(&worktree);
    outcome
}

fn push_patch(
    target: &ActionTarget<'_>,
    clone: &Path,
    worktree: &Path,
    branch: &str,
    base: &str,
    repo: &str,
    patch: &str,
) -> DoctorActionResult {
    if let Some(parent) = worktree.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tree = worktree.to_string_lossy().to_string();
    if let Err(error) = run_in(
        target,
        clone,
        "git",
        &["fetch", HARNESS_REMOTE, HARNESS_BASE],
    ) {
        return Failed(error);
    }
    if let Err(error) = run_in(
        target,
        clone,
        "git",
        &["worktree", "add", "-b", branch, &tree, base],
    ) {
        return Failed(error);
    }
    let patch_file = worktree.with_extension("patch");
    if std::fs::write(&patch_file, format!("{patch}\n")).is_err() {
        return Failed("the patch file was not written".to_string());
    }
    let file = patch_file.to_string_lossy().to_string();
    let applied = run_in(target, worktree, "git", &["apply", "--check", &file])
        .and_then(|_| run_in(target, worktree, "git", &["apply", &file]));
    let _ = std::fs::remove_file(&patch_file);
    if let Err(error) = applied {
        return Refused(format!("the patch does not apply on {base}: {error}"));
    }
    let title = format!("fix(doctor): {}", target.incident.candidate.id);
    let mut commit = HARNESS_AUTHOR.to_vec();
    commit.extend(["commit", "-m", &title]);
    let committed = run_in(target, worktree, "git", &["add", "-A"])
        .and_then(|_| run_in(target, worktree, "git", &commit));
    if let Err(error) = committed {
        return Failed(error);
    }
    if let Err(error) = run_in(
        target,
        worktree,
        "git",
        &["push", "--set-upstream", HARNESS_REMOTE, branch],
    ) {
        return Failed(error);
    }
    let body = format!(
        "Patch proposé par le docteur aidp pour l'incident `{}`.\n\n{}\n\nÀ relire : le docteur ne fusionne jamais.",
        target.incident.candidate.id,
        target.diagnosis.cause.trim()
    );
    let created = run_in(
        target,
        worktree,
        GH_PROGRAM,
        &[
            "pr",
            "create",
            GH_REPO_FLAG,
            repo,
            "--base",
            HARNESS_BASE,
            "--head",
            branch,
            "--title",
            &title,
            "--body",
            &body,
        ],
    );
    match created {
        Ok(url) => Done(format!("pull request opened on {repo}: {url}")),
        Err(error) => Failed(format!(
            "branch `{branch}` pushed, pull request not opened: {error}"
        )),
    }
}
