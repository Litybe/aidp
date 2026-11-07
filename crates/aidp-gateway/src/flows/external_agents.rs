//! Agent jobs of a flow: the sessions of its pipeline runs that a host launches. They sit in the
//! same listing as the actions and probes, and the host claims, renews and settles them by flow key.

use super::executor::Executor;
use super::external::accepts_work;
use super::store::Runtime;
use super::{FlowEngine, FlowEngineError};
use crate::models::action_job::ID_SEPARATOR;
use crate::models::agent_claim::AgentClaim;
use crate::models::agent_claim_request::AgentClaimRequest;
use crate::models::agent_heartbeat::AgentHeartbeat;
use crate::models::agent_heartbeat_request::AgentHeartbeatRequest;
use crate::models::agent_job_ack::AgentJobAck;
use crate::models::agent_job_result::AgentJobResult;
use crate::models::constants::agent_jobs::ERROR_FOREIGN_JOB;
use crate::models::constants::flow_external::{FLOW_NOT_ACTIVE, UNKNOWN_JOB};
use crate::models::enums::{ExternalJobKind, SessionState};
use crate::models::ExternalJob;
use crate::native_runs::agent_board::parse_job_id;
use std::collections::BTreeMap;

/// The flow step an effect id belongs to; the whole id when it has no step part.
fn step_of(effect: &str) -> String {
    let mut parts = effect.rsplitn(3, ID_SEPARATOR);
    match (parts.next(), parts.next(), parts.next()) {
        (_, _, Some(step)) => step.to_string(),
        _ => effect.to_string(),
    }
}

impl Executor {
    /// The pipeline runs of the flow still executing, with the flow step that started each.
    fn executing_runs(&self, runtime: &Runtime) -> Vec<(String, String)> {
        runtime
            .sessions
            .iter()
            .filter(|(_, run_id)| {
                matches!(self.ports.sessions.state(run_id), SessionState::Running)
            })
            .map(|(effect, run_id)| (step_of(effect), run_id.clone()))
            .collect()
    }

    /// The pending agent jobs of the flow's executing runs, as listing items.
    pub(super) fn agent_items(
        &self,
        repo: &str,
        ticket_key: &str,
        local: bool,
        runtime: &Runtime,
    ) -> Vec<ExternalJob> {
        let mut items = Vec::new();
        for (step, run_id) in self.executing_runs(runtime) {
            for summary in self.ports.sessions.agent_jobs(&run_id) {
                items.push(ExternalJob {
                    id: summary.id,
                    kind: ExternalJobKind::Agent,
                    step: step.clone(),
                    name: summary.name,
                    attempt: summary.attempt,
                    manual: false,
                    writes: summary.writes,
                    with: BTreeMap::new(),
                    ticket_key: ticket_key.to_string(),
                    local,
                    repo: repo.to_string(),
                    workspace: Some(summary.workspace.to_string_lossy().to_string()),
                    repo_config: None,
                    created_at: summary.created_at,
                    started_ms: None,
                    now_ms: None,
                    state: None,
                    agent: Some(summary.agent),
                });
            }
        }
        items
    }

    /// True while a host holds a lease on an agent job of the flow.
    pub(super) fn has_claimed_agent(&self, runtime: &Runtime) -> bool {
        self.executing_runs(runtime)
            .iter()
            .any(|(_, run_id)| self.ports.sessions.has_claimed_agent_job(run_id))
    }

    /// The run `job_id` belongs to, when the flow `key` started that run.
    fn agent_run_of(&self, key: &str, job_id: &str) -> anyhow::Result<(String, bool)> {
        let (repo, state) = self.load(key)?;
        let runtime = self.store.load_runtime(&repo, key);
        let (owner, _) = parse_job_id(job_id)
            .ok_or_else(|| FlowEngineError::NotFound(format!("{UNKNOWN_JOB} `{job_id}`")))?;
        if !runtime.sessions.values().any(|run_id| run_id == owner) {
            return Err(
                FlowEngineError::NotFound(format!("{ERROR_FOREIGN_JOB} `{job_id}`")).into(),
            );
        }
        Ok((owner.to_string(), accepts_work(&state)))
    }
}

impl FlowEngine {
    /// Gives an agent job of flow `key` to a host. A paused or finished flow answers a conflict.
    pub fn agent_claim(
        &self,
        key: &str,
        job_id: &str,
        request: &AgentClaimRequest,
    ) -> anyhow::Result<AgentClaim> {
        let (run_id, active) = self.executor.agent_run_of(key, job_id)?;
        if !active {
            return Err(FlowEngineError::Conflict {
                message: FLOW_NOT_ACTIVE.to_string(),
                existing: None,
            }
            .into());
        }
        self.executor
            .ports
            .sessions
            .claim_agent_job(&run_id, job_id, request)
    }

    /// Renews the lease of an agent job of flow `key`.
    pub fn agent_heartbeat(
        &self,
        key: &str,
        job_id: &str,
        request: &AgentHeartbeatRequest,
    ) -> anyhow::Result<AgentHeartbeat> {
        let (run_id, _) = self.executor.agent_run_of(key, job_id)?;
        self.executor
            .ports
            .sessions
            .heartbeat_agent_job(&run_id, job_id, request)
    }

    /// Judges the result a host posted for an agent job of flow `key`.
    pub fn agent_result(
        &self,
        key: &str,
        job_id: &str,
        result: &AgentJobResult,
    ) -> anyhow::Result<AgentJobAck> {
        let (run_id, _) = self.executor.agent_run_of(key, job_id)?;
        self.executor
            .ports
            .sessions
            .finish_agent_job(&run_id, job_id, result)
    }
}
