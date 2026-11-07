use crate::models::enums::ApiStatus;
use crate::models::setting_issue::SettingIssue;
use serde_json::{json, Value};

/// Why a settings request did not succeed.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsFailure {
    /// The body holds refused values, reported one by one.
    Issues(Vec<SettingIssue>),
    /// Any other failure, with its status.
    Api(ApiStatus, String),
}

impl SettingsFailure {
    /// The HTTP status and JSON body of the failure.
    pub fn into_reply(self) -> (u16, Value) {
        match self {
            SettingsFailure::Issues(errors) => (
                ApiStatus::BadRequest.status(),
                json!({
                    "error": "invalid settings",
                    "code": ApiStatus::BadRequest.code(),
                    "errors": errors,
                }),
            ),
            SettingsFailure::Api(status, message) => (
                status.status(),
                json!({ "error": message, "code": status.code() }),
            ),
        }
    }
}
