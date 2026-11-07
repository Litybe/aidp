use aidp_core::models::enums::Stack;
use serde::Serialize;

/// Output of `aidp project stack`.
#[derive(Serialize)]
pub struct ProjectStack {
    pub path: String,
    pub stack: Stack,
}
