use serde::{Deserialize, Serialize};

/// Permission mode a session is launched with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "bypassPermissions")]
    BypassPermissions,
    #[serde(rename = "dontAsk")]
    DontAsk,
    #[serde(rename = "auto")]
    Auto,
}

impl PermissionMode {
    /// Every variant, for callers that must cover the whole set.
    pub const ALL: [PermissionMode; 6] = [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::BypassPermissions,
        PermissionMode::DontAsk,
        PermissionMode::Auto,
    ];

    /// What a native stage session asks claude for first.
    pub const STAGE_PREFERRED: PermissionMode = PermissionMode::BypassPermissions;

    /// What a native stage session asks for once claude refused [`PermissionMode::STAGE_PREFERRED`].
    pub const STAGE_FALLBACK: PermissionMode = PermissionMode::Auto;

    /// What a headless launch runs in when it asked for no mode or for `default`.
    pub const HEADLESS_DEFAULT: PermissionMode = PermissionMode::DontAsk;

    /// A headless launch's mode: `requested`, or [`Self::HEADLESS_DEFAULT`] when absent or `default`.
    pub const fn headless(requested: Option<PermissionMode>) -> PermissionMode {
        match requested {
            None | Some(PermissionMode::Default) => Self::HEADLESS_DEFAULT,
            Some(mode) => mode,
        }
    }

    /// The wire spelling of the mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Plan => "plan",
            PermissionMode::BypassPermissions => "bypassPermissions",
            PermissionMode::DontAsk => "dontAsk",
            PermissionMode::Auto => "auto",
        }
    }
}
