use crate::models::enums::PointState;
use serde::{Deserialize, Serialize};

/// Body of `POST /api/prs/comments/state`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentStateRequest {
    pub repo: String,
    pub number: u64,
    pub point_id: String,
    pub state: PointState,
}
