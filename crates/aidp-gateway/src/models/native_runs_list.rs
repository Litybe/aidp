use crate::models::native_run_view::NativeRunView;
use serde::Serialize;

/// Payload of `GET /api/runs`.
#[derive(Serialize)]
pub struct NativeRunsList {
    pub entries: Vec<NativeRunView>,
}
