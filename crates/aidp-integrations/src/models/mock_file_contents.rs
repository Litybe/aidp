use serde::Serialize;

/// Reply of the GitHub contents call for a file.
#[derive(Serialize)]
pub struct FileContents {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub encoding: &'static str,
    pub content: String,
}
