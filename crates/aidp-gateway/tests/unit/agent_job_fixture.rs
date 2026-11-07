use super::{prompt_of, REPLY_FILE};
use crate::models::agent_job::AgentJob;
use crate::models::agent_job_agent::AgentJobAgent;
use crate::models::agent_job_io::AgentJobIo;
use crate::models::agent_job_permissions::AgentJobPermissions;
use crate::models::agent_job_reply::AgentJobReply;
use crate::models::agent_job_tools::AgentJobTools;
use crate::models::agent_job_workspace::AgentJobWorkspace;
use crate::models::constants::agent_jobs::{
    JOB_TIMEOUT_MINUTES, REPLY_LOCATION_FILE, REPLY_SCHEMA_NAME,
};
use crate::models::enums::agent_job_kind::AgentJobKind;
use crate::models::enums::permission_intent::PermissionIntent;
use std::collections::BTreeMap;
use std::path::Path;

pub const RUN_ID: &str = "20261018-101500-a1b2";
pub const STAGE: &str = "implement";
pub const PROMPT: &str = "do the work";
pub const VALID_REPLY: &str =
    r#"{"status":"completed","summary":"done","outputs":{},"signals":{}}"#;

/// An executor job for `label` whose reply directory is `stage_dir`.
pub fn sample_job(label: &str, stage_dir: &Path) -> AgentJob {
    AgentJob {
        id: String::new(),
        run_id: RUN_ID.to_string(),
        stage: STAGE.to_string(),
        label: label.to_string(),
        attempt: 1,
        job_kind: AgentJobKind::Initial,
        agent: AgentJobAgent {
            role: "executor".to_string(),
            agent_type: "executor".to_string(),
            provider: "claude".to_string(),
            tier: "sonnet".to_string(),
            model: None,
            effort: Some("high".to_string()),
            ultracode: false,
        },
        prompt: prompt_of(PROMPT),
        skills: Vec::new(),
        tools: AgentJobTools {
            strict_mcp: false,
            mcp_config_id: None,
            denied_mcp_tools: BTreeMap::new(),
            denied_shell: Vec::new(),
            read_only: false,
            no_shell: false,
        },
        permissions: AgentJobPermissions {
            intent: PermissionIntent::AutonomousEdit,
            autonomous: false,
            headless: false,
            guidance: None,
        },
        workspace: AgentJobWorkspace {
            cwd: stage_dir.to_path_buf(),
            root: stage_dir.to_path_buf(),
            add_dirs: Vec::new(),
            shared_writer_group: RUN_ID.to_string(),
            isolated: false,
        },
        io: AgentJobIo {
            stage_dir: stage_dir.to_path_buf(),
            reply_file: stage_dir.join(REPLY_FILE),
            declared_outputs: Vec::new(),
            inputs: BTreeMap::new(),
            upstream: BTreeMap::new(),
        },
        reply: AgentJobReply {
            schema: REPLY_SCHEMA_NAME.to_string(),
            location: REPLY_LOCATION_FILE.to_string(),
            reviewer_signals: false,
        },
        env: BTreeMap::new(),
        timeout_minutes: JOB_TIMEOUT_MINUTES,
        resume_session_id: None,
    }
}
