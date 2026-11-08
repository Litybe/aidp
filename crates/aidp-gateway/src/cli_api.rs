//! Typed entry points behind `aidp gateway`: each verb returns a [`Reply`]
//! instead of printing, so the CLI decides how to render it.

use crate::client::{
    delete, dispatcher_tick, get, post, put, render_view, resolve_project, start_body,
    with_template_engine, StartBodyArgs,
};
use crate::models::enums::engine::Engine;
use crate::models::WorkspaceRequest;
use crate::routes::{
    AUTOMATIONS, AUTOMATION_RUNS, DISPATCHER, DISPATCHER_PAUSE, DISPATCHER_RESUME, EVENTS_RECENT,
    GATEWAY_STATUS, PRESETS, PROJECTS, PROJECTS_IMPORT, QUEUE, SESSIONS,
};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Which human-readable rendering applies to a reply's JSON value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextView {
    Status,
    Sessions,
    Automations,
    Presets,
    Queue,
    Dispatcher,
    StartedSession,
    Raw,
}

/// A daemon answer plus the view its `text` format uses.
#[derive(Debug)]
pub struct Reply {
    pub value: Value,
    pub view: TextView,
}

impl Reply {
    fn new(value: Value, view: TextView) -> Self {
        Self { value, view }
    }

    fn raw(value: Value) -> Self {
        Self::new(value, TextView::Raw)
    }

    /// The `--format text` rendering of this reply.
    pub fn text(&self) -> String {
        render_view(self.view, &self.value)
    }
}

/// Arguments of `aidp gateway start`.
#[derive(Debug, Default)]
pub struct StartParams {
    pub project: String,
    pub template: Option<String>,
    pub engine: Option<Engine>,
    pub target: Option<String>,
    pub inputs: Vec<String>,
    pub prompt: Option<String>,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub max_budget_usd: Option<f64>,
    pub context_from: Option<String>,
    pub keep_open: bool,
    pub workspace: Option<WorkspaceRequest>,
}

/// Run the daemon, serving the web UI build of `ui_dir` on `/`; blocks until it stops.
pub fn serve(
    bind: std::net::IpAddr,
    port: u16,
    state_dir: Option<PathBuf>,
    ui_dir: Option<PathBuf>,
) -> Result<()> {
    crate::http::serve(bind, port, state_dir, ui_dir)
}

pub fn status(url: &Option<String>) -> Result<Reply> {
    Ok(Reply::new(get(url, GATEWAY_STATUS)?, TextView::Status))
}

pub fn sessions(url: &Option<String>, status: Option<&str>, limit: usize) -> Result<Reply> {
    let mut path = format!("{SESSIONS}?limit={limit}");
    if let Some(status) = status {
        path.push_str(&format!("&status={status}"));
    }
    Ok(Reply::new(get(url, &path)?, TextView::Sessions))
}

/// Parses repeated `key=value` pipeline inputs, trimming both sides.
fn parse_inputs(inputs: Vec<String>) -> Result<serde_json::Map<String, Value>> {
    let mut map = serde_json::Map::new();
    for pair in inputs {
        match pair.split_once('=') {
            Some((key, value)) => {
                map.insert(key.trim().to_string(), json!(value.trim()));
            }
            None => bail!("--input expects key=value, got {pair:?}"),
        }
    }
    Ok(map)
}

pub fn start(url: &Option<String>, params: StartParams) -> Result<Reply> {
    if params.template.is_none() && params.prompt.is_none() {
        bail!("give --template <id> (pipeline) or --prompt <text>");
    }
    let pid = resolve_project(url, &params.project)?;
    let has_template = params.template.is_some();
    let body = start_body(StartBodyArgs {
        pid,
        template: params.template,
        target: params.target,
        inputs_map: parse_inputs(params.inputs)?,
        prompt: params.prompt,
        permission_mode: params.permission_mode,
        model: params.model,
        effort: params.effort,
        max_budget_usd: params.max_budget_usd,
        context_from: params.context_from,
        keep_open: params.keep_open,
        workspace: params.workspace,
    });
    let body = if has_template {
        with_template_engine(
            body,
            params.engine.map(|engine| engine.as_str().to_string()),
        )
    } else {
        body
    };
    Ok(Reply::new(
        post(url, SESSIONS, body)?,
        TextView::StartedSession,
    ))
}

pub fn reply(url: &Option<String>, id: &str, text: &str) -> Result<Reply> {
    let path = format!("{SESSIONS}/{id}/reply");
    Ok(Reply::raw(post(url, &path, json!({ "text": text }))?))
}

pub fn cancel(url: &Option<String>, id: &str) -> Result<Reply> {
    let path = format!("{SESSIONS}/{id}/cancel");
    Ok(Reply::raw(post(url, &path, json!({}))?))
}

fn read_json_file(file: &Path) -> Result<Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(file)?)?)
}

pub fn automations_list(url: &Option<String>) -> Result<Reply> {
    Ok(Reply::new(get(url, AUTOMATIONS)?, TextView::Automations))
}

/// Adds an automation, or updates it when the JSON carries a non-empty `id`.
pub fn automations_add(url: &Option<String>, file: &Path) -> Result<Reply> {
    let body = read_json_file(file)?;
    let reply = match body["id"].as_str().filter(|id| !id.is_empty()) {
        Some(id) => put(url, &format!("{AUTOMATIONS}/{id}"), body.clone())
            .or_else(|_| post(url, AUTOMATIONS, body.clone()))?,
        None => post(url, AUTOMATIONS, body)?,
    };
    Ok(Reply::raw(reply))
}

