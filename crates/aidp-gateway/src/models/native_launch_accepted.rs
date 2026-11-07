use crate::models::enums::engine::Engine;
use serde::Serialize;

/// Answer of a template launch served by the native engine.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeLaunchAccepted {
    pub ok: bool,
    pub run_id: String,
    pub mode: Engine,
}
