use crate::models::PrStatusSource;
use serde_json::Value;

/// A source that knows no PR.
pub struct NoPrStatusSource;

impl PrStatusSource for NoPrStatusSource {
    fn status(&self, _key: &str) -> Option<Value> {
        None
    }
}
