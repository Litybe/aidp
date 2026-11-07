//! Typed entry points for every local `aidp ticket` verb.
//! Each takes an open [`TicketStore`] and returns the value the verb reports.

use crate::index;
use crate::ledger::{self, Event};
use crate::model::{self, Actor, Autopilot, IssueType, Priority, Source, Status, Ticket};
use crate::models::constants::cli::{
    ARCHIVED_FIELD, BODY_FIELD, CSV_SEPARATOR, DEFAULT_CANCEL_REASON, ERRORS_FIELD, FROM_FIELD,
    FROM_UNROUTED_CONFLICT, KEYS_FIELD, KEY_FIELD, LEFT_FIELD, NO_PREVIOUS_STATUS,
    NO_TEMPLATE_FLAG, PATH_FIELD, ROUTED_FIELD, ROUTE_ARGS_REQUIRED, TICKET_FIELD, TO_FIELD,
    UNROUTED_SERVICE, WARNINGS_FIELD,
};
use crate::models::enums::cli_kind::IssueTypeName;
use crate::models::track::Track;
use crate::note;
use crate::plan;
use crate::routing;
use crate::store::{ListFilter, TicketStore};
pub use aidp_core::render_value;
pub use aidp_core::Format;
pub use aidp_core::Format as Fmt;
use chrono::Utc;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Where a new ticket's body comes from, in precedence order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodySource {
    /// Read the body from this file.
    File(PathBuf),
    /// Use this text as the body.
    Inline(String),
    /// Read the body from standard input.
    Stdin,
    /// Generate the default body from the key and the summary.
    Template,
}

impl BodySource {
    /// Picks the source with the CLI precedence: file, then inline, then stdin.
    pub fn from_flags(file: Option<PathBuf>, inline: Option<String>, stdin: bool) -> Self {
        match (file, inline, stdin) {
            (Some(path), _, _) => Self::File(path),
            (None, Some(text), _) => Self::Inline(text),
            (None, None, true) => Self::Stdin,
            (None, None, false) => Self::Template,
        }
    }
}

/// The parameters of a new local ticket.
#[derive(Clone, Debug)]
pub struct NewTicket {
    pub service: String,
    pub summary: String,
    pub issue_type: Option<IssueType>,
    pub priority: Option<Priority>,
    pub body: BodySource,
    pub labels: Option<String>,
    pub template: Option<String>,
    pub track: Option<Track>,
    pub epic: Option<String>,
    pub allow_unknown_service: bool,
}

/// The frontmatter fields `edit` may change; the body is never touched.
#[derive(Clone, Debug, Default)]
pub struct TicketEdit {
    pub priority: Option<Priority>,
    pub service: Option<String>,
    pub template: Option<String>,
    pub track: Option<Track>,
    pub summary: Option<String>,
    pub labels: Option<String>,
    pub epic: Option<String>,
}

/// The parameters of a `list` query.
#[derive(Clone, Debug, Default)]
pub struct ListQuery {
    pub status: Vec<Status>,
    pub service: Option<String>,
    pub source: Option<Source>,
    pub archived: bool,
    pub reason: Option<String>,
}

/// A rendered `plan`: the stdout text plus the warnings to show on stderr.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanOutput {
    pub text: String,
    pub warnings: Vec<String>,
}

