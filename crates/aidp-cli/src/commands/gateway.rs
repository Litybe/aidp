use super::CommandOutput;
use crate::args::gateway::{AutoOp, DispatcherOp, Op, PresetOp, ProjOp, QueueOp, WorktreesOp};
use crate::ctx::Ctx;
use crate::models::enums::engine_arg::EngineArg;
use crate::output;
use aidp_core::Format;
use aidp_gateway::cli_api::{self, Reply, StartParams, TextView};
use aidp_gateway::client;

/// Renders a daemon reply in the requested format.
fn render(ctx: &Ctx, reply: Reply) -> anyhow::Result<CommandOutput> {
    let format = ctx.format().unwrap_or(Format::JsonPretty);
    let rendered = match format {
        Format::Text => reply.text(),
        other => output::render_value(other, &reply.value)?,
    };
    Ok(CommandOutput::text(rendered))
}

pub fn run(ctx: &Ctx, op: Op) -> anyhow::Result<CommandOutput> {
    match op {
        Op::Serve {
            port,
            bind,
            state_dir,
            ui_dir,
        } => {
            cli_api::serve(bind, port, state_dir, ui_dir)?;
            Ok(CommandOutput::empty())
        }
        Op::Status { url } => render(ctx, cli_api::status(&url)?),
        Op::Sessions { url, status, limit } => {
            render(ctx, cli_api::sessions(&url, status.as_deref(), limit)?)
        }
        Op::Start {
            url,
            project,
            template,
            engine,
            target,
            inputs,
            prompt,
            permission_mode,
            model,
            effort,
            max_budget_usd,
            context_from,
            keep_open,
            workspace,
        } => {
            let params = StartParams {
                project,
                template,
                engine: engine.map(EngineArg::into_engine),
                target,
                inputs,
                prompt,
                permission_mode,
                model,
                effort,
                max_budget_usd,
                context_from,
                keep_open,
                workspace: workspace.map(|boxed| *boxed),
            };
            render(ctx, cli_api::start(&url, params)?)
        }
        Op::Reply { url, id, text } => render(ctx, cli_api::reply(&url, &id, &text)?),
        Op::Cancel { url, id } => render(ctx, cli_api::cancel(&url, &id)?),
        Op::Automations { url, op } => automations(ctx, &url, op),
        Op::Projects { url, op } => projects(ctx, &url, op),
        Op::Presets { url, op } => presets(ctx, &url, op),
        Op::Queue { url, op } => queue(ctx, &url, op),
        Op::Worktrees { url, op } => worktrees(ctx, &url, op),
        Op::Dispatcher { url, op } => dispatcher(ctx, &url, op),
        Op::Events { url, limit } => render(ctx, cli_api::events(&url, limit)?),
    }
}

fn automations(ctx: &Ctx, url: &Option<String>, op: AutoOp) -> anyhow::Result<CommandOutput> {
    match op {
        AutoOp::List {} => render(ctx, cli_api::automations_list(url)?),
        AutoOp::Add { file } => render(ctx, cli_api::automations_add(url, &file)?),
        AutoOp::Rm { id } => render(ctx, cli_api::automations_rm(url, &id)?),
        AutoOp::Run { id } => render(ctx, cli_api::automations_run(url, &id)?),
        AutoOp::Enable { id } => render(ctx, cli_api::automations_set_enabled(url, &id, true)?),
        AutoOp::Disable { id } => render(ctx, cli_api::automations_set_enabled(url, &id, false)?),
        AutoOp::Runs { id, limit } => {
            render(ctx, cli_api::automations_runs(url, id.as_deref(), limit)?)
        }
    }
}

fn projects(ctx: &Ctx, url: &Option<String>, op: ProjOp) -> anyhow::Result<CommandOutput> {
    match op {
        ProjOp::List => render(ctx, cli_api::projects_list(url)?),
        ProjOp::Add { path } => render(ctx, cli_api::projects_add(url, &path)?),
        ProjOp::Rm { id } => render(ctx, cli_api::projects_rm(url, &id)?),
        ProjOp::Import { file } => render(ctx, cli_api::projects_import(url, &file)?),
    }
}

fn presets(ctx: &Ctx, url: &Option<String>, op: PresetOp) -> anyhow::Result<CommandOutput> {
    match op {
        PresetOp::List {} => render(ctx, cli_api::presets_list(url)?),
        PresetOp::Add { file } => render(ctx, cli_api::presets_add(url, &file)?),
        PresetOp::Rm { id } => render(ctx, cli_api::presets_rm(url, &id)?),
        PresetOp::Start { id } => render(ctx, cli_api::presets_start(url, &id)?),
    }
}

fn queue(ctx: &Ctx, url: &Option<String>, op: QueueOp) -> anyhow::Result<CommandOutput> {
    match op {
        QueueOp::List { project_id } => {
            render(ctx, cli_api::queue_list(url, project_id.as_deref())?)
        }
        QueueOp::Promote { id } => render(ctx, cli_api::queue_promote(url, &id)?),
        QueueOp::Cancel { id } => render(ctx, cli_api::queue_cancel(url, &id)?),
    }
}

fn dispatcher(ctx: &Ctx, url: &Option<String>, op: DispatcherOp) -> anyhow::Result<CommandOutput> {
    match op {
        DispatcherOp::Status {} => render(ctx, cli_api::dispatcher_status(url)?),
        DispatcherOp::Pause => render(ctx, cli_api::dispatcher_pause(url)?),
        DispatcherOp::Resume => render(ctx, cli_api::dispatcher_resume(url)?),
        DispatcherOp::Tick { dry_run } => render(ctx, cli_api::dispatcher_run_tick(url, dry_run)?),
    }
}

fn worktrees(ctx: &Ctx, url: &Option<String>, op: WorktreesOp) -> anyhow::Result<CommandOutput> {
    let value = match op {
        WorktreesOp::List { project } => client::worktrees_list(url, project.as_deref())?,
        WorktreesOp::Prune { dry_run } => client::worktrees_prune(url, dry_run)?,
        WorktreesOp::Ensure { key } => client::worktrees_ensure(url, &key)?,
        WorktreesOp::Remove { key } => client::worktrees_remove(url, &key)?,
    };
    render(
        ctx,
        Reply {
            value,
            view: TextView::Raw,
        },
    )
}
