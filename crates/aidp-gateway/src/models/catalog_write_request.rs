use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogWriteRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub content: String,
}
