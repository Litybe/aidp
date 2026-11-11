use serde::{Deserialize, Serialize};

/// The kind of work a knowledge block is meant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KnowledgeRoute {
    Code,
    Art,
    Map,
    Doc,
    Tooling,
}

impl KnowledgeRoute {
    /// Wire value, identical to the serialized form.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Art => "art",
            Self::Map => "map",
            Self::Doc => "doc",
            Self::Tooling => "tooling",
        }
    }
}
