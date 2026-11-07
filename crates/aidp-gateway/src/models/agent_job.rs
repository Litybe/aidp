use crate::models::agent_job_agent::AgentJobAgent;
use crate::models::agent_job_io::AgentJobIo;
use crate::models::agent_job_permissions::AgentJobPermissions;
use crate::models::agent_job_prompt::AgentJobPrompt;
use crate::models::agent_job_reply::AgentJobReply;
use crate::models::agent_job_skill::AgentJobSkill;
use crate::models::agent_job_tools::AgentJobTools;
use crate::models::agent_job_workspace::AgentJobWorkspace;
use crate::models::enums::agent_job_kind::AgentJobKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One agent session a host runs for a native run: the prompt and the parameters of the session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJob {
    pub id: String,
    pub run_id: String,
    pub stage: String,
    pub label: String,
    pub attempt: u32,
    pub job_kind: AgentJobKind,
    pub agent: AgentJobAgent,
    pub prompt: AgentJobPrompt,
    pub skills: Vec<AgentJobSkill>,
    pub tools: AgentJobTools,
    pub permissions: AgentJobPermissions,
    pub workspace: AgentJobWorkspace,
    pub io: AgentJobIo,
    pub reply: AgentJobReply,
    pub env: BTreeMap<String, String>,
    pub timeout_minutes: u32,
    pub resume_session_id: Option<String>,
}
