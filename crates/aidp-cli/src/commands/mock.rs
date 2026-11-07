use super::CommandOutput;
use crate::ctx::Ctx;
use std::path::PathBuf;

pub struct MockArgs {
    pub addr: String,
    pub data: PathBuf,
}

pub fn run(_ctx: &Ctx, args: MockArgs) -> anyhow::Result<CommandOutput> {
    aidp_integrations::mock::serve(&args.addr, &args.data)?;
    Ok(CommandOutput::empty())
}
