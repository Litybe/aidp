use serde::{Deserialize, Serialize};

/// Where a fan-out takes its shard labels from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", content = "value", rename_all = "snake_case")]
pub enum FanoutItems {
    /// Labels written in the template.
    Static(Vec<String>),
    /// An upstream array signal, `stages.<stage>.signals.<prop>`, known only at run time.
    FromSignal(String),
}
