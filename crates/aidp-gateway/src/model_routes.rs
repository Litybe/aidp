//! `/api/memory/model` and `/api/embed`: start, stop and use the one embedding model of the daemon.
//!
//! Errors use the flat `{ "error", "code" }` shape with the codes `model_missing` and `model_stopped`.

use crate::http::App;
use crate::models::constants::model_service::{
    CODE_BATCH_TOO_LARGE, CODE_MODEL_MISSING, CODE_MODEL_STOPPED, SEGMENT_EMBED,
    SEGMENT_EMBED_PENDING, SEGMENT_MODEL, SEGMENT_START, SEGMENT_STOP,
};
use crate::models::enums::ApiStatus;
use crate::models::model_error_body::ModelErrorBody;
use crate::models::model_patch_body::ModelPatchBody;
use crate::models::model_queued::ModelQueued;
use aidp_memory::models::constants::remote::EMBED_MAX_TEXTS;
use aidp_memory::models::embed_request::EmbedRequest;
use aidp_memory::models::embed_response::EmbedResponse;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use tiny_http::Method;

const MEMORY_SEGMENT: &str = "memory";
const STATUS_OK: u16 = 200;
const STATUS_ACCEPTED: u16 = 202;

type Reply = (u16, Value);

fn json_reply(status: u16, body: &impl Serialize) -> Reply {
    (status, serde_json::to_value(body).unwrap_or(Value::Null))
}

fn failure(status: ApiStatus, code: &str, message: impl ToString) -> Reply {
    json_reply(
        status.status(),
        &ModelErrorBody {
            error: message.to_string(),
            code: code.to_string(),
        },
    )
}

fn plain_failure(status: ApiStatus, message: impl ToString) -> Reply {
    failure(status, status.code(), message)
}

fn parse<T: for<'de> serde::Deserialize<'de>>(body: &Value) -> Result<T, Reply> {
    serde_json::from_value(body.clone())
        .map_err(|error| plain_failure(ApiStatus::BadRequest, format!("invalid body: {error}")))
}

fn start(app: &App) -> Reply {
    if !app.memory.model().start() {
        return failure(
            ApiStatus::Conflict,
            CODE_MODEL_MISSING,
            "the embedding model is not installed",
        );
    }
    json_reply(STATUS_ACCEPTED, &app.memory.model().status())
}

fn stop(app: &App) -> Reply {
    app.memory.model().stop();
    json_reply(STATUS_OK, &app.memory.model().status())
}

fn patch(app: &App, body: &Value) -> Reply {
    let patch: ModelPatchBody = match parse(body) {
        Ok(patch) => patch,
        Err(reply) => return reply,
    };
    match app.memory.model().set_autostart(patch.autostart) {
        Ok(()) => json_reply(STATUS_OK, &app.memory.model().status()),
        Err(error) => plain_failure(ApiStatus::Io, error),
    }
}

fn embed_pending(app: &App) -> Reply {
    if app.memory.embedder().is_none() {
        return failure(
            ApiStatus::Conflict,
            CODE_MODEL_STOPPED,
            "the embedding model is not running",
        );
    }
    app.memory.enqueue_embed_pending();
    json_reply(STATUS_ACCEPTED, &ModelQueued { queued: true })
}

fn embed(app: &App, body: &Value) -> Reply {
    let request: EmbedRequest = match parse(body) {
        Ok(request) => request,
        Err(reply) => return reply,
    };
    if request.texts.len() > EMBED_MAX_TEXTS {
        return failure(
            ApiStatus::BadRequest,
            CODE_BATCH_TOO_LARGE,
            format!("at most {EMBED_MAX_TEXTS} texts per call"),
        );
    }
    let Some(embedder) = app.memory.embedder() else {
        return failure(
            ApiStatus::Conflict,
            CODE_MODEL_STOPPED,
            "the embedding model is not running",
        );
    };
    let texts: Vec<&str> = request.texts.iter().map(String::as_str).collect();
    match embedder.embed(&texts) {
        Ok(vectors) => json_reply(
            STATUS_OK,
            &EmbedResponse {
                model: embedder.model_id().to_string(),
                dim: embedder.dim(),
                vectors,
            },
        ),
        Err(error) => plain_failure(ApiStatus::Internal, error),
    }
}

/// Answers the model routes; `None` for any other path.
pub fn handle(
    app: &App,
    method: &Method,
    path: &str,
    _query: &HashMap<String, String>,
    body: &Value,
) -> Option<Reply> {
    let segments: Vec<&str> = path
        .trim_start_matches("/api")
        .trim_matches('/')
        .split('/')
        .collect();
    match (method, segments.as_slice()) {
        (Method::Post, [SEGMENT_EMBED]) => Some(embed(app, body)),
        (Method::Get, [MEMORY_SEGMENT, SEGMENT_MODEL]) => {
            Some(json_reply(STATUS_OK, &app.memory.model().status()))
        }
        (Method::Patch, [MEMORY_SEGMENT, SEGMENT_MODEL]) => Some(patch(app, body)),
        (Method::Post, [MEMORY_SEGMENT, SEGMENT_MODEL, action]) => match *action {
            SEGMENT_START => Some(start(app)),
            SEGMENT_STOP => Some(stop(app)),
            SEGMENT_EMBED_PENDING => Some(embed_pending(app)),
            _ => None,
        },
        _ => None,
    }
}
