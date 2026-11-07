//! HTTP handlers of the native run routes. [`handle`] answers every `/api/runs`
//! request and returns `None` for any other path.

use super::NativeRunManager;
use crate::models::enums::ApiStatus;
use crate::models::native_error_body::NativeErrorBody;
use crate::models::native_run_accepted::NativeRunAccepted;
use crate::models::native_run_request::{NativeRunRequest, NativeRunStarted};
use crate::models::native_run_start::StartNativeRun;
use crate::models::native_runs_list::NativeRunsList;
use crate::models::{ApiError, FlowReply};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tiny_http::Method;

const API_PREFIX: &str = "/api";
const RUNS_SEGMENT: &str = "runs";
const STAGES_SEGMENT: &str = "stages";
const CANCEL_VERB: &str = "cancel";
const RETRY_VERB: &str = "retry";
const SKIP_VERB: &str = "skip";
const APPROVE_VERB: &str = "approve";
const STATUS_OK: u16 = 200;
const STATUS_CREATED: u16 = 201;
const MISSING_PROJECT: &str = "projectId is required";
const MISSING_TEMPLATE: &str = "templateId is required";
const INVALID_BODY: &str = "invalid run request";
const RELATIVE_WORKSPACE: &str = "workspace must be an absolute directory";
const NO_RUN: &str = "run not found";

fn reply<T: Serialize>(status: u16, body: &T) -> FlowReply {
    FlowReply {
        status,
        body: serde_json::to_value(body).unwrap_or(Value::Null),
    }
}

/// Maps a failure to its reply through the gateway's single error mapping.
fn error_reply(error: &anyhow::Error) -> FlowReply {
    let api = ApiError::from(error);
    reply(
        api.status.status(),
        &NativeErrorBody {
            error: api.message,
            code: api.status.code(),
        },
    )
}

fn bad_request(message: &str) -> FlowReply {
    reply(
        ApiStatus::BadRequest.status(),
        &NativeErrorBody {
            error: message.to_string(),
            code: ApiStatus::BadRequest.code(),
        },
    )
}

fn parse_request(body: &Value) -> Result<StartNativeRun, FlowReply> {
    let request: NativeRunRequest =
        serde_json::from_value(body.clone()).map_err(|_| bad_request(INVALID_BODY))?;
    if request.project_id.trim().is_empty() {
        return Err(bad_request(MISSING_PROJECT));
    }
    if request.template_id.trim().is_empty() {
        return Err(bad_request(MISSING_TEMPLATE));
    }
    if request
        .client_request_id
        .as_deref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return Err(bad_request(
            crate::models::constants::flow_execution::EMPTY_CLIENT_REQUEST_ID,
        ));
    }
    if request
        .workspace
        .as_ref()
        .is_some_and(|workspace| !workspace.is_absolute())
    {
        return Err(bad_request(RELATIVE_WORKSPACE));
    }
    Ok(request.into())
}

fn start(manager: &Arc<NativeRunManager>, body: &Value) -> FlowReply {
    let request = match parse_request(body) {
        Ok(request) => request,
        Err(refusal) => return refusal,
    };
    match manager.start(request) {
        Ok(run_id) => reply(STATUS_CREATED, &NativeRunStarted { run_id }),
        Err(error) => error_reply(&error),
    }
}

fn read_run(manager: &NativeRunManager, run_id: &str) -> FlowReply {
    match manager.get(run_id) {
        Some(view) => reply(STATUS_OK, &view),
        None => error_reply(&super::not_found(format!("{NO_RUN}: {run_id}"))),
    }
}

fn accepted(result: anyhow::Result<()>) -> FlowReply {
    match result {
        Ok(()) => reply(STATUS_OK, &NativeRunAccepted { ok: true }),
        Err(error) => error_reply(&error),
    }
}

fn stage_verb(
    manager: &Arc<NativeRunManager>,
    run_id: &str,
    stage: &str,
    verb: &str,
) -> Option<FlowReply> {
    let result = match verb {
        RETRY_VERB => manager.retry(run_id, stage),
        SKIP_VERB => manager.skip(run_id, stage),
        APPROVE_VERB => manager.approve(run_id, stage),
        _ => return None,
    };
    Some(accepted(result))
}

/// Answers a request on `/api/runs`, or `None` when the path is not a native run
/// route. A known path with the wrong method is also `None`.
pub fn handle(
    manager: &Arc<NativeRunManager>,
    method: &Method,
    path: &str,
    body: &Value,
) -> Option<FlowReply> {
    let trimmed = path.strip_prefix(API_PREFIX)?.trim_matches('/');
    let decoded = crate::path_segments::split(trimmed);
    let segments: Vec<&str> = decoded.iter().map(String::as_str).collect();
    if segments.first() != Some(&RUNS_SEGMENT) {
        return None;
    }
    match (method, &segments[1..]) {
        (Method::Get, []) => Some(reply(
            STATUS_OK,
            &NativeRunsList {
                entries: manager.list(),
            },
        )),
        (Method::Post, []) => Some(start(manager, body)),
        (Method::Get, [run_id]) => Some(read_run(manager, run_id)),
        (Method::Post, [run_id, CANCEL_VERB]) => Some(accepted(manager.cancel(run_id))),
        (Method::Post, [run_id, STAGES_SEGMENT, stage, verb]) => {
            stage_verb(manager, run_id, stage, verb)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/native_routes_tests.rs"]
mod tests;
