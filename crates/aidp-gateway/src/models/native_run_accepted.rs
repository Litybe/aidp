use serde::Serialize;

/// Payload acknowledging a run action.
#[derive(Serialize)]
pub struct NativeRunAccepted {
    pub ok: bool,
}
