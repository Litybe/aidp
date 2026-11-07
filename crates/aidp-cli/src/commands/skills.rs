use super::CommandOutput;
use crate::args::SkillsOp;
use crate::ctx::Ctx;

const SKILL_MANIFEST: &str = "SKILL.md";
const USER_SKILLS_RELATIVE: &str = ".claude/skills";
const USER_SKILLS_DEPTH: usize = 2;

pub fn run(ctx: &Ctx, op: SkillsOp) -> anyhow::Result<CommandOutput> {
    match op {
        SkillsOp::Resolve { name } => resolve_skill(&name),
        SkillsOp::List => list_skills(ctx),
    }
}

fn resolve_skill(name: &str) -> anyhow::Result<CommandOutput> {
    let search = aidp_core::skills::default_search_paths();
    let refs: Vec<&std::path::Path> = search.iter().map(|p| p.as_path()).collect();
    match aidp_core::skills::resolve(name, &refs) {
        Some(path) => Ok(CommandOutput::text(path.display().to_string())),
        None => anyhow::bail!("skill not found: {name}"),
    }
}

fn list_skills(ctx: &Ctx) -> anyhow::Result<CommandOutput> {
    let mut out = vec![];
    if let Some(home) = &ctx.home {
        let root = home.join(USER_SKILLS_RELATIVE);
        if root.exists() {
            for entry in walkdir::WalkDir::new(&root)
                .max_depth(USER_SKILLS_DEPTH)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let is_manifest = entry
                    .path()
                    .file_name()
                    .map(|n| n == SKILL_MANIFEST)
                    .unwrap_or(false);
                if !is_manifest {
                    continue;
                }
                if let Some(name) = entry
                    .path()
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                {
                    out.push(serde_json::json!({"name": name, "path": entry.path()}));
                }
            }
        }
    }
    Ok(CommandOutput::text(
        serde_json::Value::Array(out).to_string(),
    ))
}
