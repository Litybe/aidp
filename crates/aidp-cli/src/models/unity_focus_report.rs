//! What `flow unity-focus` prints: whether the editor of the workspace got the foreground.

use serde::Serialize;

/// The outputs of `flow unity-focus`. `editor_found` is false when no editor is open on the
/// workspace, or it shows no main window.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct UnityFocusReport {
    pub focused: bool,
    pub editor_found: bool,
    pub dialogs: Vec<String>,
    pub message: String,
}
