use serde::{Deserialize, Serialize};

/// Launch origins the gateway distinguishes by behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginKind {
    Api,
    Ui,
    Ci,
    Flow,
    Review,
    Run,
    #[serde(rename = "pr-fix")]
    PrFix,
}

impl OriginKind {
    /// Persisted and serialized spelling of the origin kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            OriginKind::Api => "api",
            OriginKind::Ui => "ui",
            OriginKind::Ci => "ci",
            OriginKind::Flow => "flow",
            OriginKind::Review => "review",
            OriginKind::Run => "run",
            OriginKind::PrFix => "pr-fix",
        }
    }
}

impl std::str::FromStr for OriginKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(value.to_string())).map_err(|_| ())
    }
}