pub fn automations_rm(url: &Option<String>, id: &str) -> Result<Reply> {
    Ok(Reply::raw(delete(url, &format!("{AUTOMATIONS}/{id}"))?))
}

pub fn automations_run(url: &Option<String>, id: &str) -> Result<Reply> {
    let path = format!("{AUTOMATIONS}/{id}/run");
    Ok(Reply::raw(post(url, &path, json!({}))?))
}

pub fn automations_set_enabled(url: &Option<String>, id: &str, enabled: bool) -> Result<Reply> {
    let path = format!("{AUTOMATIONS}/{id}");
    Ok(Reply::raw(put(url, &path, json!({ "enabled": enabled }))?))
}

pub fn automations_runs(url: &Option<String>, id: Option<&str>, limit: usize) -> Result<Reply> {
    let path = match id {
        Some(id) => format!("{AUTOMATIONS}/{id}/runs"),
        None => format!("{AUTOMATION_RUNS}?limit={limit}"),
    };
    Ok(Reply::raw(get(url, &path)?))
}

pub fn projects_list(url: &Option<String>) -> Result<Reply> {
    Ok(Reply::raw(get(url, PROJECTS)?))
}

pub fn projects_add(url: &Option<String>, path: &str) -> Result<Reply> {
    Ok(Reply::raw(post(url, PROJECTS, json!({ "path": path }))?))
}

/// Sets the test checkout of a project (`None` clears it).
pub fn projects_set_test_checkout(
    url: &Option<String>,
    id: &str,
    path: Option<&str>,
) -> Result<Reply> {
    let body = json!({ "testCheckout": path });
    Ok(Reply::raw(put(url, &format!("{PROJECTS}/{id}"), body)?))
}

pub fn projects_rm(url: &Option<String>, id: &str) -> Result<Reply> {
    Ok(Reply::raw(delete(url, &format!("{PROJECTS}/{id}"))?))
}

/// Imports projects from a file holding either `{"projects": [...]}` or a bare array.
pub fn projects_import(url: &Option<String>, file: &Path) -> Result<Reply> {
    let body = read_json_file(file)?;
    let projects = if body["projects"].is_array() {
        body["projects"].clone()
    } else {
        body
    };
    Ok(Reply::raw(post(
        url,
        PROJECTS_IMPORT,
        json!({ "projects": projects }),
    )?))
}

pub fn presets_list(url: &Option<String>) -> Result<Reply> {
    Ok(Reply::new(get(url, PRESETS)?, TextView::Presets))
}

pub fn presets_add(url: &Option<String>, file: &Path) -> Result<Reply> {
    Ok(Reply::raw(post(url, PRESETS, read_json_file(file)?)?))
}

pub fn presets_rm(url: &Option<String>, id: &str) -> Result<Reply> {
    Ok(Reply::raw(delete(url, &format!("{PRESETS}/{id}"))?))
}

pub fn presets_start(url: &Option<String>, id: &str) -> Result<Reply> {
    let path = format!("{PRESETS}/{id}/start");
    Ok(Reply::new(
        post(url, &path, json!({}))?,
        TextView::StartedSession,
    ))
}

pub fn queue_list(url: &Option<String>, project_id: Option<&str>) -> Result<Reply> {
    let mut path = QUEUE.to_string();
    if let Some(project_id) = project_id {
        path.push_str(&format!("?projectId={project_id}"));
    }
    Ok(Reply::new(get(url, &path)?, TextView::Queue))
}

pub fn queue_promote(url: &Option<String>, id: &str) -> Result<Reply> {
    let path = format!("{QUEUE}/{id}/promote");
    Ok(Reply::raw(post(url, &path, json!({}))?))
}

pub fn queue_cancel(url: &Option<String>, id: &str) -> Result<Reply> {
    let path = format!("{QUEUE}/{id}/cancel");
    Ok(Reply::raw(post(url, &path, json!({}))?))
}

pub fn dispatcher_status(url: &Option<String>) -> Result<Reply> {
    Ok(Reply::new(get(url, DISPATCHER)?, TextView::Dispatcher))
}

pub fn dispatcher_pause(url: &Option<String>) -> Result<Reply> {
    let body = serde_json::to_value(crate::models::DispatcherPauseRequest {
        paused: true,
        actor: None,
    })?;
    Ok(Reply::raw(post(url, DISPATCHER_PAUSE, body)?))
}

pub fn dispatcher_resume(url: &Option<String>) -> Result<Reply> {
    Ok(Reply::raw(post(url, DISPATCHER_RESUME, json!({}))?))
}

pub fn dispatcher_run_tick(url: &Option<String>, dry_run: bool) -> Result<Reply> {
    Ok(Reply::new(
        dispatcher_tick(url, dry_run)?,
        TextView::Dispatcher,
    ))
}

pub fn events(url: &Option<String>, limit: usize) -> Result<Reply> {
    Ok(Reply::raw(get(
        url,
        &format!("{EVENTS_RECENT}?limit={limit}"),
    )?))
}

#[cfg(test)]
#[path = "../tests/unit/cli_api_tests.rs"]
mod tests;
