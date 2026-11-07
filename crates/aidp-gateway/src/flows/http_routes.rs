//! HTTP handlers of the ticket flow routes. [`handle`] answers every
//! `/api/flows` request and returns `None` for any other path.

use super::{FlowEngine, FlowEngineError};
use crate::models::constants::flow_external::AGENT_ID_PREFIX;
use crate::models::constants::flow_manual::{BODY_PUBLISH, INVALID_PUBLISH};
use crate::models::enums::ApiStatus;
pub use crate::models::enums::FlowVerb;
use crate::models::ApiError;
pub use crate::models::FlowReply;
use aidp_core::models::enums::HasErrorKind;
use aidp_flows::ManualRequest;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tiny_http::Method;

/// Path prefix of the gateway API.
pub const API_PREFIX: &str = "/api";
/// Collection segment of the flow routes.
pub const FLOWS_SEGMENT: &str = "flows";
/// Segment of the journal route.
pub const JOURNAL_SEGMENT: &str = "journal";
/// Segment of the manual request route.
pub const MANUAL_SEGMENT: &str = "manual";
/// Segment introducing a step.
pub const STEPS_SEGMENT: &str = "steps";
/// Segment of the external jobs route.
pub const EXTERNAL_SEGMENT: &str = "external";
/// Last segment of the external job result route.
pub const RESULT_SEGMENT: &str = "result";
/// Last segment of the agent job claim route.
pub const CLAIM_SEGMENT: &str = "claim";
/// Last segment of the agent job lease renewal route.
pub const HEARTBEAT_SEGMENT: &str = "heartbeat";

const QUERY_STATUS: &str = "status";
const QUERY_SERVICE: &str = "service";
/// Body key of the ticket.
pub const BODY_TICKET: &str = "ticket";
/// Body key of the flow name.
pub const BODY_FLOW: &str = "flow";
/// Body key of the run settings.
pub const BODY_SETTINGS: &str = "settings";
pub const BODY_ACTOR: &str = "actor";
/// Body key of a verdict outcome.
pub const BODY_OK: &str = "ok";
/// Body key of a verdict report.
pub const BODY_REPORT: &str = "report";
pub const BODY_ACTION: &str = "action";
pub const BODY_PIPELINE: &str = "pipeline";
pub const BODY_WITH: &str = "with";
pub const BODY_EXISTING: &str = "existing";
const BODY_ERROR: &str = "error";
const BODY_CODE: &str = "code";
const KEY_FIELD: &str = "key";
const JOBS_FIELD: &str = "jobs";
const JOB_ID_SEPARATOR: &str = "/";
const ENTRIES_FIELD: &str = "entries";
const DEFAULT_ACTOR: &str = "api";

const MISSING_TICKET: &str = "ticket is required";
const MISSING_OK: &str = "ok must be a boolean";
const MISSING_MANUAL: &str = "action or pipeline is required";
const INVALID_WITH: &str = "with must be a JSON object";

const STATUS_OK: u16 = 200;
const STATUS_CREATED: u16 = 201;

impl FlowReply {
    pub(crate) fn ok(body: Value) -> Self {
        FlowReply {
            status: STATUS_OK,
            body,
        }
    }
}

/// Maps a failure to its reply. A start conflict carries the run already
/// holding the ticket under `existing`.
fn error_reply(error: &anyhow::Error) -> FlowReply {
    let engine_error = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<FlowEngineError>());
    let (status, existing) = match engine_error {
        Some(typed) => {
            let existing = match typed {
                FlowEngineError::Conflict { existing, .. } => existing.clone(),
                _ => None,
            };
            (ApiStatus::from(typed.kind()), existing)
        }
        None => (ApiError::from(error).status, None),
    };
    let message = engine_error.map_or_else(|| error.to_string(), ToString::to_string);
    let mut body = json!({ BODY_ERROR: message, BODY_CODE: status.code() });
    if let Some(existing) = existing {
        body[BODY_EXISTING] = existing;
    }
    FlowReply {
        status: status.status(),
        body,
    }
}

fn state_reply(result: anyhow::Result<aidp_flows::FlowState>, engine: &FlowEngine) -> FlowReply {
    match result.and_then(|state| engine.view(&state.key)) {
        Ok(view) => FlowReply::ok(view),
        Err(error) => error_reply(&error),
    }
}

fn actor_of(body: &Value) -> String {
    body.get(BODY_ACTOR)
        .and_then(Value::as_str)
        .filter(|actor| !actor.is_empty())
        .unwrap_or(DEFAULT_ACTOR)
        .to_string()
}

