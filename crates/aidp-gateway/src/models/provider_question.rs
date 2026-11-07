use serde::Serialize;

#[derive(Serialize)]
pub struct ProviderQuestion {
    pub question: String,
    pub header: String,
    pub options: Vec<crate::models::provider_question_option::ProviderQuestionOption>,
}
