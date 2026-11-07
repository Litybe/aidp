use crate::models::enums::Stack;
use serde::Serialize;

/// The `stack` input of a run, as serialized into its inputs object.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct StackInputs {
    pub stack: Stack,
}