fn bad_request(message: &str) -> FlowReply {
    error_reply(&anyhow::anyhow!(message.to_string()))
}

fn start(engine: &Arc<FlowEngine>, body: &Value) -> FlowReply {
    let Some(_) = body.get(BODY_TICKET).filter(|value| value.is_object()) else {
        return bad_request(MISSING_TICKET);
    };
    let request: crate::models::TicketFlowRequest = match serde_json::from_value(body.clone()) {
        Ok(request) => request,
        Err(error) => return bad_request(&error.to_string()),
    };
    let result = engine
        .start_request(request)
        .and_then(|state| engine.view(&state.key));
    match result {
        Ok(view) => FlowReply {
            status: STATUS_CREATED,
            body: view,
        },
        Err(error) => error_reply(&error),
    }
}

fn list(engine: &FlowEngine, query: &HashMap<String, String>) -> FlowReply {
    let status = query.get(QUERY_STATUS).map(String::as_str);
    let service = query.get(QUERY_SERVICE).map(String::as_str);
    let entries = engine.list(
        status.filter(|value| !value.is_empty()),
        service.filter(|value| !value.is_empty()),
    );
    FlowReply::ok(json!({ ENTRIES_FIELD: entries }))
}

fn manual_request(body: &Value) -> Result<ManualRequest, FlowReply> {
    let publish = match body.get(BODY_PUBLISH) {
        None => false,
        Some(Value::Bool(publish)) => *publish,
        Some(_) => return Err(bad_request(INVALID_PUBLISH)),
    };
    let with = match body.get(BODY_WITH) {
        None | Some(Value::Null) => BTreeMap::new(),
        Some(Value::Object(map)) => map.clone().into_iter().collect(),
        Some(_) => return Err(bad_request(INVALID_WITH)),
    };
    if let Some(name) = body.get(BODY_ACTION).and_then(Value::as_str) {
        return Ok(ManualRequest::Action {
            name: name.to_string(),
            with,
        });
    }
    match body.get(BODY_PIPELINE).and_then(Value::as_str) {
        Some(template) => Ok(ManualRequest::Pipeline {
            template: template.to_string(),
            publish,
            with,
        }),
        None => Err(bad_request(MISSING_MANUAL)),
    }
}

fn run_control(engine: &FlowEngine, key: &str, verb: &str, body: &Value) -> Option<FlowReply> {
    let actor = actor_of(body);
    if verb == MANUAL_SEGMENT {
        return Some(match manual_request(body) {
            Ok(request) => state_reply(engine.manual(key, request, &actor), engine),
            Err(reply) => reply,
        });
    }
    let result = match FlowVerb::parse(verb)? {
        FlowVerb::Pause => engine.pause(key, &actor),
        FlowVerb::Resume => engine.resume(key, &actor),
        FlowVerb::Abort => engine.abort(key, &actor),
        _ => return None,
    };
    Some(state_reply(result, engine))
}

fn step_decision(
    engine: &FlowEngine,
    key: &str,
    step: &str,
    verb: &str,
    body: &Value,
) -> Option<FlowReply> {
    let actor = actor_of(body);
    let result = match FlowVerb::parse(verb)? {
        FlowVerb::Retry => engine.retry(key, step, &actor),
        FlowVerb::Skip => engine.skip(key, step, &actor),
        FlowVerb::Approve => engine.approve(key, step, &actor),
        FlowVerb::Reject => engine.reject(key, step, &actor),
        FlowVerb::Click => engine.click(key, step, &actor),
        FlowVerb::Verdict => {
            let Some(ok) = body.get(BODY_OK).and_then(Value::as_bool) else {
                return Some(bad_request(MISSING_OK));
            };
            let report = body.get(BODY_REPORT).and_then(Value::as_str);
            engine.verdict(key, step, ok, report, &actor)
        }
        _ => return None,
    };
    Some(state_reply(result, engine))
}

fn read_external_jobs(engine: &FlowEngine, key: &str) -> FlowReply {
    match engine.external_jobs(key) {
        Ok(jobs) => FlowReply::ok(json!({ KEY_FIELD: key, JOBS_FIELD: jobs })),
        Err(error) => error_reply(&error),
    }
}

