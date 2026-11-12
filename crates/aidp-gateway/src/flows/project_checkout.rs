//! Flows with `workspace: project` run in the main checkout of the project, on the branch of the
//! ticket, with no worktree. One run at a time holds the project's `unity-editor` resource, from
//! `git.checkout` until the checkout is given back (`git.restore`, in the flow or by the daemon
//! when the run ends). The hold is derived from the run state, like every resource.

use super::Executor;
use crate::flows::registry::ActionContext;
use crate::models::constants::flow_actions::{
    ACTION_GIT_CHECKOUT, ACTION_GIT_PUSH, ACTION_GIT_RESTORE,
};
use crate::models::ActionJob;
use crate::unity_lock::{self, Acquire};
use aidp_flows::models::constants::resource;
use aidp_flows::models::enums::WorkspaceMode;
use aidp_flows::{ActionOutcome, FlowState, StepStatus, MANUAL_PREFIX};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Marker in `effects_applied`: the checkout was given back, the resource is free.
pub(super) const RELEASED_MARKER: &str = "project-checkout:released";
/// Effect id of the restore the daemon runs when a run ends.
const END_RESTORE_EFFECT: &str = "manual-end-restore";

/// True for a run that works in the project's main checkout.
pub(super) fn is_project_flow(state: &FlowState) -> bool {
    state.def.workspace == Some(WorkspaceMode::Project)
}

fn needs_checkout(action: &str) -> bool {
    action == ACTION_GIT_CHECKOUT || action == ACTION_GIT_PUSH
}

/// True when the current step of `state` is parked on a resource.
fn parked_on_resource(state: &FlowState) -> bool {
    state
        .current
        .as_ref()
        .and_then(|rid| state.steps.get(rid))
        .is_some_and(|step| {
            step.status == StepStatus::WaitingExternal
                && step
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.starts_with(resource::REASON_PREFIX))
        })
}

/// True when a project run waits for the resource in an action step.
pub(super) fn waits_for_project(state: &FlowState) -> bool {
    is_project_flow(state) && parked_on_resource(state)
}

impl Executor {
    /// True while a project run holds, or waits for, the project's main checkout: it counts in the
    /// test lane. A run past its `git.restore` (waiting for the merge) no longer does.
    pub fn project_checkout_held(&self, state: &FlowState) -> bool {
        is_project_flow(state)
            && !state.status.is_terminal()
            && !state.effects_applied.contains(RELEASED_MARKER)
    }

    /// The directory a project flow works in: the test checkout of the project when one is set,
    /// else its main checkout.
    pub(super) fn project_checkout_path(&self, state: &FlowState) -> Option<PathBuf> {
        self.ports
            .sessions
            .test_checkout_for(&state.ticket)
            .or_else(|| {
                self.ports
                    .sessions
                    .project_for(&state.ticket)
                    .map(|project| project.path)
            })
    }

    /// The resource a project flow holds, `unity-editor:<project id>`.
    pub(super) fn project_resource(&self, state: &FlowState) -> Option<String> {
        let project = self.ports.sessions.project_for(&state.ticket)?;
        Some(unity_lock::resource_key(
            resource::UNITY_EDITOR,
            &project.id,
        ))
    }

    /// What a project flow does with its resource: `(holds, waits for)`.
    pub(super) fn project_role(&self, state: &FlowState) -> (Option<String>, Option<String>) {
        let none = (None, None);
        if state.is_fresh() || state.effects_applied.contains(RELEASED_MARKER) {
            return none;
        }
        let Some(key) = self.project_resource(state) else {
            return none;
        };
        if parked_on_resource(state) {
            (None, Some(key))
        } else {
            (Some(key), None)
        }
    }

    /// Before an action runs: `git.checkout` and `git.push` of a project flow take the resource.
    /// `Some(outcome)` ends the job with it (resource busy, or the checkout could not be redone).
    pub(super) fn project_gate(self: &Arc<Self>, job: &ActionJob) -> Option<ActionOutcome> {
        if job.effect_id.starts_with(MANUAL_PREFIX) || !needs_checkout(&job.name) {
            return None;
        }
        let (_, state) = self.load(&job.key).ok()?;
        if !is_project_flow(&state) {
            return None;
        }
        let resource_key = self.project_resource(&state)?;
        let (answer, change) = self
            .resources
            .acquire(&resource_key, self.resource_holder(&state));
        self.announce(change.into_iter().collect());
        if answer == Acquire::Busy {
            return Some(ActionOutcome::WaitingExternal {
                reason: format!("{}{}", resource::REASON_PREFIX, resource::UNITY_EDITOR),
                outputs: BTreeMap::new(),
            });
        }
        if job.name == ACTION_GIT_CHECKOUT {
            return None;
        }
        self.reenter_project_checkout(&job.repo, &state, false)
    }

