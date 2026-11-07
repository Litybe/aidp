use serde::Serialize;

/// One pipeline that failed `aidp validate --all`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct PipelineValidationFailure<'a> {
    pub error: String,
    pub id: &'a str,
    pub ok: bool,
    pub path: String,
}
