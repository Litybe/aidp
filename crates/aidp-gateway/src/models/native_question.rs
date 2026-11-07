use serde::{Deserialize, Serialize};

/// A question a stage agent raised, with the options it offered.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NativeQuestion {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub recommended: Option<String>,
}
