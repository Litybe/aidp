use serde::{Deserialize, Serialize};

/// The project stack `aidp project stack` reports for a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stack {
    Csharp,
    LambdaTs,
    Angular,
    Rust,
    TsNode,
    Unknown,
}
