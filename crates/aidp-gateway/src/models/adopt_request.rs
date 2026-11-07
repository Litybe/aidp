use crate::models::enums::PrReference;
use serde::{Deserialize, Serialize};

/// Body of `POST /api/tickets/<key>/adopt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptRequest {
    pub pr: PrReference,
}
