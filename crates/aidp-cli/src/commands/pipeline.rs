use super::CommandOutput;
use crate::args::PipelineOp;
use crate::ctx::Ctx;
use crate::pipelines::{compose_roots, list_pipelines, resolve_pipeline_ref, show_pipeline};
use std::path::Path;

pub fn run(ctx: &Ctx, op: PipelineOp) -> anyhow::Result<CommandOutput> {
    match op {
        PipelineOp::List => Ok(CommandOutput::text(list_pipelines(
            ctx,
            &ctx.workspace(),
            ctx.format(),
        )?)),
        PipelineOp::Show { id_or_path } => Ok(CommandOutput::text(show_pipeline(
            ctx,
            &ctx.workspace(),
            &id_or_path,
        )?)),
        PipelineOp::Expand { pipeline } => {
            let workspace = ctx.workspace();
            let path = resolve_pipeline_ref(ctx, &workspace, &pipeline)?;
            let pipe = aidp_core::pipeline::load(&path)?;
            let roots = compose_roots(ctx, &workspace, &path);
            let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
            let effective = aidp_core::compose::expand(&pipe, &refs)?;
            Ok(CommandOutput::text(serde_yaml::to_string(&effective)?))
        }
    }
}
