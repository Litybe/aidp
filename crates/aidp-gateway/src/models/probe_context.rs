use crate::models::PrStatusSource;
use serde_json::Value;
use std::sync::Arc;

/// What a probe sees when it is asked whether its fact has happened.
pub struct ProbeContext {
    pub key: String,
    pub now_ms: i64,
    pub started_ms: i64,
    /// The state the probe returned on its previous poll, if any.
    pub last_state: Option<Value>,
    pub pr_status: Arc<dyn PrStatusSource>,
}
