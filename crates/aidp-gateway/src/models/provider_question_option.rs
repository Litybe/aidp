use serde::Serialize;

#[derive(Serialize)]
pub struct ProviderQuestionOption {
    pub label: String,
    pub description: String,
}
