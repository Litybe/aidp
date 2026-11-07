//! The input of the GitHub `createRef` mutation.

use serde::Serialize;

/// A new ref `name` of the repository `repository_id`, pointing at `oid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRefInput {
    pub repository_id: String,
    pub name: String,
    pub oid: String,
}
