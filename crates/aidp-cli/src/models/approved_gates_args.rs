use serde::Serialize;

/// The `args` object handed to the Workflow tool on resume.
#[derive(Serialize)]
pub struct ApprovedGatesArgs<'a> {
    pub approved_gates: [&'a String; 1],
}
