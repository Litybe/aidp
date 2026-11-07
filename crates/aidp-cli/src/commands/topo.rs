use super::CommandOutput;
use crate::ctx::Ctx;
use crate::pipelines::resolve_pipeline_ref;

pub struct TopoArgs {
    pub pipeline: String,
}

pub fn run(ctx: &Ctx, args: TopoArgs) -> anyhow::Result<CommandOutput> {
    let path = resolve_pipeline_ref(ctx, &ctx.cwd, &args.pipeline)?;
    let pipeline = aidp_core::pipeline::load(&path)?;
    let layers = aidp_core::pipeline::topo_sort(&pipeline)?;
    Ok(CommandOutput::text(serde_json::to_string_pretty(&layers)?))
}

#[cfg(test)]
#[path = "../../tests/unit/topo_tests.rs"]
mod tests;
