//! The doctor's actions that work on git: pruning worktrees, merging `origin/main` into a pull
//! request branch for `.meta` conflicts, rebuilding a branch. None forces, none touches `main`.

use crate::doctor::actions::{checkout_for, run_in, ActionTarget};
use crate::doctor::converge::{self, Git};
use crate::doctor::detect::millis_of;
use crate::doctor::preconditions;
use crate::flows::facts;
use crate::models::constants::doctor::{
    FORBIDDEN_BRANCH_PREFIX, HARNESS_AUTHOR, HARNESS_BASE, HARNESS_REMOTE,
    META_MERGE_COMMIT_PREFIX, PROTECTED_BRANCHES, REBUILD_BRANCH_PREFIX, REBUILD_MAX_COMMITS,
    WORKTREES_DIR,
};
use crate::models::enums::DoctorActionResult;
use crate::worktrees::WORKTREE_DIR;
use std::path::{Path, PathBuf};

use DoctorActionResult::{Done, Failed, Refused};

const GIT: &str = "git";

/// The branch of the ticket's pull request: the one its `pr.json` names, else `feat/<ticket>`.
fn pull_request_branch(target: &ActionTarget<'_>) -> String {
    let candidate = &target.incident.candidate;
    target
        .engine
        .executor()
        .load(&candidate.key)
        .ok()
        .and_then(|(_, state)| facts::pr_artifact(&state))
        .and_then(|artifact| artifact.branch)
        .unwrap_or_else(|| converge::parent_branch(&candidate.ticket))
}

fn forbidden(branch: &str) -> bool {
    PROTECTED_BRANCHES.contains(&branch) || branch.starts_with(FORBIDDEN_BRANCH_PREFIX)
}

/// Runs `work` in a temporary detached worktree of `clone` cut from `start`; the worktree is
/// removed afterwards whatever `work` returned.
fn in_temporary_worktree(
    target: &ActionTarget<'_>,
    clone: &Path,
    name: &str,
    start: &str,
    work: impl FnOnce(&Path) -> DoctorActionResult,
) -> DoctorActionResult {
    let worktree: PathBuf = target.state_dir.join(WORKTREES_DIR).join(name);
    if let Some(parent) = worktree.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tree = worktree.to_string_lossy().to_string();
    if let Err(error) = run_in(
        target,
        clone,
        GIT,
        &["worktree", "add", "--detach", &tree, start],
    ) {
        return Failed(error);
    }
    let outcome = work(&worktree);
    let _ = run_in(
        target,
        clone,
        GIT,
        &["worktree", "remove", "--force", &tree],
    );
    let _ = std::fs::remove_dir_all(&worktree);
    outcome
}

/// Removes the worktrees of the flows of the project that are over for more than
/// `doctor.prune_worktree_hours`. `git worktree remove` without force: a dirty worktree stays.
pub(super) fn prune_worktrees(target: &ActionTarget<'_>) -> DoctorActionResult {
    let candidate = &target.incident.candidate;
    let Some(clone) = checkout_for(target.engine, &candidate.key) else {
        return Refused("no checkout for the project of the incident".to_string());
    };
    let executor = target.engine.executor();
    let now_ms = executor.ports().clock.now_ms();
    let hours = target.settings.prune_worktree_hours;
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    let mut tried = 0usize;
    for (key, entry) in executor.store().index() {
        if key == candidate.key {
            continue;
        }
        let Some(state) = executor.store().load_state(&entry.repo, &key) else {
            continue;
        };
        let same_project = executor
            .ports()
            .sessions
            .project_for(&state.ticket)
            .is_some_and(|project| project.path == clone);
        let ended_ms = state.ended_at.as_deref().and_then(millis_of);
        let active = target.engine.has_active_run(&key);
        if !same_project
            || !preconditions::worktree_prunable(entry.status, ended_ms, now_ms, hours, active)
        {
            continue;
        }
        let path = executor
            .store()
            .ticket_dir(&entry.repo, &key)
            .join(WORKTREE_DIR);
        if !path.exists() {
            continue;
        }
        tried += 1;
        match run_in(
            target,
            &clone,
            GIT,
            &["worktree", "remove", &path.to_string_lossy()],
        ) {
            Ok(_) => removed.push(key),
            Err(error) => kept.push(format!("{key}: {error}")),
        }
    }
    if tried == 0 {
        return Refused(format!(
            "no flow of the project is over for more than {hours} h with a worktree left"
        ));
    }
    let _ = run_in(target, &clone, GIT, &["worktree", "prune"]);
    if removed.is_empty() {
        return Failed(format!("no worktree removed: {}", kept.join("; ")));
    }
    Done(format!(
        "{} worktree(s) removed: {}; kept: {}",
        removed.len(),
        removed.join(", "),
        if kept.is_empty() {
            "none".to_string()
        } else {
            kept.join("; ")
        }
    ))
}

