use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CatalogDocument<T> {
    pub parsed: T,
    pub raw: String,
}
