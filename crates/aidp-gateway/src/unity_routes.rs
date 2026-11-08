//! `GET /api/unity/status` and `POST /api/unity/lock/release`.

use crate::http::App;
use crate::models::enums::ApiStatus;
use crate::projects::Project;
use crate::unity_lock::resource_key;
use crate::unity_status::{status, CachedProcesses, SystemProcesses, PROCESS_CACHE_TTL};
use aidp_flows::models::constants::resource::UNITY_EDITOR;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use tiny_http::Method;

const UNITY_SEGMENT: &str = "unity";
const QUERY_PROJECT: &str = "project";
const BODY_PROJECT: &str = "project";
const STATUS_OK: u16 = 200;
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_UNAVAILABLE: u16 = 503;

static PROCESSES: OnceLock<CachedProcesses> = OnceLock::new();

fn processes() -> &'static CachedProcesses {
    PROCESSES.get_or_init(|| CachedProcesses::new(Box::new(SystemProcesses), PROCESS_CACHE_TTL))
}

fn failure(code: u16, message: impl ToString) -> (u16, Value) {
    let status = ApiStatus::from_status(code);
    (
        code,
        json!({ "error": message.to_string(), "code": status.code() }),
    )
}

/// The project asked for (id or name), else the only registered one; several registered and none asked is a 400.
/// Shared with `memory_routes`.
pub(crate) fn resolve_project(app: &App, requested: Option<&str>) -> Result<Project, (u16, Value)> {
    if let Some(name) = requested {
        return app
            .projects
            .get(name)
            .ok_or_else(|| failure(STATUS_NOT_FOUND, format!("unknown project `{name}`")));
    }
    let mut all = app.projects.list();
    match all.len() {
        1 => Ok(all.remove(0)),
        0 => Err(failure(STATUS_NOT_FOUND, "no project registered")),
        _ => Err(failure(
            STATUS_BAD_REQUEST,
            "several projects registered: pass `?project=<id or name>`",
        )),
    }
}

fn get_status(app: &App, requested: Option<&str>) -> (u16, Value) {
    let project = match resolve_project(app, requested) {
        Ok(project) => project,
        Err(reply) => return reply,
    };
    let (holder, waiting) = app.flows.as_ref().map_or((None, Vec::new()), |engine| {
        let key = resource_key(UNITY_EDITOR, &project.id);
        engine.executor().resources().snapshot(&key)
    });
    let body = status(
        &project.id,
        project.checkout_path(),
        &processes().list(),
        holder.as_ref(),
        &waiting,
    );
    (STATUS_OK, body)
}

fn release_lock(app: &App, requested: Option<&str>) -> (u16, Value) {
    let Some(engine) = app.flows.as_ref() else {
        return failure(STATUS_UNAVAILABLE, "flow engine unavailable");
    };
    let project = match resolve_project(app, requested) {
        Ok(project) => project,
        Err(reply) => return reply,
    };
    let key = resource_key(UNITY_EDITOR, &project.id);
    let released = engine.executor().release_resource(&key);
    match &released {
        Some(holder) => tracing::warn!(
            resource = %key,
            flow = %holder.flow_key,
            ticket = %holder.ticket_key,
            "unity lock released by hand"
        ),
        None => tracing::info!(resource = %key, "unity lock release requested, nobody held it"),
    }
    (
        STATUS_OK,
        json!({
            "ok": true,
            "released": released.is_some(),
            "resource": key,
            "holder": released,
        }),
    )
}

/// Answers the `/api/unity` routes; `None` for any other path.
pub fn handle(
    app: &App,
    method: &Method,
    path: &str,
    query: &HashMap<String, String>,
    body: &Value,
) -> Option<(u16, Value)> {
    let segments: Vec<&str> = path
        .trim_start_matches("/api")
        .trim_matches('/')
        .split('/')
        .collect();
    let requested = query
        .get(QUERY_PROJECT)
        .map(String::as_str)
        .or_else(|| body.get(BODY_PROJECT).and_then(Value::as_str))
        .filter(|name| !name.is_empty());
    match (method, segments.as_slice()) {
        (Method::Get, [UNITY_SEGMENT, "status"]) => Some(get_status(app, requested)),
        (Method::Post, [UNITY_SEGMENT, "lock", "release"]) => Some(release_lock(app, requested)),
        _ => None,
    }
}
