use super::CommandOutput;
use crate::args::project::ProjectOp;
use crate::ctx::Ctx;
use crate::models::project_stack::ProjectStack;
use aidp_core::{Error, Format};
use std::path::PathBuf;

const CURRENT_DIR: &str = ".";
const MISSING_DIRECTORY: &str = "project directory not found";

/// Runs one `aidp project` verb.
pub fn run(_ctx: &Ctx, op: ProjectOp) -> anyhow::Result<CommandOutput> {
    match op {
        ProjectOp::Stack { path } => stack(path.unwrap_or_else(|| PathBuf::from(CURRENT_DIR))),
    }
}

/// Detects the stack of `path` and reports it with the absolute path.
fn stack(path: PathBuf) -> anyhow::Result<CommandOutput> {
    if !path.is_dir() {
        return Err(Error::NotFound(format!("{MISSING_DIRECTORY}: {}", path.display())).into());
    }
    let absolute = std::fs::canonicalize(&path)?;
    let report = ProjectStack {
        stack: aidp_core::stack::detect_project(&absolute),
        path: absolute.display().to_string(),
    };
    Ok(CommandOutput::json(
        Some(Format::JsonCompact),
        serde_json::to_value(report)?,
    ))
}
