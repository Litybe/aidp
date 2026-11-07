//! Daemon resources a step can hold.

/// The Unity editor of a project: one holder at a time.
pub const UNITY_EDITOR: &str = "unity-editor";
/// Resources a flow may name in `resource:`.
pub const KNOWN: [&str; 1] = [UNITY_EDITOR];
/// Prefix of the reason of a step waiting for a resource: `resource:unity-editor`.
pub const REASON_PREFIX: &str = "resource:";
