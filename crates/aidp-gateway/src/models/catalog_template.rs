use aidp_core::pipeline::Pipeline;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogTemplate {
    #[serde(flatten)]
    pub pipeline: Pipeline,
    pub source: String,
    pub input_count: usize,
    pub stage_count: usize,
}
