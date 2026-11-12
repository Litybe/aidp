use super::CommandOutput;
use crate::args::AgentsOp;
use crate::ctx::{claude_agents_dir, Ctx};

const MARKDOWN_EXTENSION: &str = "md";

pub fn run(ctx: &Ctx, op: AgentsOp) -> anyhow::Result<CommandOutput> {
    match op {
        AgentsOp::List => list_agents(ctx),
    }
}

fn list_agents(ctx: &Ctx) -> anyhow::Result<CommandOutput> {
    let mut out = vec![];
    if let Some(dir) = claude_agents_dir(ctx) {
        if dir.exists() {
            for entry in walkdir::WalkDir::new(&dir)
                .max_depth(1)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let is_markdown = entry
                    .path()
                    .extension()
                    .map(|x| x == MARKDOWN_EXTENSION)
                    .unwrap_or(false);
                if !is_markdown {
                    continue;
                }
                if let Some(name) = entry.path().file_stem().and_then(|s| s.to_str()) {
                    out.push(serde_json::json!({"name": name, "path": entry.path()}));
                }
            }
        }
    }
    Ok(CommandOutput::text(
        serde_json::Value::Array(out).to_string(),
    ))
}
