use serde::Deserialize;

/// Groups of synonyms, French and English, that widen a query.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Glossary {
    #[serde(default)]
    pub groups: Vec<Vec<String>>,
}
