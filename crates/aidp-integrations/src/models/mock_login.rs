use serde::{Deserialize, Serialize};

/// A GitHub account reference carrying only its login.
#[derive(Clone, Serialize, Deserialize)]
pub struct Login {
    pub login: String,
}
