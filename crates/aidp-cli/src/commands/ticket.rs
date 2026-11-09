use super::CommandOutput;
use crate::args::tickets::Op;
use crate::args::TicketLoc;
use crate::ctx::Ctx;
use aidp_core::Format;
use aidp_tickets::cli as tickets;
use aidp_tickets::model::{IssueType, Priority, Source, Status};
use aidp_tickets::store::TicketStore;
use aidp_tickets::Track;
use aidp_tickets::{BodySource, ListQuery, NewTicket, TicketEdit};
use std::path::PathBuf;

const NO_VERDICT_LABEL: &str = "-";
const NO_RESET_LABEL: &str = "—";

/// Dispatches `aidp ticket`: the daemon verbs `start`, `dispatch` and `budget` are served here.
/// Every other verb stays on the local path and needs no daemon.
pub fn run(ctx: &Ctx, loc: TicketLoc, op: Op) -> anyhow::Result<CommandOutput> {
    match op {
        Op::Start {
            key,
            force,
            service,
            template,
            track,
            url,
        } => {
            let response = aidp_gateway::client::ticket_start(
                &url,
                &key,
                force,
                service.as_deref(),
                template.as_deref(),
                track.map(|arg| Track::from(arg).as_str()),
            )?;
            Ok(CommandOutput::text(serde_json::to_string_pretty(
                &response,
            )?))
        }
        Op::Adopt { key, pr, url } => {
            let response = aidp_gateway::client::adopt_ticket(&url, &key, &pr)?;
            let rendered = match fmt(ctx) {
                Format::JsonCompact | Format::Ndjson => response.to_string(),
                Format::JsonPretty | Format::Text => serde_json::to_string_pretty(&response)?,
            };
            Ok(CommandOutput::text(rendered))
        }
        Op::Dispatch { dry_run, url } => {
            let response = aidp_gateway::client::dispatcher_tick(&url, dry_run)?;
            Ok(CommandOutput::text(serde_json::to_string_pretty(
                &response,
            )?))
        }
        Op::Budget { url } => {
            let budget = aidp_gateway::client::dispatcher_budget(&url)?;
            let rendered = match fmt(ctx) {
                Format::JsonPretty => serde_json::to_string_pretty(&budget)?,
                Format::JsonCompact | Format::Ndjson => budget.to_string(),
                Format::Text => text_budget(&budget),
            };
            Ok(CommandOutput::text(rendered))
        }
        other => run_local(ctx, &loc, other),
    }
}

/// Serves the verbs that work on the local store, with no daemon.
fn run_local(ctx: &Ctx, loc: &TicketLoc, op: Op) -> anyhow::Result<CommandOutput> {
    let vault = ticket_vault_root_for(ctx, loc)?;
    let store = TicketStore::open(&vault)?;
    match op {
        Op::New {
            service,
            summary,
            issue_type,
            priority,
            body_file,
            body,
            stdin,
            labels,
            template,
            track,
            epic,
            allow_unknown_service,
        } => json_output(tickets::new_ticket(
            &store,
            NewTicket {
                service,
                summary,
                issue_type: issue_type.map(IssueType::from),
                priority: priority.map(Priority::from),
                body: BodySource::from_flags(body_file, body, stdin),
                labels,
                template,
                track: track.map(Track::from),
                epic,
                allow_unknown_service,
            },
        )),
        Op::List {
            status,
            service,
            source,
            archived,
            reason,
        } => text_output(tickets::list(
            &store,
            ListQuery {
                status: status.into_iter().map(Status::from).collect(),
                service,
                source: source.map(Source::from),
                archived,
                reason,
            },
            fmt(ctx),
        )),
        Op::Show { key } => text_output(tickets::show(&store, &key, fmt(ctx))),
        Op::Render { key } => {
            let body = tickets::render(&store, &key)?;
            Ok(raw_body_output(body))
        }
        Op::Edit {
            key,
            priority,
            service,
            template,
            track,
            summary,
            labels,
            epic,
        } => json_output(tickets::edit(
            &store,
            &key,
            TicketEdit {
                priority: priority.map(Priority::from),
                service,
                template,
                track: track.map(Track::from),
                summary,
                labels,
                epic,
            },
        )),
        Op::Status {
            key,
            status,
            reason,
            force,
        } => json_output(tickets::status(
            &store,
            &key,
            Status::from(status),
            reason,
            force,
        )),
        Op::Route {
            key,
            service,
            template,
            from_unrouted,
        } => json_output(tickets::route(
            &store,
            key,
            service,
            template,
            from_unrouted,
        )),
        Op::Cancel { key, reason } => json_output(tickets::cancel(&store, &key, reason)),
        Op::Log { key, limit } => text_output(tickets::log(&store, key, limit, fmt(ctx))),
        Op::Reindex => json_output(tickets::reindex(&store)),
        Op::Plan => {
            let plan = tickets::plan(&store, ctx.explicit_skill_root(), fmt(ctx))?;
            Ok(text_or_empty(plan.text).with_diagnostics(plan.warnings))
        }
        Op::Sync {
            repo,
            limit,
            dry_run,
            ..
        } => super::ticket_github::sync(
            &store,
            super::ticket_github::SyncArgs {
                repo,
                limit,
                dry_run,
            },
        ),
        Op::Label { key, state } => super::ticket_github::label(&store, &key, &state),
        Op::Close { key } => super::ticket_github::close(&store, &key),
        Op::Comment {
            key,
            body_file,
            body,
            cost_footer,
        } => super::ticket_github::comment(
            &store,
            &key,
            body_file.as_deref(),
            body.as_deref(),
            cost_footer,
        ),
        Op::Start { .. } | Op::Adopt { .. } | Op::Dispatch { .. } | Op::Budget { .. } => {
            unreachable!("daemon verbs are served before the local store is opened")
        }
    }
}

