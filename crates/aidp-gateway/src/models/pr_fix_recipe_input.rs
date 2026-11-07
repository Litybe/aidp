use crate::models::pr_fix_thread_input::PrFixThreadInput;
use crate::models::selection_point::SelectionPoint;
use serde::{Deserialize, Serialize};

/// Everything the fix recipe interpolates: the PR, the selected threads and points, the note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrFixRecipeInput {
    pub repo: String,
    pub pr_number: u64,
    pub head_branch: String,
    #[serde(default)]
    pub threads: Vec<PrFixThreadInput>,
    #[serde(default)]
    pub points: Vec<SelectionPoint>,
    #[serde(default)]
    pub note: Option<String>,
}
