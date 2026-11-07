use serde::Serialize;

/// Envelope of every action response and bus payload; `ticket` is the key carrying the PR.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionBody<T: Serialize> {
    pub repo: String,
    pub number: u64,
    #[serde(flatten)]
    pub result: T,
    pub ticket: Option<String>,
}
