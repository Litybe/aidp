use crate::claude_provider::StdinCell;

/// Writer over the child's stdin; dropping an owning guard closes the pipe.
pub struct StdinGuard {
    pub(crate) cell: StdinCell,
    pub(crate) close_on_drop: bool,
}
