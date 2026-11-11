//! `aidp memory model`: drive the one embedding model the daemon owns.

use super::gh::daemon_failure;
use super::memory::wants_json;
use super::CommandOutput;
use crate::args::memory::MemoryModelOp;
use crate::ctx::Ctx;
use crate::models::model_status_fallback::ModelStatusFallback;
use aidp_gateway::client;
use aidp_gateway::models::gateway_unreachable::GatewayUnreachable;
use aidp_memory::models::enums::model_state::ModelState;
use aidp_memory::models::model_status_body::ModelStatusBody;
use aidp_memory::{model_status, MemoryStore};
use serde_json::Value;

const ACTION_START: &str = "start";
const ACTION_STOP: &str = "stop";
const ACTION_EMBED_PENDING: &str = "embed-pending";
const BYTES_PER_MEGABYTE: u64 = 1024 * 1024;
const NO_DAEMON: &str = "daemon: arrete (statut lu sur le disque, aucun modele charge)";
const NONE: &str = "-";

/// Runs one `aidp memory model` verb against the daemon.
pub fn run(ctx: &Ctx, op: MemoryModelOp) -> anyhow::Result<CommandOutput> {
    let reply = match op {
        MemoryModelOp::Status => return status(ctx),
        MemoryModelOp::Start => client::memory_model_action(&None, ACTION_START),
        MemoryModelOp::Stop => client::memory_model_action(&None, ACTION_STOP),
        MemoryModelOp::Autostart { value } => {
            client::memory_model_autostart(&None, value.enabled())
        }
        MemoryModelOp::EmbedPending => client::memory_model_action(&None, ACTION_EMBED_PENDING),
    };
    render(ctx, reply.map_err(daemon_failure)?)
}

fn render(ctx: &Ctx, value: Value) -> anyhow::Result<CommandOutput> {
    if wants_json(ctx) {
        return Ok(CommandOutput::json(ctx.format(), value));
    }
    match serde_json::from_value::<ModelStatusBody>(value.clone()) {
        Ok(body) => Ok(CommandOutput::text(describe(&body))),
        Err(_) => Ok(CommandOutput::json(ctx.format(), value)),
    }
}

fn status(ctx: &Ctx) -> anyhow::Result<CommandOutput> {
    match client::memory_model_status(&None) {
        Ok(value) => render(ctx, value),
        Err(error) if error.downcast_ref::<GatewayUnreachable>().is_some() => offline(ctx),
        Err(error) => Err(daemon_failure(error)),
    }
}

fn offline(ctx: &Ctx) -> anyhow::Result<CommandOutput> {
    let gateway = aidp_gateway::paths::state_dir()?;
    let store = MemoryStore::open_in(&gateway)?;
    let status = model_status(&store, &gateway)?;
    if wants_json(ctx) {
        let fallback = ModelStatusFallback {
            daemon_running: false,
            status,
        };
        return Ok(CommandOutput::json(
            ctx.format(),
            serde_json::to_value(&fallback)?,
        ));
    }
    Ok(CommandOutput::text(format!(
        "{NO_DAEMON}\nmodele: {}\nchemin: {}\ninstalle: {}\nchunks vectorises: {} / {}\nchunks d'un autre modele: {}\n",
        status.model,
        status.path.display(),
        yes_no(status.present),
        status.embedded_chunks,
        status.total_chunks,
        status.stale_chunks,
    )))
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "oui"
    } else {
        "non"
    }
}

fn describe(body: &ModelStatusBody) -> String {
    let state = match body.state {
        ModelState::Error => format!("error ({})", body.error.as_deref().unwrap_or(NONE)),
        other => other.as_str().to_string(),
    };
    format!(
        "modele: {}\nchemin: {}\ninstalle: {}\netat: {}\ndemarrage automatique: {}\ndimension: {}\nmemoire (estimation): {}\ncharge le: {}\nchunks vectorises: {} / {}\nchunks d'un autre modele: {}",
        body.model,
        body.path,
        yes_no(body.installed),
        state,
        yes_no(body.autostart),
        body.dim.map_or_else(|| NONE.to_string(), |dim| dim.to_string()),
        body.ram_bytes.map_or_else(
            || NONE.to_string(),
            |bytes| format!("{} Mo", bytes / BYTES_PER_MEGABYTE)
        ),
        body.loaded_at.as_deref().unwrap_or(NONE),
        body.chunks.embedded,
        body.chunks.total,
        body.chunks.stale,
    )
}