/// Applies the result of an external job. A job id holds `:` and may hold `/`
/// when the caller left it unencoded, so `id_segments` are rejoined.
fn external_result(
    engine: &FlowEngine,
    key: &str,
    id_segments: &[&str],
    body: &Value,
) -> FlowReply {
    let job_id = id_segments.join(JOB_ID_SEPARATOR);
    if job_id.starts_with(AGENT_ID_PREFIX) {
        return agent_result(engine, key, &job_id, body);
    }
    let result: crate::models::ExternalResult = match serde_json::from_value(body.clone()) {
        Ok(result) => result,
        Err(error) => return bad_request(&error.to_string()),
    };
    match engine.external_result(key, &job_id, result, &actor_of(body)) {
        Ok(reply) => FlowReply::ok(reply.into_body()),
        Err(error) => error_reply(&error),
    }
}

/// The body as `T`, or `T`'s default when the caller sent none.
fn body_or_default<T: serde::de::DeserializeOwned + Default>(body: &Value) -> Result<T, FlowReply> {
    if body.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(body.clone()).map_err(|error| bad_request(&error.to_string()))
}

fn agent_reply<T: serde::Serialize>(result: anyhow::Result<T>) -> FlowReply {
    match result {
        Ok(reply) => FlowReply::ok(serde_json::to_value(reply).unwrap_or(Value::Null)),
        Err(error) => error_reply(&error),
    }
}

fn agent_result(engine: &FlowEngine, key: &str, job_id: &str, body: &Value) -> FlowReply {
    match serde_json::from_value::<crate::models::agent_job_result::AgentJobResult>(body.clone()) {
        Ok(result) => agent_reply(engine.agent_result(key, job_id, &result)),
        Err(error) => bad_request(&error.to_string()),
    }
}

fn agent_claim(engine: &FlowEngine, key: &str, id_segments: &[&str], body: &Value) -> FlowReply {
    match body_or_default(body) {
        Ok(request) => {
            agent_reply(engine.agent_claim(key, &id_segments.join(JOB_ID_SEPARATOR), &request))
        }
        Err(reply) => reply,
    }
}

fn agent_heartbeat(
    engine: &FlowEngine,
    key: &str,
    id_segments: &[&str],
    body: &Value,
) -> FlowReply {
    match body_or_default(body) {
        Ok(request) => {
            agent_reply(engine.agent_heartbeat(key, &id_segments.join(JOB_ID_SEPARATOR), &request))
        }
        Err(reply) => reply,
    }
}

fn read_run(engine: &FlowEngine, key: &str) -> FlowReply {
    match engine.view(key) {
        Ok(view) => FlowReply::ok(view),
        Err(error) => error_reply(&error),
    }
}

fn read_journal(engine: &FlowEngine, key: &str) -> FlowReply {
    match engine.journal(key) {
        Ok(lines) => FlowReply::ok(json!({ KEY_FIELD: key, ENTRIES_FIELD: lines })),
        Err(error) => error_reply(&error),
    }
}

/// Answers a request on `/api/flows`, or `None` when the path is not a flow
/// route. A known path with the wrong method is also `None`.
pub fn handle(
    engine: &Arc<FlowEngine>,
    method: &Method,
    path: &str,
    query: &HashMap<String, String>,
    body: &Value,
) -> Option<FlowReply> {
    let trimmed = path.strip_prefix(API_PREFIX)?.trim_matches('/');
    let decoded = crate::path_segments::split(trimmed);
    let segments: Vec<&str> = decoded.iter().map(String::as_str).collect();
    if segments.first() != Some(&FLOWS_SEGMENT) {
        return None;
    }
    match (method, &segments[1..]) {
        (Method::Get, []) => Some(list(engine, query)),
        (Method::Post, []) => Some(start(engine, body)),
        (Method::Get, [key]) => Some(read_run(engine, key)),
        (Method::Get, [key, JOURNAL_SEGMENT]) => Some(read_journal(engine, key)),
        (Method::Get, [key, EXTERNAL_SEGMENT]) => Some(read_external_jobs(engine, key)),
        (Method::Post, [key, EXTERNAL_SEGMENT, id @ .., RESULT_SEGMENT]) if !id.is_empty() => {
            Some(external_result(engine, key, id, body))
        }
        (Method::Post, [key, EXTERNAL_SEGMENT, id @ .., CLAIM_SEGMENT]) if !id.is_empty() => {
            Some(agent_claim(engine, key, id, body))
        }
        (Method::Post, [key, EXTERNAL_SEGMENT, id @ .., HEARTBEAT_SEGMENT]) if !id.is_empty() => {
            Some(agent_heartbeat(engine, key, id, body))
        }
        (Method::Post, [key, verb]) => run_control(engine, key, verb, body),
        (Method::Post, [key, STEPS_SEGMENT, step, verb]) => {
            step_decision(engine, key, step, verb, body)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/http_routes_tests.rs"]
mod tests;
