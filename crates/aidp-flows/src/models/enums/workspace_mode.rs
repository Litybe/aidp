use serde::{Deserialize, Serialize};

/// Where the steps of a flow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    /// The main checkout of the project, on the branch of the ticket. The run holds the project's
    /// `unity-editor` resource from its first step until its checkout is given back (`git.restore`).
    Project,
}
