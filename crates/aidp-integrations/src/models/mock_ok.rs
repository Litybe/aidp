use serde::Serialize;

/// Acknowledgement body of the internal routes.
#[derive(Serialize)]
pub struct MockOk {
    pub ok: bool,
}