/// Creates a local ticket and returns `{key, path, warnings}`.
pub fn new_ticket(store: &TicketStore, params: NewTicket) -> anyhow::Result<serde_json::Value> {
    let NewTicket {
        service,
        summary,
        issue_type,
        priority,
        body,
        labels,
        template,
        track,
        epic,
        allow_unknown_service,
    } = params;
    TicketStore::check_service(store.vault_root(), &service, allow_unknown_service)?;

    let issue_type = issue_type.unwrap_or(IssueType::Task);
    let priority = priority.unwrap_or(Priority::Medium);

    let key = store.allocate_key()?;

    let template = template.or_else(|| store.routing().template_for(&service).map(str::to_string));

    let raw_body = read_body(&body, &key, &summary)?;
    let body = note::normalise_body(&raw_body);

    let now = Utc::now();
    let mut ticket = Ticket {
        key: key.clone(),
        source: Source::Local,
        r#type: issue_type,
        summary,
        status: Status::Todo,
        priority,
        priority_rank: 0,
        service,
        template,
        track,
        labels: labels.map(|s| split_csv(&s)).unwrap_or_default(),
        components: Vec::new(),
        epic,
        assignee: None,
        created: now,
        updated: now,
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot: Autopilot::default(),
        tags: Vec::new(),
    };

    let (path, report) = store.save_reporting(&mut ticket, &body, &serde_yaml::Mapping::new())?;

    ledger::append(
        store.vault_root(),
        &Event {
            ts: Utc::now(),
            key: key.clone(),
            from: None,
            to: Status::Todo,
            actor: Actor::Cli,
            reason: None,
            session_id: None,
        },
    );

    Ok(serde_json::json!({
        KEY_FIELD: key,
        PATH_FIELD: path.display().to_string(),
        WARNINGS_FIELD: report.warnings(),
    }))
}

/// Resolves a [`BodySource`] to raw text.
fn read_body(source: &BodySource, key: &str, summary: &str) -> anyhow::Result<String> {
    match source {
        BodySource::File(path) => std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read --body-file {}: {e}", path.display())),
        BodySource::Inline(text) => Ok(text.clone()),
        BodySource::Stdin => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| anyhow::anyhow!("failed to read --stdin: {e}"))?;
            Ok(text)
        }
        BodySource::Template => Ok(note::default_body(key, summary)),
    }
}

/// Lists tickets in dequeue order, rendered in `fmt`.
/// An empty `Text` listing renders as the empty string.
pub fn list(store: &TicketStore, query: ListQuery, fmt: Format) -> anyhow::Result<String> {
    let ListQuery {
        status,
        service,
        source,
        archived,
        reason,
    } = query;
    let filter = ListFilter {
        status,
        service,
        source,
        archived,
    };
    let mut tickets = store.list(&filter);
    if let Some(want) = &reason {
        tickets.retain(|t| t.autopilot.blocked_reason.as_deref() == Some(want.as_str()));
    }

    Ok(match fmt {
        Format::Text => tickets
            .iter()
            .map(list_text_line)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => render_value(fmt, &serde_json::to_value(&tickets)?)?,
    })
}

/// One `list --format text` line: key, status, priority, service, summary.
/// A `[no-template]` flag is appended when `template` is unset.
fn list_text_line(t: &Ticket) -> String {
    let flag = if t.template.is_none() {
        NO_TEMPLATE_FLAG
    } else {
        ""
    };
    format!(
        "{}\t{}\t{}\t{}\t{}{}",
        t.key,
        t.status.as_str(),
        t.priority.as_str(),
        t.service,
        t.summary,
        flag
    )
}

/// Shows one ticket: frontmatter and body, rendered in `fmt`.
pub fn show(store: &TicketStore, key: &str, fmt: Format) -> anyhow::Result<String> {
    let (ticket, body, extra) = store.get(key)?;
    Ok(match fmt {
        Format::Text => note::render(&ticket, &body, &extra),
        Format::JsonPretty => serde_json::to_string_pretty(
            &serde_json::json!({ TICKET_FIELD: ticket, BODY_FIELD: body }),
        )?,
        Format::JsonCompact | Format::Ndjson => {
            serde_json::json!({ TICKET_FIELD: ticket, BODY_FIELD: body }).to_string()
        }
    })
}

/// Returns a ticket's body exactly as stored: no frontmatter, no heading.
pub fn render(store: &TicketStore, key: &str) -> anyhow::Result<String> {
    let (_ticket, body, _extra) = store.get(key)?;
    Ok(body)
}

