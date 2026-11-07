//! The `aidp project` subcommand tree: facts about a project directory.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ProjectOp {
    /// Detect the stack of a project directory. Emits {path, stack}.
    Stack {
        /// Project directory; the current directory when omitted.
        path: Option<PathBuf>,
    },
}
