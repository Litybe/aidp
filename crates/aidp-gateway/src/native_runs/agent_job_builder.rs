//! Builds the agent job of a session job: the prompt and every parameter the host needs to run it.

use super::dispatch::{session_env, session_workspace, stage_add_dirs};
use super::reply::REPLY_FILE;
use crate::models::agent_job::AgentJob;
use crate::models::agent_job_agent::AgentJobAgent;
use crate::models::agent_job_io::AgentJobIo;
use crate::models::agent_job_permissions::AgentJobPermissions;
use crate::models::agent_job_prompt::AgentJobPrompt;
use crate::models::agent_job_reply::AgentJobReply;
use crate::models::agent_job_skill::AgentJobSkill;
use crate::models::agent_job_tools::AgentJobTools;
use crate::models::agent_job_workspace::AgentJobWorkspace;
use crate::models::constants::agent_jobs::{
    ENV_ALLOWLIST, ENV_HEADLESS, ENV_HEADLESS_ENABLED, INPUT_VALUE_CAP, JOB_TIMEOUT_MINUTES,
    REPLY_LOCATION_FILE, REPLY_SCHEMA_NAME,
};
use crate::models::constants::launch_policy::HEADLESS_DENIAL_GUIDANCE;
use crate::models::enums::agent_job_kind::AgentJobKind;
use crate::models::enums::permission_intent::PermissionIntent;
use crate::models::native_run_view::NativeRunView;
use crate::models::session_job::SessionJob;
use aidp_core::models::enums::{AgentRole, ToolEnforcement};
use aidp_core::pipeline::Pipeline;
use aidp_core::run::RunState;
use aidp_core::skills;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The first attempt of an agent job; the board numbers it and gives it an id.
const FIRST_ATTEMPT: u32 = 1;

/// Lowercase hexadecimal SHA-256 of `text`.
pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The prompt block for `text`, with its fingerprint and length.
pub fn prompt_of(text: &str) -> AgentJobPrompt {
    AgentJobPrompt {
        text: text.to_string(),
        sha256: sha256_hex(text),
        chars: text.chars().count(),
    }
}

/// True for the roles that lose the editing tools under `tool_enforcement: strict`.
fn read_only_role(role: AgentRole) -> bool {
    matches!(
        role,
        AgentRole::Analyzer | AgentRole::Planner | AgentRole::Reviewer
    )
}

fn intent_of(read_only: bool, role: AgentRole) -> PermissionIntent {
    match (read_only, role) {
        (true, AgentRole::Planner) => PermissionIntent::Plan,
        (true, _) => PermissionIntent::ReadOnly,
        (false, _) => PermissionIntent::AutonomousEdit,
    }
}

fn skills_of(names: &[String], search_paths: &[PathBuf]) -> Vec<AgentJobSkill> {
    let roots: Vec<&Path> = search_paths.iter().map(PathBuf::as_path).collect();
    names
        .iter()
        .filter_map(|name| {
            skills::resolve(name, &roots).map(|path| AgentJobSkill {
                name: name.clone(),
                path,
                required: true,
            })
        })
        .collect()
}

fn inputs_of(view: &NativeRunView) -> BTreeMap<String, String> {
    view.request
        .inputs
        .iter()
        .filter_map(|(name, value)| {
            value
                .as_str()
                .map(|text| (name.clone(), text.chars().take(INPUT_VALUE_CAP).collect()))
        })
        .collect()
}

fn upstream_of(needs: &[String], state: &RunState) -> BTreeMap<String, PathBuf> {
    let mut upstream = BTreeMap::new();
    for need in needs {
        let Some(stage) = state.stages.iter().find(|stage| &stage.name == need) else {
            continue;
        };
        for (output, path) in &stage.outputs {
            upstream.insert(format!("{need}.{output}"), path.clone());
        }
    }
    upstream
}

fn env_of(view: &NativeRunView, stage: &str) -> BTreeMap<String, String> {
    let mut env = session_env(&view.run_id, stage, view.autonomous);
    if view.request.headless {
        env.insert(ENV_HEADLESS.to_string(), ENV_HEADLESS_ENABLED.to_string());
    }
    env.retain(|name, _| ENV_ALLOWLIST.contains(&name.as_str()));
    env
}

/// The job for `job`, with `prompt` as its text. The id stays empty until the board publishes it.
pub fn build_agent_job(
    job: &SessionJob,
    prompt: &str,
    view: &NativeRunView,
    pipe: &Pipeline,
    state: &RunState,
    provider: &str,
    skill_search_paths: &[PathBuf],
) -> AgentJob {
    let stage = &job.stage;
    let request = &view.request;
    let read_only =
        pipe.settings.tool_enforcement == ToolEnforcement::Strict && read_only_role(stage.agent);
    let headless = request.headless;
    AgentJob {
        id: String::new(),
        run_id: view.run_id.clone(),
        stage: stage.name.clone(),
        label: job.label.clone(),
        attempt: FIRST_ATTEMPT,
        job_kind: AgentJobKind::Initial,
        agent: AgentJobAgent {
            role: stage.agent.as_str().to_string(),
            agent_type: stage.agent_type.clone(),
            provider: provider.to_string(),
            tier: stage.model.as_str().to_string(),
            model: request.model.clone(),
            effort: request
                .effort
                .clone()
                .or_else(|| stage.effort.map(|level| level.as_str().to_string())),
            ultracode: false,
        },
        prompt: prompt_of(prompt),
        skills: skills_of(&stage.skills, skill_search_paths),
        tools: AgentJobTools {
            strict_mcp: request.strict_mcp_config.unwrap_or(false),
            mcp_config_id: request.mcp_config_id.clone(),
            denied_mcp_tools: request.denied_mcp_tools.clone(),
            denied_shell: request.denied_shell_commands.clone(),
            read_only,
            no_shell: false,
        },
        permissions: AgentJobPermissions {
            intent: intent_of(read_only, stage.agent),
            autonomous: view.autonomous,
            headless,
            guidance: headless.then(|| HEADLESS_DENIAL_GUIDANCE.to_string()),
        },
        workspace: AgentJobWorkspace {
            cwd: session_workspace(job.workspace.as_deref(), &view.workspace).to_path_buf(),
            root: view.workspace.clone(),
            add_dirs: stage_add_dirs(&view.run_dir, state.vault_root.as_deref(), headless),
            shared_writer_group: view.run_id.clone(),
            isolated: job.workspace.is_some(),
        },
        io: AgentJobIo {
            stage_dir: job.reply_dir.clone(),
            reply_file: job.reply_dir.join(REPLY_FILE),
            declared_outputs: job.declared_outputs.clone(),
            inputs: inputs_of(view),
            upstream: upstream_of(&stage.needs, state),
        },
        reply: AgentJobReply {
            schema: REPLY_SCHEMA_NAME.to_string(),
            location: REPLY_LOCATION_FILE.to_string(),
            reviewer_signals: stage.agent == AgentRole::Reviewer,
        },
        env: env_of(view, &stage.name),
        timeout_minutes: JOB_TIMEOUT_MINUTES,
        resume_session_id: None,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/agent_job_fixture.rs"]
pub(crate) mod fixture;

#[cfg(test)]
#[path = "../../tests/unit/agent_job_tests.rs"]
mod tests;
