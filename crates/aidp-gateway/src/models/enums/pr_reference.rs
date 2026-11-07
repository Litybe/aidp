use serde::{Deserialize, Serialize};

/// A pull request named by number or by url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrReference {
    Number(u64),
    Url(String),
}

impl PrReference {
    /// The reference `gh` accepts, `None` for a blank url.
    pub fn gh_reference(&self) -> Option<String> {
        match self {
            PrReference::Number(number) => Some(number.to_string()),
            PrReference::Url(url) => {
                let trimmed = url.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
        }
    }
}