/// Edits a ticket's frontmatter and returns `{key, path}`.
/// A `service` delegates to the routing move first.
pub fn edit(
    store: &TicketStore,
    key: &str,
    changes: TicketEdit,
) -> anyhow::Result<serde_json::Value> {
    let TicketEdit {
        priority,
        service,
        template,
        track,
        summary,
        labels,
        epic,
    } = changes;
    if let Some(new_service) = service.as_deref() {
        route_ticket(store, key, new_service, template.as_deref())?;
    }

    let (mut ticket, body, extra) = store.get(key)?;
    if let Some(p) = priority {
        ticket.priority = p;
    }
    if service.is_none() {
        if let Some(t) = &template {
            ticket.template = Some(t.clone());
        }
    }
    if let Some(tr) = track {
        ticket.track = Some(tr);
    }
    if let Some(s) = summary {
        ticket.summary = s;
    }
    if let Some(l) = &labels {
        ticket.labels = split_csv(l);
    }
    if let Some(e) = epic {
        ticket.epic = Some(e);
    }

    let path = store.save(&mut ticket, &body, &extra)?;
    Ok(serde_json::json!({ KEY_FIELD: ticket.key, PATH_FIELD: path.display().to_string() }))
}

/// Shared by `route` and `edit`: check the service and move the note.
/// Saves once more only when a template is also given.
fn route_ticket(
    store: &TicketStore,
    key: &str,
    service: &str,
    template: Option<&str>,
) -> anyhow::Result<PathBuf> {
    TicketStore::check_service(store.vault_root(), service, false)?;
    let path = store.move_to_service(key, service)?;
    match template {
        Some(t) => {
            let (mut ticket, body, extra) = store.get(key)?;
            ticket.template = Some(t.to_string());
            store.save(&mut ticket, &body, &extra)
        }
        None => Ok(path),
    }
}

/// The two shapes `aidp ticket route` can take, decided by [`route_mode`].
#[derive(Debug)]
enum RouteMode {
    /// Single-ticket behaviour: `route KEY --service X`.
    One { key: String, service: String },
    /// `route --from-unrouted`: re-route the whole `unrouted/` backlog.
    FromUnrouted,
}

/// Validate the combination of `route`'s flags.
/// A pure function because the argument parser cannot express the XOR.
fn route_mode(
    key: Option<String>,
    service: Option<String>,
    from_unrouted: bool,
) -> anyhow::Result<RouteMode> {
    if from_unrouted {
        if key.is_some() || service.is_some() {
            anyhow::bail!(FROM_UNROUTED_CONFLICT);
        }
        return Ok(RouteMode::FromUnrouted);
    }
    match (key, service) {
        (Some(key), Some(service)) => Ok(RouteMode::One { key, service }),
        _ => anyhow::bail!(ROUTE_ARGS_REQUIRED),
    }
}

/// Routes one ticket (`key` and `service`) or the whole `unrouted/` backlog.
/// Returns `{key, path}` or `{routed, left, keys, errors}` accordingly.
pub fn route(
    store: &TicketStore,
    key: Option<String>,
    service: Option<String>,
    template: Option<String>,
    from_unrouted: bool,
) -> anyhow::Result<serde_json::Value> {
    match route_mode(key, service, from_unrouted)? {
        RouteMode::One { key, service } => {
            let path = route_ticket(store, &key, &service, template.as_deref())?;
            Ok(serde_json::json!({ KEY_FIELD: key, PATH_FIELD: path.display().to_string() }))
        }
        RouteMode::FromUnrouted => route_from_unrouted(store),
    }
}

