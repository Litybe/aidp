use serde::Serialize;

#[derive(Serialize)]
pub struct CatalogSaved {
    pub success: bool,
    pub id: String,
}
