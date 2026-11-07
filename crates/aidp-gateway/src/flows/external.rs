//! External executor: the jobs a run hands to its caller and the results the caller posts back.
//! A job is derived from the persisted state, never stored, except manual actions.

use super::executor::{step_at_cursor, Executor};
use super::lock::workspace_key;
use super::store::{Runtime, WaitRecord};
use super::FlowEngineError;
use crate::models::action_job::ID_SEPARATOR;
use crate::models::constants::flow_actions::LOCAL_KEY_SEPARATOR;
use crate::models::constants::flow_external::{
    DEFAULT_ERROR_REASON, DEFAULT_WAITING_REASON, FLOW_NOT_ACTIVE, KIND_MISMATCH,
    MAX_RESULT_DIGESTS, NOT_EXTERNAL, NOT_PENDING, OUTPUTS_NOT_OBJECT, SKIPPED_NEEDS_REASON,
    UNKNOWN_JOB, UNKNOWN_OUTCOME,
};
use crate::models::enums::{
    BuiltinAction, BuiltinProbe, ExternalJobKind, ExternalOutcome, FlowExecutorMode,
};
use crate::models::{ActionJob, ExternalDigest, ExternalJob, ExternalManualEntry, ExternalResult};
use crate::sync::lock;
use aidp_flows::{
    effect_id, ActionOutcome, EffectName, Event, FlowState, FlowStatus, StepKind, StepReason,
    StepStatus, MANUAL_PREFIX,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// An action with no I/O: it runs in the gateway even when the caller runs the rest.
pub(super) fn is_internal_action(name: &str) -> bool {
    name == BuiltinAction::Noop.as_str() || name == BuiltinAction::Fail.as_str()
}

/// A probe with no I/O: the gateway polls it even when the caller runs the rest.
pub(super) fn is_internal_probe(name: &str) -> bool {
    name == BuiltinProbe::Timer.as_str()
}

/// What the caller is asked to run, before its `with` is completed.
struct Target {
    id: String,
    kind: ExternalJobKind,
    step: String,
    name: String,
    attempt: u32,
    manual: bool,
    with: BTreeMap<String, Value>,
    created_at: String,
    started_ms: Option<i64>,
    state: Option<Value>,
}

fn is_external(runtime: &Runtime) -> bool {
    runtime
        .start_request
        .as_ref()
        .and_then(|request| request.policy.as_ref())
        .is_some_and(|policy| policy.executor == FlowExecutorMode::External)
}

/// True while the run lets work happen: not paused, not pausing, not finished.
pub(super) fn accepts_work(state: &FlowState) -> bool {
    !matches!(state.status, FlowStatus::Paused | FlowStatus::Pausing) && !state.status.is_terminal()
}

/// True when `id` names an effect of a step the run has, or of a manual request it issued.
fn is_known_id(state: &FlowState, id: &str) -> bool {
    let mut parts = id.rsplitn(3, ID_SEPARATOR);
    let (_kind, _iteration, Some(step)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    state.steps.contains_key(step)
        || step
            .strip_prefix(MANUAL_PREFIX)
            .and_then(|index| index.parse::<u32>().ok())
            .is_some_and(|index| index < state.manual_count)
}

/// The settings a job carries from the workspace flow config; `None` when it cannot be read.
fn repo_config_of(workspace: &Path) -> Option<Value> {
    let config = aidp_integrations::flow_repo::load(workspace).ok()?;
    serde_json::to_value(config).ok()
}

/// A stable fingerprint of what the caller reported, without the actor.
fn digest_of(result: &ExternalResult) -> String {
    let text = json!({
        "outcome": result.outcome,
        "outputs": result.outputs,
        "reason": result.reason,
        "state": result.state,
    })
    .to_string();
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn conflict(message: String) -> anyhow::Error {
    FlowEngineError::Conflict {
        message,
        existing: None,
    }
    .into()
}

fn outputs_of(result: &ExternalResult) -> anyhow::Result<BTreeMap<String, Value>> {
    match &result.outputs {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => Ok(map.clone().into_iter().collect()),
        Some(_) => anyhow::bail!(OUTPUTS_NOT_OBJECT),
    }
}

impl Executor {
    /// True when the run's caller, not the gateway, runs action `name`.
    pub(super) fn external_action(&self, repo: &str, key: &str, name: &str) -> bool {
        !is_internal_action(name) && is_external(&self.store.load_runtime(repo, key))
    }

    /// True when the run's caller, not the gateway, polls probe `name`.
    pub(super) fn external_probe(&self, repo: &str, key: &str, name: &str) -> bool {
        !is_internal_probe(name) && is_external(&self.store.load_runtime(repo, key))
    }

    /// Keeps a manual action for the caller, under the effect id the machine gave it.
    pub(super) fn hold_manual(
        &self,
        repo: &str,
        key: &str,
        effect: &str,
        entry: ExternalManualEntry,
    ) -> anyhow::Result<()> {
        let mut runtime = self.store.load_runtime(repo, key);
        runtime.external_manual.insert(effect.to_string(), entry);
        self.store.save_runtime(repo, key, &runtime)
    }

    fn pending_targets(&self, state: &FlowState, runtime: &Runtime) -> Vec<Target> {
        let mut targets = Vec::new();
        if let (Some(step_id), Some(step)) = (state.current.as_ref(), step_at_cursor(state)) {
            if let Some(step_state) = state.steps.get(step_id) {
                let iteration = step_state.iteration;
                let created_at = step_state.started_at.clone().unwrap_or_else(|| self.now());
                let name = step.use_.clone().unwrap_or_default();
                let waiting_for_caller = step_state.status == StepStatus::WaitingExternal
                    && step_state.reason.as_deref() == Some(StepReason::ActionWaiting.as_str());
                match (step.kind, step_state.status) {
                    (StepKind::Action, StepStatus::Running | StepStatus::WaitingExternal)
                        if (step_state.status == StepStatus::Running || waiting_for_caller)
                            && !is_internal_action(&name) =>
                    {
                        targets.push(Target {
                            id: effect_id(step_id, iteration, EffectName::RunAction.as_str()),
                            kind: ExternalJobKind::Action,
                            step: step_id.clone(),
                            name,
                            attempt: iteration,
                            manual: false,
                            with: step.with.clone(),
                            created_at,
                            started_ms: None,
                            state: None,
                        });
                    }
                    (StepKind::Wait, StepStatus::WaitingExternal) => {
                        let id = effect_id(step_id, iteration, EffectName::StartWait.as_str());
                        if let Some(record) = runtime.waits.get(&id) {
                            if !is_internal_probe(&record.probe) {
                                targets.push(probe_target(id, record, iteration, created_at));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for (id, entry) in &runtime.external_manual {
            targets.push(Target {
                id: id.clone(),
                kind: ExternalJobKind::Action,
                step: entry.step.clone(),
                name: entry.name.clone(),
                attempt: 0,
                manual: true,
                with: entry.with.clone(),
                created_at: entry.created_at.clone(),
                started_ms: None,
                state: None,
            });
        }
        targets
    }

    /// The jobs run `key` waits on, oldest first, with the agent sessions a host launches. Nothing
    /// while the run is paused, aborted or finished, no writer job while the workspace is busy, and
    /// no writer manual action while a host holds an agent job.
    pub(super) fn external_jobs(&self, key: &str) -> anyhow::Result<Vec<ExternalJob>> {
        let (repo, state) = self.load(key)?;
        let runtime = self.store.load_runtime(&repo, key);
        if !is_external(&runtime) || !accepts_work(&state) {
            return Ok(Vec::new());
        }
        let targets = self.pending_targets(&state, &runtime);
        let vault = self.store.vault_root();
        let busy = self.ports.lock.is_busy(&workspace_key(vault, &repo, key));
        let workspace = crate::worktrees::worktree_path(vault, &repo, key);
        let repo_config = repo_config_of(&workspace);
        let config = aidp_tickets::config::load(vault);
        let local_marker = format!("{}{LOCAL_KEY_SEPARATOR}", config.local.key_prefix);
        let now_ms = self.ports.clock.now_ms();
        let mut jobs = self.agent_items(&repo, key, key.starts_with(&local_marker), &runtime);
        let agent_claimed =
            targets.iter().any(|target| target.manual) && self.has_claimed_agent(&runtime);
        for target in targets {
            let action = self.registry().action(&target.name);
            let writes = target.kind == ExternalJobKind::Action
                && action.as_ref().is_some_and(|action| action.writes());
            if writes && (busy || (target.manual && agent_claimed)) {
                continue;
            }
            let ticket_key = key.to_string();
            let probe = target.kind == ExternalJobKind::Probe;
            let mut with = target.with.clone();
            if !probe {
                let job = ActionJob {
                    key: key.to_string(),
                    repo: repo.clone(),
                    effect_id: target.id.clone(),
                    name: target.name.clone(),
                    with: target.with.clone(),
                };
                let placeholders = action
                    .map(|action| action.placeholders())
                    .unwrap_or_default();
                with = self.with_facts(&job, placeholders, false);
            }
            jobs.push(ExternalJob {
                id: target.id,
                kind: target.kind,
                step: target.step,
                name: target.name,
                attempt: target.attempt,
                manual: target.manual,
                writes,
                with,
                local: ticket_key.starts_with(&local_marker),
                ticket_key,
                repo: repo.clone(),
                workspace: Some(workspace.to_string_lossy().to_string()),
                repo_config: repo_config.clone(),
                created_at: target.created_at,
                started_ms: target.started_ms,
                now_ms: probe.then_some(now_ms),
                state: target.state,
                agent: None,
            });
        }
        jobs.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(jobs)
    }

    /// Applies the caller's result to job `job_id`; true when it replayed an applied result.
    pub(super) fn apply_external_result(
        self: &Arc<Self>,
        key: &str,
        job_id: &str,
        result: &ExternalResult,
        actor: &str,
    ) -> anyhow::Result<bool> {
        let outcome = ExternalOutcome::parse(&result.outcome)
            .ok_or_else(|| anyhow::anyhow!("{UNKNOWN_OUTCOME} `{}`", result.outcome))?;
        let outputs = outputs_of(result)?;
        let reason = result
            .reason
            .clone()
            .filter(|reason| !reason.trim().is_empty());
        if outcome == ExternalOutcome::Skipped && reason.is_none() {
            anyhow::bail!(SKIPPED_NEEDS_REASON);
        }
        let _transition = self.transition()?;
        let guard = self.key_lock(key);
        let _held = lock(&guard);
        let (repo, state) = self.load(key)?;
        let runtime = self.store.load_runtime(&repo, key);
        if !is_external(&runtime) {
            return Err(conflict(NOT_EXTERNAL.to_string()));
        }
        let digest = digest_of(result);
        let recorded = runtime
            .external_results
            .iter()
            .find(|entry| entry.job_id == job_id)
            .map(|entry| entry.digest.clone());
        let Some(target) = self
            .pending_targets(&state, &runtime)
            .into_iter()
            .find(|target| target.id == job_id)
        else {
            return match recorded {
                Some(known) if known == digest => Ok(true),
                Some(_) => Err(conflict(format!(
                    "job `{job_id}` already has another result"
                ))),
                None if is_known_id(&state, job_id) => {
                    Err(conflict(format!("job `{job_id}` {NOT_PENDING}")))
                }
                None => Err(FlowEngineError::NotFound(format!("{UNKNOWN_JOB} `{job_id}`")).into()),
            };
        };
        if !accepts_work(&state) {
            return Err(conflict(FLOW_NOT_ACTIVE.to_string()));
        }
        if outcome.is_probe() != (target.kind == ExternalJobKind::Probe) {
            return Err(conflict(KIND_MISMATCH.to_string()));
        }
        let at = self.now();
        let replay = match target.kind {
            ExternalJobKind::Action => {
                let repeat = recorded.as_deref() == Some(digest.as_str());
                self.finish_external_action(
                    key, &target, outcome, outputs, reason, &state, &at, repeat,
                )?
            }
            ExternalJobKind::Probe => {
                self.finish_external_probe(&repo, key, &target, outcome, outputs, result, &at)?
            }
            ExternalJobKind::Agent => return Err(conflict(KIND_MISMATCH.to_string())),
        };
        if replay {
            return Ok(true);
        }
        let mut fresh = self.store.load_runtime(&repo, key);
        if target.manual && outcome != ExternalOutcome::WaitingExternal {
            fresh.external_manual.remove(job_id);
        }
        fresh
            .external_results
            .retain(|entry| entry.job_id != job_id);
        fresh.external_results.push(ExternalDigest {
            job_id: job_id.to_string(),
            digest,
        });
        let excess = fresh
            .external_results
            .len()
            .saturating_sub(MAX_RESULT_DIGESTS);
        fresh.external_results.drain(..excess);
        self.store.save_runtime(&repo, key, &fresh)?;
        if !matches!(
            outcome,
            ExternalOutcome::WaitingExternal | ExternalOutcome::Pending
        ) {
            self.journal(
                key,
                Some(actor),
                format!("external {} {} by {actor}", target.name, result.outcome),
            );
        }
        Ok(false)
    }

    /// Delivers the action outcome; true when the step already holds exactly that waiting state.
    #[allow(clippy::too_many_arguments)]
    fn finish_external_action(
        self: &Arc<Self>,
        key: &str,
        target: &Target,
        outcome: ExternalOutcome,
        outputs: BTreeMap<String, Value>,
        reason: Option<String>,
        state: &FlowState,
        at: &str,
        repeat: bool,
    ) -> anyhow::Result<bool> {
        let action_outcome = match outcome {
            ExternalOutcome::Ok => ActionOutcome::Ok(outputs),
            ExternalOutcome::Skipped => ActionOutcome::Skipped {
                reason: reason.unwrap_or_default(),
            },
            ExternalOutcome::WaitingExternal => {
                let reason = reason.unwrap_or_else(|| DEFAULT_WAITING_REASON.to_string());
                let held = (target.manual && repeat)
                    || state.steps.get(&target.step).is_some_and(|step| {
                        !target.manual
                            && step.status == StepStatus::WaitingExternal
                            && step.reason.as_deref() == Some(StepReason::ActionWaiting.as_str())
                            && step.detail.as_deref() == Some(reason.as_str())
                            && step.outputs == outputs
                    });
                if held {
                    return Ok(true);
                }
                ActionOutcome::WaitingExternal { reason, outputs }
            }
            ExternalOutcome::TechnicalError => ActionOutcome::TechnicalError(
                reason.unwrap_or_else(|| DEFAULT_ERROR_REASON.to_string()),
            ),
            ExternalOutcome::BusinessError => ActionOutcome::BusinessError(
                reason.unwrap_or_else(|| DEFAULT_ERROR_REASON.to_string()),
            ),
            ExternalOutcome::Satisfied | ExternalOutcome::Pending => {
                return Err(conflict(KIND_MISMATCH.to_string()));
            }
        };
        self.deliver_locked(
            key,
            Event::ActionFinished {
                effect_id: target.id.clone(),
                outcome: action_outcome,
                at: at.to_string(),
            },
        )?;
        Ok(false)
    }

    /// Delivers the probe outcome; true when the wait record already holds that state.
    #[allow(clippy::too_many_arguments)]
    fn finish_external_probe(
        self: &Arc<Self>,
        repo: &str,
        key: &str,
        target: &Target,
        outcome: ExternalOutcome,
        outputs: BTreeMap<String, Value>,
        result: &ExternalResult,
        at: &str,
    ) -> anyhow::Result<bool> {
        let mut runtime = self.store.load_runtime(repo, key);
        if outcome == ExternalOutcome::Pending {
            let same = runtime
                .waits
                .get(&target.id)
                .is_some_and(|record| record.state == result.state);
            if !same {
                self.remember_probe_state(
                    &mut runtime,
                    repo,
                    key,
                    &target.id,
                    result.state.clone(),
                )?;
            }
            return Ok(same);
        }
        runtime.waits.remove(&target.id);
        self.store.save_runtime(repo, key, &runtime)?;
        self.deliver_locked(
            key,
            Event::WaitSatisfied {
                effect_id: target.id.clone(),
                outputs,
                at: at.to_string(),
            },
        )?;
        Ok(false)
    }
}

fn probe_target(id: String, record: &WaitRecord, iteration: u32, created_at: String) -> Target {
    Target {
        id,
        kind: ExternalJobKind::Probe,
        step: record.step.clone(),
        name: record.probe.clone(),
        attempt: iteration,
        manual: false,
        with: record.with.clone(),
        created_at,
        started_ms: Some(record.started_ms),
        state: record.state.clone(),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/flow_external_tests.rs"]
mod tests;
