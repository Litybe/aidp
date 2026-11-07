use serde_json::Value;

const HEAD_SHA_POINTER: &str = "/head/sha";
const HEAD_REF_POINTER: &str = "/head/ref";
const AUTHOR_POINTER: &str = "/author";
const VIEWER_POINTER: &str = "/viewer";

/// The live facts a write is decided on, re-read from the PR status right before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrWriteState {
    pub head_sha: String,
    pub head_ref: String,
    pub author: Option<String>,
    pub viewer: Option<String>,
}

fn text_at(status: &Value, pointer: &str) -> Option<String> {
    status.pointer(pointer)?.as_str().map(str::to_string)
}

impl PrWriteState {
    /// Reads the state out of a status document; `None` when it carries no head sha.
    pub fn from_status(status: &Value) -> Option<Self> {
        Some(PrWriteState {
            head_sha: text_at(status, HEAD_SHA_POINTER)?,
            head_ref: text_at(status, HEAD_REF_POINTER).unwrap_or_default(),
            author: text_at(status, AUTHOR_POINTER),
            viewer: text_at(status, VIEWER_POINTER),
        })
    }

    /// True when the viewer wrote the pull request.
    pub fn is_own_pr(&self) -> bool {
        matches!((&self.author, &self.viewer), (Some(author), Some(viewer)) if author.eq_ignore_ascii_case(viewer))
    }
}
