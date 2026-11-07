use serde::{Deserialize, Serialize};

/// A GitHub team reference carrying only its slug.
#[derive(Clone, Serialize, Deserialize)]
pub struct TeamSlug {
    pub slug: String,
}
