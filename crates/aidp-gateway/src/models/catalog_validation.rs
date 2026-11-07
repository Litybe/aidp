use serde::Serialize;

#[derive(Serialize)]
pub struct CatalogValidation {
    pub valid: bool,
    pub errors: Vec<String>,
}