/// Merges `origin/main` into the pull request branch in a temporary worktree. Add/add conflicts
/// on Unity `.meta` files keep main's version; any other conflict refuses the action. Committed as
/// `aidp-doctor`, pushed normally to the branch.
pub(super) fn resolve_meta_conflict(target: &ActionTarget<'_>) -> DoctorActionResult {
    let candidate = &target.incident.candidate;
    let Some(clone) = checkout_for(target.engine, &candidate.key) else {
        return Refused("no checkout for the project of the ticket".to_string());
    };
    let branch = pull_request_branch(target);
    if forbidden(&branch) {
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
    let name = format!("{}-meta", crate::doctor::bundle_name(&candidate.id));
    in_temporary_worktree(target, &clone, &name, &remote_tip, |worktree| {
        merge_keeping_main_meta(target, worktree, &branch, &base)
    })
}

fn merge_keeping_main_meta(
    target: &ActionTarget<'_>,
    worktree: &Path,
    branch: &str,
    base: &str,
) -> DoctorActionResult {
    let merged = run_in(
        target,
        worktree,
        GIT,
        &["merge", "--no-commit", "--no-ff", base],
    );
    let status = match run_in(target, worktree, GIT, &["status", "--porcelain"]) {
        Ok(status) => status,
        Err(error) => return Failed(error),
    };
    let conflicts = preconditions::meta_conflicts(&status);
    let resolved = match conflicts {
        Err(blocking) => {
            let _ = run_in(target, worktree, GIT, &["merge", "--abort"]);
            return Refused(format!(
                "conflicts outside the Unity .meta files: {}",
                blocking.join(", ")
            ));
        }
        Ok(resolved) => resolved,
    };
    if resolved.is_empty() {
        let _ = run_in(target, worktree, GIT, &["merge", "--abort"]);
        return match merged {
            Ok(_) => Refused(format!("`{branch}` merges {base} without a conflict")),
            Err(error) => Failed(error),
        };
    }
    for path in &resolved {
        let staged = run_in(target, worktree, GIT, &["checkout", "--theirs", "--", path])
            .and_then(|_| run_in(target, worktree, GIT, &["add", "--", path]));
        if let Err(error) = staged {
            let _ = run_in(target, worktree, GIT, &["merge", "--abort"]);
            return Failed(error);
        }
    }
    let message = format!(
        "{META_MERGE_COMMIT_PREFIX} ({})\n\n- {}",
        target.incident.candidate.id,
        resolved.join("\n- ")
    );
    let mut commit = HARNESS_AUTHOR.to_vec();
    commit.extend(["commit", "-m", &message]);
    if let Err(error) = run_in(target, worktree, GIT, &commit) {
        return Failed(error);
    }
    let destination = format!("HEAD:refs/heads/{branch}");
    if let Err(error) = run_in(
        target,
        worktree,
        GIT,
        &["push", HARNESS_REMOTE, &destination],
    ) {
        return Failed(error);
    }
    Done(format!(
        "{base} merged into {branch}, {} .meta file(s) kept from main: {}",
        resolved.len(),
        resolved.join(", ")
    ))
}

/// Recreates the pull request branch from `origin/main` and replays its own commits on it in a
/// temporary worktree, then pushes the result to the new branch `doctor/rebuild/<incident>`. The
/// original branch and the pull request are left as they are.
pub(super) fn rebuild_branch(target: &ActionTarget<'_>) -> DoctorActionResult {
    let candidate = &target.incident.candidate;
    if candidate.ticket.is_empty() {
        return Refused("the incident has no ticket".to_string());
    }
    let Some(clone) = checkout_for(target.engine, &candidate.key) else {
        return Refused("no checkout for the project of the ticket".to_string());
    };
    let branch = pull_request_branch(target);
    if forbidden(&branch) {
        return Refused(format!("branch `{branch}` is forbidden"));
    }
    let name = crate::doctor::bundle_name(&candidate.id);
    let rebuilt = format!("{REBUILD_BRANCH_PREFIX}{name}");
    if forbidden(&rebuilt) {
        return Refused(format!("branch `{rebuilt}` is forbidden"));
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
    let range = format!("{base}..{remote_tip}");
    let commits: Vec<String> = match git.run(&["rev-list", "--reverse", "--no-merges", &range]) {
        Ok(list) => list.lines().map(str::to_string).collect(),
        Err(error) => return Failed(error),
    };
    if commits.is_empty() {
        return Refused(format!("`{branch}` holds no commit of its own past {base}"));
    }
    if commits.len() > REBUILD_MAX_COMMITS {
        return Refused(format!(
            "`{branch}` holds {} commits: rebuilding is limited to {REBUILD_MAX_COMMITS}",
            commits.len()
        ));
    }
    let work_name = format!("{name}-rebuild");
    in_temporary_worktree(target, &clone, &work_name, &base, |worktree| {
        for sha in &commits {
            let mut pick = HARNESS_AUTHOR.to_vec();
            pick.extend(["cherry-pick", sha.as_str()]);
            if let Err(error) = run_in(target, worktree, GIT, &pick) {
                let _ = run_in(target, worktree, GIT, &["cherry-pick", "--abort"]);
                return Refused(format!("commit {sha} does not apply on {base}: {error}"));
            }
        }
        let destination = format!("HEAD:refs/heads/{rebuilt}");
        match run_in(
            target,
            worktree,
            GIT,
            &["push", HARNESS_REMOTE, &destination],
        ) {
            Ok(_) => Done(format!(
                "{} commit(s) of {branch} replayed on {base} and pushed to {rebuilt}",
                commits.len()
            )),
            Err(error) => Failed(error),
        }
    })
}
