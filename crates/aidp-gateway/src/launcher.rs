//! The single seam deciding whether a launch is a native run or a legacy session.

use crate::models::enums::engine::Engine;
use crate::models::launched_run::LaunchedRun;
use crate::models::native_run_start::StartNativeRun;
use crate::models::native_run_view::NativeRunView;
use crate::native_runs::NativeRunManager;
use crate::sessions::{SessionManager, StartRequest};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const AUTO_APPROVE_ALL: &str = "*";

/// Routes a launch to a native run or to a legacy workflow or prompt session.
pub struct RunLauncher {
    sessions: Arc<SessionManager>,
    native: Arc<NativeRunManager>,
}

/// True when the request must stay a session: a workflow or prompt engine, or no template.
fn stays_session(request: &StartRequest) -> bool {
    let engine = request.engine.as_deref().and_then(Engine::parse);
    matches!(engine, Some(Engine::Workflow | Engine::Prompt)) || request.template_id.is_none()
}

/// The inputs file content, overridden by the inline inputs.
fn merged_inputs(request: &StartRequest) -> anyhow::Result<BTreeMap<String, Value>> {
    let mut inputs: BTreeMap<String, Value> = match &request.inputs_file {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => BTreeMap::new(),
    };
    for (name, value) in request.inputs.iter().flatten() {
        inputs.insert(name.clone(), Value::String(value.clone()));
    }
    Ok(inputs)
}

/// The native run request equivalent to a start request that has a template.
fn native_request(
    request: StartRequest,
    template_id: String,
    autonomous: bool,
) -> anyhow::Result<StartNativeRun> {
    let ticket = request
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.ticket.clone())
        .or_else(|| request.worktree.clone());
    let workspace = request
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.path.as_ref())
        .map(std::path::PathBuf::from);
    Ok(StartNativeRun {
        client_request_id: None,
        inputs: merged_inputs(&request)?,
        strict_mcp_config: request.strict_mcp_config,
        mcp_config_id: request.mcp_config_id.clone(),
        mcp_servers: request.mcp_servers.clone(),
        denied_shell_commands: request.denied_shell_commands.clone(),
        denied_mcp_tools: request.denied_mcp_tools.clone(),
        project_id: request.project_id,
        template_id,
        target: request.target,
        workspace,
        provider: request.provider,
        autonomous,
        headless: request.headless,
        auto_approve_gates: if autonomous {
            vec![AUTO_APPROVE_ALL.to_string()]
        } else {
            request.auto_approve_gates.unwrap_or_default()
        },
        origin: request.origin,
        model: request.model,
        permission_mode: request.permission_mode,
        effort: request.effort,
        ticket,
        agent_executor: request.agent_executor,
        ..StartNativeRun::default()
    })
}

impl RunLauncher {
    pub fn new(sessions: Arc<SessionManager>, native: Arc<NativeRunManager>) -> Self {
        RunLauncher { sessions, native }
    }

    /// Starts a native run, or delegates unchanged to the session manager.
    pub fn launch(&self, request: StartRequest, autonomous: bool) -> anyhow::Result<LaunchedRun> {
        if stays_session(&request) {
            return Ok(LaunchedRun::Session(self.sessions.start(request)?));
        }
        let template_id = request.template_id.clone().unwrap_or_default();
        let workspace_request = request.clone();
        let mut native = native_request(request, template_id, autonomous)?;
        if native.workspace.is_none() {
            native.workspace = self.sessions.native_workspace(&workspace_request)?;
        }
        let run_id = self.native.start(native)?;
        Ok(LaunchedRun::Native { run_id })
    }

    /// Waits until the native run stops executing.
    pub fn wait_for(&self, run_id: &str, timeout: Duration) -> Option<NativeRunView> {
        self.native.wait_for(run_id, timeout)
    }

    pub fn native(&self) -> &Arc<NativeRunManager> {
        &self.native
    }
}

#[cfg(test)]
#[path = "../tests/unit/launcher_tests.rs"]
mod tests;