    /// A run retried after the daemon gave its checkout back checks its branch out again.
    /// `Some(outcome)` when that failed. `key_lock_held`: the caller already holds the key lock of
    /// the run (a pipeline start, from a transition or a reconcile), and it is not reentrant.
    pub(super) fn reenter_project_checkout(
        self: &Arc<Self>,
        repo: &str,
        state: &FlowState,
        key_lock_held: bool,
    ) -> Option<ActionOutcome> {
        if !state.effects_applied.contains(RELEASED_MARKER) {
            return None;
        }
        let outcome = self.run_registered(state, ACTION_GIT_CHECKOUT);
        if matches!(outcome, ActionOutcome::Ok(_)) {
            if key_lock_held {
                self.write_released(repo, &state.key, false);
            } else {
                self.set_released(repo, &state.key, false);
            }
            return None;
        }
        Some(outcome)
    }

    fn run_registered(&self, state: &FlowState, name: &str) -> ActionOutcome {
        let Some(action) = self.registry.action(name) else {
            return ActionOutcome::TechnicalError(format!("unknown action `{name}`"));
        };
        let workspace = self.project_checkout_path(state);
        let context = ActionContext {
            key: state.key.as_str(),
            workspace: workspace.as_deref(),
            runner: &self.action_processes,
        };
        action.run(&context, &BTreeMap::new())
    }

    /// Sets or clears the marker that tells the checkout was given back.
    /// Under the key lock: a transition of the run saves its state from a copy loaded earlier.
    pub(super) fn set_released(&self, repo: &str, key: &str, released: bool) {
        let guard = self.key_lock(key);
        let _held = crate::sync::lock(&guard);
        self.write_released(repo, key, released);
    }

    /// [`Self::set_released`] for a caller that already holds the key lock.
    fn write_released(&self, repo: &str, key: &str, released: bool) {
        let Ok((_, mut state)) = self.load(key) else {
            return;
        };
        let changed = if released {
            state.effects_applied.insert(RELEASED_MARKER.to_string())
        } else {
            state.effects_applied.remove(RELEASED_MARKER)
        };
        if changed {
            if let Err(error) = self.store.save_state(repo, &state, &self.now()) {
                tracing::warn!(key, %error, "project checkout marker not saved");
            }
        }
    }

    /// After a job: `git.checkout` done clears the marker, `git.restore` done (or the restore of a
    /// run's end, whatever its outcome) sets it and frees the resource.
    pub(super) fn project_job_done(self: &Arc<Self>, job: &ActionJob, outcome: &ActionOutcome) {
        let ok = matches!(outcome, ActionOutcome::Ok(_));
        if job.name == ACTION_GIT_CHECKOUT && ok {
            self.set_released(&job.repo, &job.key, false);
        }
        let ended = job.effect_id == END_RESTORE_EFFECT;
        if job.name == ACTION_GIT_RESTORE && ended && !ok {
            // The checkout may still be dirty: keep it held (no release), say so once, retry later.
            let first = crate::sync::lock(&self.restore_failed).insert(job.key.clone());
            tracing::warn!(key = %job.key, "end-of-run git.restore failed: test checkout may be dirty, retrying");
            if first {
                self.journal(
                    &job.key,
                    None,
                    "end-of-run git.restore failed: the test checkout may be left dirty; retried on every dispatcher tick, no new test flow starts before it succeeds".to_string(),
                );
            }
            return;
        }
        if job.name == ACTION_GIT_RESTORE && (ok || ended) {
            crate::sync::lock(&self.restore_failed).remove(&job.key);
            self.set_released(&job.repo, &job.key, true);
            if let Ok((_, state)) = self.load(&job.key) {
                self.sync_resources(&state);
            }
        }
    }

    /// Keys of the ended project runs whose checkout restore is still to be redone.
    pub fn failed_restores(&self) -> Vec<String> {
        crate::sync::lock(&self.restore_failed)
            .iter()
            .cloned()
            .collect()
    }

    /// Redoes, once, the end-of-run restore of every project run whose restore failed.
    pub fn retry_failed_restores(self: &Arc<Self>) {
        for key in self.failed_restores() {
            let Ok((repo, state)) = self.load(&key) else {
                crate::sync::lock(&self.restore_failed).remove(&key);
                continue;
            };
            if state.effects_applied.contains(RELEASED_MARKER) {
                crate::sync::lock(&self.restore_failed).remove(&key);
                continue;
            }
            self.spawn_action(ActionJob {
                key,
                repo,
                effect_id: END_RESTORE_EFFECT.to_string(),
                name: ACTION_GIT_RESTORE.to_string(),
                with: BTreeMap::new(),
            });
        }
    }

    /// A project run that ended gives its checkout back, whatever the way it ended.
    pub(super) fn end_project_checkout(self: &Arc<Self>, repo: &str, state: &FlowState) {
        if !is_project_flow(state)
            || !state.status.is_terminal()
            || state.is_fresh()
            || state.effects_applied.contains(RELEASED_MARKER)
        {
            return;
        }
        self.spawn_action(ActionJob {
            key: state.key.clone(),
            repo: repo.to_string(),
            effect_id: END_RESTORE_EFFECT.to_string(),
            name: ACTION_GIT_RESTORE.to_string(),
            with: BTreeMap::new(),
        });
    }
}