/// `route --from-unrouted`: re-route every `unrouted` ticket that now matches a rule.
/// Failures leave the ticket in place and count in `left`; never aborts the batch.
fn route_from_unrouted(store: &TicketStore) -> anyhow::Result<serde_json::Value> {
    let tickets = store.list(&ListFilter {
        service: Some(UNROUTED_SERVICE.to_string()),
        ..ListFilter::default()
    });

    let mut routed_keys: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut left = 0usize;

    for ticket in tickets {
        let facts = routing::RouteFacts {
            key: ticket.key.clone(),
            summary: ticket.summary.clone(),
            issuetype: Some(IssueTypeName(ticket.r#type).as_str().to_string()),
            labels: ticket.labels.clone(),
            components: ticket.components.clone(),
            epic: ticket.epic.clone(),
            body: String::new(),
        };

        let Some(route) = routing::route(&facts, store.routing()) else {
            left += 1;
            continue;
        };

        if let Err(e) = TicketStore::check_service(store.vault_root(), &route.service, false) {
            errors.push(format!("{}: {e}", ticket.key));
            left += 1;
            continue;
        }

        match apply_unrouted_route(store, &ticket.key, &route) {
            Ok(()) => routed_keys.push(ticket.key.clone()),
            Err(e) => {
                errors.push(format!("{}: {e}", ticket.key));
                left += 1;
            }
        }
    }

    Ok(serde_json::json!({
        ROUTED_FIELD: routed_keys.len(),
        LEFT_FIELD: left,
        KEYS_FIELD: routed_keys,
        ERRORS_FIELD: errors,
    }))
}

/// Move one `unrouted/` ticket to `route.service`, set `track` and clear the blocked reason.
/// Then transition `Blocked -> Todo` and journal it as [`transition`] does.
fn apply_unrouted_route(
    store: &TicketStore,
    key: &str,
    route: &routing::Route,
) -> anyhow::Result<()> {
    route_ticket(store, key, &route.service, route.template.as_deref())?;

    let (mut ticket, body, extra) = store.get(key)?;
    let from = ticket.status;
    model::guard(from, Status::Todo, Actor::Cli, false)?;

    ticket.track = route.track;
    ticket.autopilot.blocked_reason = None;
    ticket.status = Status::Todo;

    store.save(&mut ticket, &body, &extra)?;

    ledger::append(
        store.vault_root(),
        &Event {
            ts: Utc::now(),
            key: key.to_string(),
            from: Some(from),
            to: Status::Todo,
            actor: Actor::Cli,
            reason: None,
            session_id: None,
        },
    );

    Ok(())
}

/// Manual status transition by a human, journalled. Returns `{key, from, to, path}`.
pub fn status(
    store: &TicketStore,
    key: &str,
    to: Status,
    reason: Option<String>,
    force: bool,
) -> anyhow::Result<serde_json::Value> {
    transition(store, key, to, reason, Actor::Human, force)
}

/// `cancel` is `status <KEY> cancelled` as a human, and always forces the transition.
/// A deliberate terminal action, so it exposes no force option of its own.
pub fn cancel(
    store: &TicketStore,
    key: &str,
    reason: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    let reason = reason.unwrap_or_else(|| DEFAULT_CANCEL_REASON.to_string());
    transition(
        store,
        key,
        Status::Cancelled,
        Some(reason),
        Actor::Human,
        true,
    )
}

/// Guard, apply and journal one status transition.
/// A refusal leaves the ticket and the journal untouched.
fn transition(
    store: &TicketStore,
    key: &str,
    to: Status,
    reason: Option<String>,
    actor: Actor,
    force: bool,
) -> anyhow::Result<serde_json::Value> {
    let (mut ticket, body, extra) = store.get(key)?;
    let from = ticket.status;
    model::guard(from, to, actor, force)?;

    ticket.status = to;
    let path = store.save(&mut ticket, &body, &extra)?;

    ledger::append(
        store.vault_root(),
        &Event {
            ts: Utc::now(),
            key: key.to_string(),
            from: Some(from),
            to,
            actor,
            reason,
            session_id: None,
        },
    );

    Ok(serde_json::json!({
        KEY_FIELD: key,
        FROM_FIELD: from.as_str(),
        TO_FIELD: to.as_str(),
        PATH_FIELD: path.display().to_string(),
    }))
}

/// Reads the transition journal, most recent last, rendered in `fmt`.
/// An empty `Text` journal renders as the empty string.
pub fn log(
    store: &TicketStore,
    key: Option<String>,
    limit: usize,
    fmt: Format,
) -> anyhow::Result<String> {
    let events = ledger::read(store.vault_root(), key.as_deref(), limit);
    Ok(match fmt {
        Format::Text => events
            .iter()
            .map(log_text_line)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => render_value(fmt, &serde_json::to_value(&events)?)?,
    })
}

/// One `log --format text` line.
fn log_text_line(e: &Event) -> String {
    let from = e.from.map(Status::as_str).unwrap_or(NO_PREVIOUS_STATUS);
    let reason = e
        .reason
        .as_deref()
        .map(|r| format!(" reason={r}"))
        .unwrap_or_default();
    format!(
        "{} {} {} -> {} ({}){}",
        e.ts.to_rfc3339(),
        e.key,
        from,
        e.to.as_str(),
        e.actor.as_str(),
        reason
    )
}

/// Lists every note once, archives old done tickets, then refreshes the index once.
/// Returns `{archived, warnings}`; an unwritable index never fails it.
pub fn reindex(store: &TicketStore) -> anyhow::Result<serde_json::Value> {
    let entries = store.list_with_paths(&ListFilter {
        archived: true,
        ..ListFilter::default()
    });
    let archived = store.archive_entries(&entries)?;
    let tickets: Vec<Ticket> = entries.into_iter().map(|(ticket, _)| ticket).collect();
    let report = index::refresh(store.vault_root(), &tickets);
    Ok(serde_json::json!({ ARCHIVED_FIELD: archived, WARNINGS_FIELD: report.warnings() }))
}

/// The local, no-daemon preview of [`plan::plan`], rendered in `fmt`, with no running sessions.
/// A missing or unreadable skill root degrades to an empty catalog plus a warning.
pub fn plan(
    store: &TicketStore,
    skill_root: Option<&Path>,
    fmt: Format,
) -> anyhow::Result<PlanOutput> {
    let filter = ListFilter {
        status: vec![Status::Todo, Status::Queued],
        ..ListFilter::default()
    };
    let candidates = store.list(&filter);

    let (catalog, warnings) = template_catalog(skill_root);

    let dispatch_plan = plan::plan(
        &candidates,
        &[],
        &plan::Verdict::unconstrained(),
        &catalog,
        &store.config().dispatcher,
    );

    let text = match fmt {
        Format::JsonCompact | Format::Ndjson => serde_json::to_string(&dispatch_plan)?,
        Format::JsonPretty => serde_json::to_string_pretty(&dispatch_plan)?,
        Format::Text => plan_text(&dispatch_plan),
    };
    Ok(PlanOutput { text, warnings })
}

/// Loads the template catalog from `skill_root`, or an empty one with a warning.
fn template_catalog(skill_root: Option<&Path>) -> (plan::TemplateCatalog, Vec<String>) {
    match skill_root {
        Some(root) if root.is_dir() => (plan::TemplateCatalog::from_skill_root(root), Vec::new()),
        Some(root) => (
            plan::TemplateCatalog::empty(),
            vec![
                crate::models::constants::cli::skill_root_unreadable_warning(
                    &root.display().to_string(),
                ),
            ],
        ),
        None => (
            plan::TemplateCatalog::empty(),
            vec![crate::models::constants::cli::SKILL_ROOT_UNSET_WARNING.to_string()],
        ),
    }
}

/// The `plan --format text` lines: launches first, then skipped tickets.
fn plan_text(dispatch_plan: &plan::DispatchPlan) -> String {
    let launches = dispatch_plan.launches.iter().map(|l| {
        format!(
            "{}\t{}\t{}\t{}",
            l.key,
            l.service,
            l.template,
            demand_text(&l.demand)
        )
    });
    let skipped = dispatch_plan
        .skipped
        .iter()
        .map(|(key, reason)| format!("{}\t{}", key, reason.as_str()));
    launches.chain(skipped).collect::<Vec<_>>().join("\n")
}

/// One-word-plus-detail rendering of a [`plan::Demand`] for `plan --format text`.
fn demand_text(d: &plan::Demand) -> String {
    match d {
        plan::Demand::Issue { number } => format!("issue:{number}"),
        plan::Demand::Prompt { feature, .. } => format!("prompt:{feature}"),
    }
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(CSV_SEPARATOR)
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/cli_port_tests.rs"]
mod tests;
