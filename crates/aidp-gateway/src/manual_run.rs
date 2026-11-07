//! Manual launches: the extras of `POST /api/tickets/:key/start` (flow override, run identifiers)
//! and the ticketless `POST /api/runs/start`.

use crate::models::enums::ApiStatus;
use crate::models::native_error_body::NativeErrorBody;
use crate::models::native_run_start::StartNativeRun;
use crate::native_runs::NativeRunManager;
use crate::projects::Project;
use crate::session_error::SessionError;
use crate::sessions::Origin;
use aidp_core::pipeline;
use aidp_tickets::models::track::Track;
use aidp_tickets::store::TicketStore;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tiny_http::Method;

const STATUS_ACCEPTED: u16 = 202;
const RUNS_SEGMENT: &str = "runs";
const START_SEGMENT: &str = "start";
const BODY_PROJECT: &str = "project";
const TEMPLATE_FIELD: &str = "template";
const INPUTS_FIELD: &str = "inputs";
const TRACK_FIELD: &str = "track";
const TRACK_INPUT: &str = "track";
const FLOW_KEY_FIELD: &str = "flowKey";
const RUN_ID_FIELD: &str = "runId";
const LAUNCHED_FIELD: &str = "launched";
const API_ORIGIN: &str = "api";
const TEMPLATES_DIR: &str = "templates";
const LOCAL_TEMPLATES_DIR: &str = ".aidp/pipeline/flow";
const FRAGMENT_KIND: &str = "fragment";
const TEMPLATE_REQUIRED: &str = "template is required";
const INPUTS_NOT_OBJECT: &str = "inputs must be an object";

fn failure(status: ApiStatus, message: impl ToString) -> (u16, Value) {
    let body = NativeErrorBody {
        error: message.to_string(),
        code: status.code(),
    };
    (
        status.status(),
        serde_json::to_value(body).unwrap_or(Value::Null),
    )
}

fn bad_request(message: impl ToString) -> (u16, Value) {
    failure(ApiStatus::BadRequest, message)
}

/// Refuses a `flow` override that is not a flow of the catalogue.
pub(crate) fn check_flow(skill_root: Option<PathBuf>, flow: &str) -> anyhow::Result<()> {
    let root = skill_root.ok_or_else(|| anyhow::anyhow!("unknown flow `{flow}`: no catalogue"))?;
    match crate::catalog::PipelineCatalog::new(root).flow(flow) {
        Ok(_) => Ok(()),
        Err(error) if error.is::<SessionError>() => {
            anyhow::bail!("unknown flow `{flow}`")
        }
        Err(error) => Err(error),
    }
}

/// Adds `flowKey` (the flow run driving the ticket) and `runId` (its native run) to the report
/// of a manual start that launched `key`; adds nothing when neither exists.
pub(crate) fn attach_launch(store: &TicketStore, key: &str, report: &mut Value) {
    let launched = report[LAUNCHED_FIELD]
        .as_array()
        .is_some_and(|keys| keys.iter().any(|value| value.as_str() == Some(key)));
    if !launched {
        return;
    }
    let Ok((ticket, _, _)) = store.get(key) else {
        return;
    };
    let Some(object) = report.as_object_mut() else {
        return;
    };
    if ticket.autopilot.flow.is_some() {
        object.insert(FLOW_KEY_FIELD.to_string(), json!(key));
    }
    if let Some(run_id) = ticket.autopilot.run_id {
        object.insert(RUN_ID_FIELD.to_string(), json!(run_id));
    }
}

/// The path of template `id`, looked up like the native executor does.
fn find_template(manager: &NativeRunManager, project: &Project, id: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(root) = manager.skill_root() {
        roots.push(root.join(TEMPLATES_DIR));
    }
    roots.push(Path::new(&project.path).join(LOCAL_TEMPLATES_DIR));
    let refs: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
    pipeline::discover(&refs)
        .ok()?
        .into_iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, path)| path)
}

/// Reads the `inputs` object of a body.
fn body_inputs(body: &Value) -> Result<BTreeMap<String, Value>, (u16, Value)> {
    match body.get(INPUTS_FIELD) {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => Ok(map.clone().into_iter().collect()),
        Some(_) => Err(bad_request(INPUTS_NOT_OBJECT)),
    }
}

/// Starts a native run of a pipeline in `project`, with no ticket behind it.
pub fn start_run(manager: &Arc<NativeRunManager>, project: &Project, body: &Value) -> (u16, Value) {
    let Some(template) = body[TEMPLATE_FIELD]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return bad_request(TEMPLATE_REQUIRED);
    };
    let mut inputs = match body_inputs(body) {
        Ok(inputs) => inputs,
        Err(refusal) => return refusal,
    };
    if let Some(track) = body[TRACK_FIELD]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match track.parse::<Track>() {
            Ok(track) => {
                inputs
                    .entry(TRACK_INPUT.to_string())
                    .or_insert_with(|| json!(track.as_str()));
            }
            Err(error) => return bad_request(error),
        }
    }
    let Some(path) = find_template(manager, project, template) else {
        return bad_request(format!("unknown template `{template}`"));
    };
    let pipe = match pipeline::load(&path) {
        Ok(pipe) => pipe,
        Err(error) => return bad_request(error),
    };
    if pipe.kind == FRAGMENT_KIND {
        return bad_request(format!(
            "template `{template}` is a fragment, not a pipeline"
        ));
    }
    if let Err(error) = pipeline::validate_inputs(&pipe, &inputs) {
        return bad_request(error);
    }
    let request = StartNativeRun {
        project_id: project.id.clone(),
        template_id: template.to_string(),
        inputs,
        origin: Some(Origin::new(API_ORIGIN)),
        ..StartNativeRun::default()
    };
    match manager.start(request) {
        Ok(run_id) => (STATUS_ACCEPTED, json!({ RUN_ID_FIELD: run_id })),
        Err(error) => {
            let api = crate::models::ApiError::from(&error);
            failure(api.status, api.message)
        }
    }
}

/// Answers `POST /api/runs/start`; `None` for any other route.
pub(crate) fn handle(
    app: &crate::http::App,
    method: &Method,
    path: &str,
    body: &Value,
) -> Option<(u16, Value)> {
    let segments: Vec<&str> = path
        .trim_start_matches("/api")
        .trim_matches('/')
        .split('/')
        .collect();
    if *method != Method::Post || segments != [RUNS_SEGMENT, START_SEGMENT] {
        return None;
    }
    let requested = body
        .get(BODY_PROJECT)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let project = match crate::unity_routes::resolve_project(app, requested) {
        Ok(project) => project,
        Err(reply) => return Some(reply),
    };
    Some(start_run(&app.native_runs, &project, body))
}

#[cfg(test)]
#[path = "../tests/unit/manual_run_tests.rs"]
mod tests;
