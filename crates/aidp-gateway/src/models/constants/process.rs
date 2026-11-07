//! Program names and arguments of the processes the gateway spawns.

pub const GIT_PROGRAM: &str = "git";
pub const GH_PROGRAM: &str = "gh";
pub const CAT_FILE: &str = "cat-file";
pub const EXISTS_FLAG: &str = "-e";
/// Windows creation flag that keeps a spawned console process from opening a window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
