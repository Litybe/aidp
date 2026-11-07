//! A session port whose one run, [`RUN_ID`], has a pending agent job a test can claim and settle.

use crate::flows::registry::{PipelineLaunch, ProjectRef, SessionPort, SessionStart, SessionState};
use crate::models::agent_claim::AgentClaim;
use crate::models::agent_claim_request::AgentClaimRequest;
use crate::models::agent_heartbeat::AgentHeartbeat;
use crate::models::agent_heartbeat_request::AgentHeartbeatRequest;
use crate::models::agent_job_ack::AgentJobAck;
use crate::models::agent_job_listing::AgentJobListing;
use crate::models::agent_job_result::AgentJobResult;
use crate::models::agent_job_summary::AgentJobSummary;
use crate::models::enums::agent_job_kind::AgentJobKind;
use crate::native_runs::agent_job_builder::fixture::{sample_job, RUN_ID as FIXTURE_RUN_ID};
use crate::session_error::SessionError;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

pub const RUN_ID: &str = FIXTURE_RUN_ID;
pub const CLAIM_TOKEN: &str = "claim-token";
const LABEL: &str = "implement";
const CREATED_AT: &str = "2026-10-18T10:15:03Z";

/// The id of the one agent job of [`RUN_ID`].
pub fn job_id() -> String {
    format!("agent:{RUN_ID}:1")
}

pub struct AgentSessions {
    vault: PathBuf,
    repo: String,
    claimed: Mutex<bool>,
    pub calls: Mutex<Vec<String>>,
}

impl AgentSessions {
    pub fn new(vault: PathBuf, repo: &str) -> Self {
        AgentSessions {
            vault,
            repo: repo.to_string(),
            claimed: Mutex::new(false),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn summary(&self) -> AgentJobSummary {
        AgentJobSummary {
            id: job_id(),
            name: LABEL.to_string(),
            attempt: 1,
            writes: true,
            workspace: self.vault.clone(),
            created_at: CREATED_AT.to_string(),
            agent: AgentJobListing {
                run_id: RUN_ID.to_string(),
                stage: LABEL.to_string(),
                label: LABEL.to_string(),
                job_kind: AgentJobKind::Initial,
                resumed: false,
                provider: "claude".to_string(),
                tier: "sonnet".to_string(),
                model: None,
                effort: Some("high".to_string()),
                cwd: self.vault.clone(),
            },
        }
    }

    fn record(&self, call: &str) {
        self.calls.lock().unwrap().push(call.to_string());
    }
}

impl SessionPort for AgentSessions {
    fn project_for(&self, _ticket: &Value) -> Option<ProjectRef> {
        Some(ProjectRef {
            id: "p1".to_string(),
            path: self.vault.clone(),
            repo: self.repo.clone(),
        })
    }

    fn start(&self, _launch: &PipelineLaunch) -> anyhow::Result<SessionStart> {
        Ok(SessionStart::Busy)
    }

    fn state(&self, session_id: &str) -> SessionState {
        if session_id == RUN_ID {
            SessionState::Running
        } else {
            SessionState::Gone
        }
    }

    fn cancel(&self, _session_id: &str) {}

    fn agent_jobs(&self, run_id: &str) -> Vec<AgentJobSummary> {
        if run_id != RUN_ID || *self.claimed.lock().unwrap() {
            return Vec::new();
        }
        vec![self.summary()]
    }

    fn claim_agent_job(
        &self,
        _run_id: &str,
        job_id: &str,
        _request: &AgentClaimRequest,
    ) -> anyhow::Result<AgentClaim> {
        let mut claimed = self.claimed.lock().unwrap();
        if *claimed {
            return Err(SessionError::Conflict("already claimed".to_string()).into());
        }
        *claimed = true;
        self.record(&format!("claim {job_id}"));
        let mut job = sample_job(LABEL, &self.vault);
        job.id = job_id.to_string();
        Ok(AgentClaim {
            claim_token: CLAIM_TOKEN.to_string(),
            lease_seconds: 120,
            lease_expires_at: CREATED_AT.to_string(),
            resumed: false,
            job,
        })
    }

    fn heartbeat_agent_job(
        &self,
        _run_id: &str,
        job_id: &str,
        request: &AgentHeartbeatRequest,
    ) -> anyhow::Result<AgentHeartbeat> {
        self.record(&format!("heartbeat {job_id} {}", request.claim_token));
        Ok(AgentHeartbeat {
            lease_expires_at: CREATED_AT.to_string(),
            cancel_requested: false,
        })
    }

    fn finish_agent_job(
        &self,
        _run_id: &str,
        job_id: &str,
        result: &AgentJobResult,
    ) -> anyhow::Result<AgentJobAck> {
        self.record(&format!("result {job_id} {}", result.outcome));
        *self.claimed.lock().unwrap() = false;
        Ok(AgentJobAck {
            job_id: job_id.to_string(),
            accepted: true,
            duplicate: false,
            next: "settled".to_string(),
            repair_job_id: None,
            reason: None,
        })
    }

    fn has_claimed_agent_job(&self, run_id: &str) -> bool {
        run_id == RUN_ID && *self.claimed.lock().unwrap()
    }
}
