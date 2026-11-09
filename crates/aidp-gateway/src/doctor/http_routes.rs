//! `GET /api/flows/:key/doctor` and `GET /api/doctor`: read only.

use super::Doctor;
use crate::flows::FlowEngine;
use crate::models::enums::ApiStatus;
use serde_json::{json, Value};
use std::sync::Arc;
use tiny_http::Method;

const API_PREFIX: &str = "/api";
const PATH_SEPARATOR: char = '/';
const FLOWS_SEGMENT: &str = "flows";
const DOCTOR_SEGMENT: &str = "doctor";
const UNAVAILABLE: &str = "doctor unavailable: no flow engine";
const STATUS_OK: u16 = 200;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_UNAVAILABLE: u16 = 503;

fn failure(code: u16, message: String) -> (u16, Value) {
    let status = ApiStatus::from_status(code);
    (code, json!({ "error": message, "code": status.code() }))
}

/// Answers the doctor routes; `None` for any other path or method.
pub fn handle(
    doctor: Option<&Arc<Doctor>>,
    flows: Option<&Arc<FlowEngine>>,
    method: &Method,
    path: &str,
) -> Option<(u16, Value)> {
    if *method != Method::Get {
        return None;
    }
    let segments: Vec<String> = crate::path_segments::split(
        path.trim_start_matches(API_PREFIX)
            .trim_matches(PATH_SEPARATOR),
    );
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    let key = match segments.as_slice() {
        [DOCTOR_SEGMENT] => None,
        [FLOWS_SEGMENT, key, DOCTOR_SEGMENT] => Some(*key),
        _ => return None,
    };
    let Some(doctor) = doctor else {
        return Some(failure(STATUS_UNAVAILABLE, UNAVAILABLE.to_string()));
    };
    let settings = doctor.settings();
    let Some(key) = key else {
        return Some((
            STATUS_OK,
            json!({ "settings": settings, "incidents": doctor.incidents() }),
        ));
    };
    let incidents = doctor.incidents_for(key);
    let known = flows.is_some_and(|engine| engine.get(key).is_ok());
    if incidents.is_empty() && !known {
        return Some(failure(
            STATUS_NOT_FOUND,
            format!("no flow run for `{key}`"),
        ));
    }
    Some((
        STATUS_OK,
        json!({ "key": key, "enabled": settings.enabled, "incidents": incidents }),
    ))
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_http_tests.rs"]
mod tests;
