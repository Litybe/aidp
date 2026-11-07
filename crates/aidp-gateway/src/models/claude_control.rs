use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::exit_signal::ExitSignal;
use crate::claude_provider::{ChildCell, PermissionCell, StdinCell};

/// Cancels, answers permissions for and awaits one running Claude child.
pub(crate) struct ClaudeControl {
    pub(crate) child: ChildCell,
    pub(crate) stdin: StdinCell,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) exit: Arc<ExitSignal>,
    pub(crate) permissions: PermissionCell,
}
