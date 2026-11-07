use serde::Serialize;

/// Error payload of the native run routes.
#[derive(Serialize)]
pub struct NativeErrorBody {
    pub error: String,
    pub code: &'static str,
}
