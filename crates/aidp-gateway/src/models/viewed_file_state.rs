use serde::Serialize;

/// Whether the viewer's last viewed blob of a file is still the current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewedFileState {
    pub path: String,
    pub viewed: bool,
}