/// The output format, defaulting to compact JSON.
fn fmt(ctx: &Ctx) -> Format {
    ctx.format().unwrap_or(Format::JsonCompact)
}

/// One compact JSON object followed by a newline.
fn json_output(value: anyhow::Result<serde_json::Value>) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput::text(value?.to_string()))
}

/// Rendered text; an empty rendering prints nothing at all.
fn text_output(text: anyhow::Result<String>) -> anyhow::Result<CommandOutput> {
    Ok(text_or_empty(text?))
}

fn text_or_empty(text: String) -> CommandOutput {
    if text.is_empty() {
        CommandOutput::empty()
    } else {
        CommandOutput::text(text)
    }
}

/// A ticket body printed verbatim.
/// The payload printer appends one newline, so one trailing newline is dropped.
fn raw_body_output(body: String) -> CommandOutput {
    match body.strip_suffix('\n') {
        Some(rest) => CommandOutput::text(rest),
        None => text_or_empty(body),
    }
}

/// Resolve the vault root for `aidp ticket …`.
fn ticket_vault_root_for(ctx: &Ctx, loc: &TicketLoc) -> anyhow::Result<PathBuf> {
    let workspace = ctx.cwd_or(loc.workspace_root.clone());
    match aidp_core::vault::resolve(loc.vault.as_deref(), &workspace) {
        Some(vault) => Ok(vault.root),
        None => anyhow::bail!(
            "no vault resolved for `aidp ticket`: pass --vault <path>, set AIDP_VAULT, or run `aidp vault init` — local tickets are never written into .aidp/"
        ),
    }
}

/// Budget guard states reported by the dispatcher, serialized in lowercase.
#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum VerdictState {
    Allow,
    Conservative,
    Wait,
    Paused,
    #[serde(other)]
    Unrecognized,
}

impl VerdictState {
    const fn label(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Conservative => "conservative",
            Self::Wait => "wait",
            Self::Paused => "paused",
            Self::Unrecognized => NO_VERDICT_LABEL,
        }
    }
}

/// The flat verdict object of `GET /api/dispatcher/budget`.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct BudgetVerdict {
    verdict: Option<VerdictState>,
    max_launches: u64,
    why: String,
    utilization_5h: f64,
    utilization_7d: f64,
    limit_5h: f64,
    limit_7d: f64,
    reserve_5h: f64,
    reserve_7d: f64,
    unreflected: u64,
    resets_at_5h: Option<String>,
}

/// Wrapper of the budget response, of which only the verdict is rendered.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct BudgetResponse {
    verdict: BudgetVerdict,
}

fn percent(ratio: f64) -> String {
    format!("{:.0} %", ratio * 100.0)
}

fn reserve_label(value: f64) -> String {
    format!("{value:.2}").replace('.', ",")
}

/// `--format text` rendering of the budget response, one line per window plus a verdict line.
fn text_budget(budget: &serde_json::Value) -> String {
    let response: BudgetResponse = serde_json::from_value(budget.clone()).unwrap_or_default();
    let verdict = response.verdict;
    let reset_5h = verdict
        .resets_at_5h
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| NO_RESET_LABEL.to_string());
    let why = verdict.why.as_str();

    let mut s = format!(
        "5 h : {} / seuil {} · réserve {} · reset {}\n7 j : {} / seuil {} · réserve {}\nVerdict : {} · {} lancement · sessions non reflétées : {}",
        percent(verdict.utilization_5h),
        percent(verdict.limit_5h),
        reserve_label(verdict.reserve_5h),
        reset_5h,
        percent(verdict.utilization_7d),
        percent(verdict.limit_7d),
        reserve_label(verdict.reserve_7d),
        verdict.verdict.map_or(NO_VERDICT_LABEL, VerdictState::label),
        verdict.max_launches,
        verdict.unreflected,
    );
    if !why.is_empty() {
        s.push_str(&format!(" · {why}"));
    }
    s
}
