use serde::Deserialize;

/// A Jira status reference, reduced to its name.
#[derive(Debug, Deserialize)]
pub struct StatusRef {
    pub name: String,
}
