use crate::models::enums::EffectKind;
use serde::{Deserialize, Serialize};

/// An effect with its deterministic id: `<step>:<iteration>:<kind>` or
/// `flow:<seq>:journal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub id: String,
    #[serde(flatten)]
    pub kind: EffectKind,
}
