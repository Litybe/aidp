use serde::{Deserialize, Serialize};

/// How bot-authored pull requests are treated by a listing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotsFilter {
    #[default]
    Hide,
    Show,
    Only,
}
