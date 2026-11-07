use super::CommandOutput;
use crate::args::GitOp;
use crate::ctx::Ctx;

const COMMIT_WINDOW: usize = 200;

pub fn run(ctx: &Ctx, op: GitOp) -> anyhow::Result<CommandOutput> {
    match op {
        GitOp::Outcomes {
            since,
            window,
            overlap,
        } => {
            let workspace = ctx.workspace();
            let commits = ctx.git.log(&workspace, COMMIT_WINDOW, since.as_deref())?;
            let verdicts = aidp_core::gitlog::verdicts(&commits, window, overlap);
            let out: Vec<_> = commits
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "sha": c.sha, "subject": c.subject, "ts": c.ts,
                        "verdict": verdicts.get(&c.sha),
                    })
                })
                .collect();
            Ok(CommandOutput::json(
                ctx.format(),
                serde_json::Value::Array(out),
            ))
        }
    }
}
