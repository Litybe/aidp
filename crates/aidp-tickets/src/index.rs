//! Generated board and per-service index rendering (`TODO/_TODO.md`,
//! `TODO/<service>/_<service>.md`) — vision §5 and the lot plan's "Index
//! générés" section of
//! `docs/plans/ticket-autopilot/01-lot-7a-jira-local.md` (historical name).
//!
//! **This is the ONLY module in the whole crate allowed to write a file whose
//! name starts with `_`.** That is an acceptance criterion of the lot (vision
//! §2 principle 5: "jamais de fichier `_*.md` écrit à la main" — generated
//! indexes are `aidp`'s job alone, and inside `aidp` that job belongs to this
//! module alone). [`refresh`] is the sole entry point `store.rs` calls (from
//! `save`, `move_to_service`, and the future `reindex` verb); this module
//! never calls back into `store` — the dependency is strictly one-way,
//! `store -> index`.

use crate::model::{Status, Ticket};
use crate::order;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// `<vault>/TODO`. A local literal, deliberately duplicated from `store.rs`'s
/// own `TODO_DIR` (itself duplicated from `aidp_core::vault`'s dashboard
/// link, see that module's comment): this crate must not depend on
/// `aidp-core`, which owns the real vault-layout constants, so the two
/// sibling modules of this one crate each carry their own copy rather than
/// import one from the other or from a dependency this crate is forbidden to
/// take.
const TODO_DIR: &str = "TODO";

/// `<vault>/TODO/.state` — where the (lot 9a) dispatcher will write
/// `dispatcher.json`.
const STATE_DIR: &str = ".state";

/// File name of the whole-vault board under `TODO/`.
const BOARD_FILE: &str = "_TODO.md";

/// One generated index file [`refresh`] could not write.
#[derive(Debug)]
pub struct FailedWrite {
    pub path: PathBuf,
    pub error: String,
}

/// What [`refresh`] did with each generated index file, sorted by outcome.
#[derive(Debug, Default)]
pub struct RefreshReport {
    pub written: Vec<PathBuf>,
    pub unchanged: Vec<PathBuf>,
    pub failed: Vec<FailedWrite>,
}

impl RefreshReport {
    /// One `could not write <path>: <error>` line per failed write, the path
    /// rendered with forward slashes so the text is identical on every OS.
    pub fn warnings(&self) -> Vec<String> {
        self.failed
            .iter()
            .map(|failure| {
                format!(
                    "could not write {}: {}",
                    forward_slashes(&failure.path),
                    failure.error
                )
            })
            .collect()
    }

    fn record(&mut self, path: PathBuf, outcome: std::io::Result<bool>) {
        match outcome {
            Ok(true) => self.written.push(path),
            Ok(false) => self.unchanged.push(path),
            Err(error) => {
                tracing::warn!("failed to write {}: {error}", path.display());
                self.failed.push(FailedWrite {
                    path,
                    error: error.to_string(),
                });
            }
        }
    }
}

fn forward_slashes(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Regenerate `TODO/_TODO.md`, then one `TODO/<name>/_<name>.md` per
/// non-hidden directory under `TODO/` on disk (`unrouted/` and fully archived
/// services included). `tickets` is exactly the set to render: this never
/// re-scans notes nor calls into `store`. Every outcome lands in the returned
/// report, and each failed write also prints one stderr line.
pub fn refresh(vault_root: &Path, tickets: &[Ticket]) -> RefreshReport {
    let todo_dir = vault_root.join(TODO_DIR);
    let mut report = RefreshReport::default();

    let board = render_board(tickets, vault_root);
    let board_path = todo_dir.join(BOARD_FILE);
    let board_outcome = write_if_changed(&board_path, &board);
    report.record(board_path, board_outcome);

    let Ok(entries) = fs::read_dir(&todo_dir) else {
        return report;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }
        let doc = render_service_index(name, tickets, vault_root);
        let index_path = path.join(format!("_{name}.md"));
        let index_outcome = write_if_changed(&index_path, &doc);
        report.record(index_path, index_outcome);
    }
    report
}

/// Render `TODO/_TODO.md`: the whole-vault ticket board.
pub fn render_board(tickets: &[Ticket], vault_root: &Path) -> String {
    render(None, tickets, vault_root)
}

/// Render `TODO/<service>/_<service>.md`: the same board, restricted to one
/// service.
pub fn render_service_index(service: &str, tickets: &[Ticket], vault_root: &Path) -> String {
    render(Some(service), tickets, vault_root)
}

fn render(scope: Option<&str>, tickets: &[Ticket], vault_root: &Path) -> String {
    let scoped: Vec<&Ticket> = match scope {
        Some(svc) => tickets.iter().filter(|t| t.service == svc).collect(),
        None => tickets.iter().collect(),
    };
    // Derived from the tickets themselves (the most recent `updated` among
    // them), not `Utc::now()`: `refresh` only ever runs right after a ticket
    // was actually mutated, so this is essentially "now" in real use, and
    // unlike a live clock read it makes this whole function a pure,
    // deterministic mapping from `tickets` to text — the property
    // `refresh_is_idempotent` (below) relies on, and the same property that
    // lets `write_if_changed` actually skip a rewrite when nothing changed
    // instead of ticking a fresh timestamp on every call regardless.
    let now = scoped
        .iter()
        .map(|t| t.updated)
        .max()
        .unwrap_or_else(Utc::now);

    let mut tags = vec!["aidp/ticket-board".to_string()];
    if let Some(svc) = scope {
        tags.push(format!("service/{svc}"));
    }

    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("aidp: ticket-board\n");
    if let Some(svc) = scope {
        out.push_str(&format!("service: {svc}\n"));
    }
    out.push_str(&format!("updated: {}\n", now.format("%Y-%m-%dT%H:%M:%S")));
    out.push_str(&format!("tags: [{}]\n", tags.join(", ")));
    out.push_str("---\n\n");

    match scope {
        Some(svc) => out.push_str(&format!("# Tickets — {svc}\n\n")),
        None => out.push_str("# Tickets\n\n"),
    }

    out.push_str("## Dispatcher\n\n");
    out.push_str(&dispatcher_section(vault_root, now));
    out.push('\n');

    out.push_str("## À faire\n\n");
    out.push_str(&dequeue_table(&scoped, Status::Todo, now, true));
    out.push('\n');

    out.push_str("## En cours\n\n");
    out.push_str(&dequeue_table(&scoped, Status::InProgress, now, false));
    out.push('\n');

    out.push_str("## En revue\n\n");
    out.push_str(&in_review_table(&scoped, now));
    out.push('\n');

    out.push_str("## Bloqués\n\n");
    out.push_str(&blocked_table(&scoped, now));
    out.push('\n');

    out.push_str("## Terminés (7 j)\n\n");
    out.push_str(&done_recent_table(&scoped, now));
    out.push('\n');

    out.push_str("## Par service\n\n");
    out.push_str(&per_service_table(&scoped));
    out.push('\n');

    out.push_str("## Légende\n\n");
    out.push_str(LEGEND);

    out
}

/// The pinned wire shape of `TODO/.state/dispatcher.json`, written by the
/// lot 9a daemon — not this crate's contract to change. Every field is
/// `#[serde(default)]` so a partial file (an in-progress write, an older or
/// newer daemon version) never drops the whole section; unknown extra
/// fields are silently ignored by serde's default behaviour.
///
/// This is deliberately a SEPARATE type from
/// [`crate::dispatcher_state::DispatcherState`] — the writer's canonical,
/// always-populated (non-tolerant) shape this struct diverges from on
/// purpose, for partial-write resilience: unifying the two would either
/// lose this struct's tolerance for a half-written or differently-versioned
/// file, or force the writer's always-present fields into `Option`
/// everywhere. `writer_shape_round_trips_through_the_tolerant_reader`
/// (below, in this module's tests) proves the two shapes still agree on
/// the wire.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DispatcherState {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    paused: bool,
    #[serde(default)]
    wait_until: Option<DateTime<Utc>>,
    #[serde(default)]
    last_tick_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_reason: Option<String>,
    #[serde(default)]
    slots: DispatcherSlots,
    #[serde(default)]
    running: Vec<DispatcherRunning>,
    #[serde(default)]
    last_plan: DispatcherLastPlan,
    /// Lot 10a's budget verdict. `#[serde(default)]` so a `dispatcher.json`
    /// written before this lot (no `"budget"` key at all) still deserializes
    /// — see [`DispatcherBudget`]'s own doc comment for the empty-`verdict`
    /// fallback this enables in `render_dispatcher_state`.
    #[serde(default)]
    budget: DispatcherBudget,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DispatcherSlots {
    #[serde(default)]
    used: u32,
    #[serde(default)]
    max: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DispatcherRunning {
    #[serde(default)]
    key: String,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    project: String,
    #[serde(default)]
    started_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DispatcherLastPlan {
    #[serde(default)]
    launched: Vec<String>,
    #[serde(default)]
    skipped: Vec<DispatcherSkip>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DispatcherSkip {
    #[serde(default)]
    key: String,
    #[serde(default)]
    reason: String,
}

/// Tolerant mirror of `crate::dispatcher_state::BudgetState` — every field
/// `#[serde(default)]`, same rationale as the rest of this private
/// `DispatcherState` tree. `verdict` empty is the one value the writer can
/// never produce once it has ticked at least once (see `BudgetState`'s own
/// doc comment): `render_dispatcher_state` treats that empty string as "no
/// budget-aware tick has happened yet" and renders a fallback line instead
/// of a table of zeroes, which would otherwise read as a real measurement.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DispatcherBudget {
    #[serde(default)]
    verdict: String,
    #[serde(default)]
    max_launches: usize,
    #[serde(default)]
    why: String,
    #[serde(default)]
    utilization_5h: f64,
    #[serde(default)]
    utilization_7d: f64,
    #[serde(default)]
    limit_5h: f64,
    #[serde(default)]
    limit_7d: f64,
    #[serde(default)]
    reserve_5h: f64,
    #[serde(default)]
    reserve_7d: f64,
    #[serde(default)]
    unreflected: usize,
    #[serde(default)]
    resets_at_5h: Option<DateTime<Utc>>,
    /// Parsed for wire-shape parity with `BudgetState` (and covered by
    /// `writer_shape_round_trips_through_the_tolerant_reader`) but not yet
    /// rendered — the verdict line already says which window a `wait`
    /// verdict is on.
    #[serde(default)]
    #[allow(dead_code)]
    wait_window: Option<String>,
    /// Parsed for the same reason as `wait_window`; not yet rendered.
    #[serde(default)]
    #[allow(dead_code)]
    observed_at: Option<DateTime<Utc>>,
}

/// `TODO/.state/dispatcher.json` if it exists and parses: rendered as a
/// status line, an **En cours** table and a **Dernier plan** block, in the
/// same Markdown style as the rest of this file. Two distinct fallbacks,
/// never conflated:
/// - the file is absent -> a one-line placeholder (the dispatcher has
///   never ticked / is not running — lot 9a has shipped, so this no longer
///   promises a future lot);
/// - the file exists but does not parse as `DispatcherState` -> the raw
///   fenced dump (so a wire-shape change never silently blanks the board),
///   with a short warning line above it.
fn dispatcher_section(vault_root: &Path, now: DateTime<Utc>) -> String {
    let path = vault_root
        .join(TODO_DIR)
        .join(STATE_DIR)
        .join("dispatcher.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => {
            return "_Le dispatcher n'a jamais tourné (pas de `dispatcher.json`)._\n".to_string()
        }
    };
    match serde_json::from_str::<DispatcherState>(&text) {
        Ok(state) => render_dispatcher_state(&state, now),
        Err(_) => format!(
            "_`dispatcher.json` illisible (forme inattendue) — contenu brut :_\n\n```\n{}\n```\n",
            text.trim()
        ),
    }
}

/// Render a parsed [`DispatcherState`]: status line, **En cours** table,
/// **Dernier plan** block.
fn render_dispatcher_state(state: &DispatcherState, now: DateTime<Utc>) -> String {
    let mut s = String::new();

    let mode = if state.mode.is_empty() {
        "?"
    } else {
        state.mode.as_str()
    };
    let last_tick = state
        .last_tick_at
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "-".to_string());
    let last_reason = match state.last_reason.as_deref() {
        Some(r) if !r.is_empty() => r,
        _ => "-",
    };
    s.push_str(&format!(
        "**Mode** : `{mode}` · **Créneaux** : {}/{} · **Dernier tick** : {last_tick} · **Raison** : {}",
        state.slots.used,
        state.slots.max,
        inline(last_reason),
    ));
    if mode == "waiting" {
        if let Some(wait_until) = state.wait_until {
            s.push_str(&format!(
                " · **Attente jusqu'à** : {}",
                wait_until.to_rfc3339()
            ));
        }
    }
    if state.paused {
        s.push_str(" · ⏸ **en pause**");
    }
    s.push_str("\n\n");

    s.push_str("**Budget**\n\n");
    if state.budget.verdict.is_empty() {
        // Pre-lot-10a `dispatcher.json`, or a daemon that has never ticked
        // since restart: no budget snapshot exists yet. A table of zeroes
        // here would read as a real measurement, so render an honest blank
        // instead.
        s.push_str("_Budget inconnu (aucun tick budgétaire depuis le démarrage)._\n");
    } else {
        let reset_5h = state
            .budget
            .resets_at_5h
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "—".to_string());
        s.push_str("| Fenêtre | Utilisation | Seuil | Réserve | Reset |\n|---|---|---|---|---|\n");
        s.push_str(&format!(
            "| 5 h | {} | {} | {} | {} |\n",
            format_budget_pct(state.budget.utilization_5h),
            format_budget_pct(state.budget.limit_5h),
            format_budget_reserve(state.budget.reserve_5h),
            reset_5h,
        ));
        s.push_str(&format!(
            "| 7 j | {} | {} | {} | — |\n",
            format_budget_pct(state.budget.utilization_7d),
            format_budget_pct(state.budget.limit_7d),
            format_budget_reserve(state.budget.reserve_7d),
        ));
        s.push('\n');
        s.push_str(&format!(
            "Verdict : `{}` · {} lancement · sessions non reflétées : {}",
            inline(&state.budget.verdict),
            state.budget.max_launches,
            state.budget.unreflected,
        ));
        if !state.budget.why.is_empty() {
            s.push_str(&format!(" · {}", inline(&state.budget.why)));
        }
        s.push('\n');
    }
    s.push('\n');

    s.push_str("**En cours**\n\n");
    if state.running.is_empty() {
        s.push_str("_Aucune session en cours._\n");
    } else {
        s.push_str("| Ticket | Projet | Session | Depuis |\n|---|---|---|---|\n");
        for r in &state.running {
            let since = r
                .started_at
                .map(|dt| human_age(now, dt))
                .unwrap_or_else(|| "-".to_string());
            s.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                inline(&r.key),
                inline(&r.project),
                inline(&r.session_id),
                since,
            ));
        }
    }
    s.push('\n');

    s.push_str("**Dernier plan**\n\n");
    if state.last_plan.launched.is_empty() {
        s.push_str("_Aucun ticket lancé._\n\n");
    } else {
        let launched = state
            .last_plan
            .launched
            .iter()
            .map(|k| inline(k))
            .collect::<Vec<_>>()
            .join(", ");
        s.push_str(&format!("Lancés : {launched}\n\n"));
    }
    if state.last_plan.skipped.is_empty() {
        s.push_str("_Aucun ticket ignoré._\n");
    } else {
        s.push_str("| Ticket | Raison |\n|---|---|\n");
        for skip in &state.last_plan.skipped {
            s.push_str(&format!(
                "| {} | {} |\n",
                inline(&skip.key),
                inline(&skip.reason),
            ));
        }
    }

    s
}

/// Tickets in `status`, in dequeue order (`order::sort_key`). Used for both
/// **À faire** and **En cours**: key, source, priority, service, summary,
/// age (since `created`). `flag_missing_template` marks a `template: null`
/// ticket — only meaningful for **À faire**, since that is the set lot 9a's
/// dispatcher will actually try to launch next.
fn dequeue_table(
    tickets: &[&Ticket],
    status: Status,
    now: DateTime<Utc>,
    flag_missing_template: bool,
) -> String {
    let rows = sorted_by_dequeue_order(tickets, status);
    if rows.is_empty() {
        return "_Aucun ticket._\n".to_string();
    }
    let mut s = String::from(
        "| Clé | Source | Priorité | Service | Résumé | Âge |\n|---|---|---|---|---|---|\n",
    );
    for t in rows {
        let flag = if flag_missing_template && t.template.is_none() {
            " ⚠️ *(sans template)*"
        } else {
            ""
        };
        s.push_str(&format!(
            "| {}{flag} | {} | {} | {} | {} | {} |\n",
            ticket_link(t),
            t.source,
            t.priority.as_str(),
            // Unescaped like `ticket_link`'s wikilink above: safe only per the
            // `safe_name::check` invariant documented on `ticket_link`.
            t.service,
            inline(&t.summary),
            human_age(now, t.created),
        ));
    }
    s
}

/// **En revue**: with the PR link when `autopilot.pr` is set.
fn in_review_table(tickets: &[&Ticket], now: DateTime<Utc>) -> String {
    let rows = sorted_by_dequeue_order(tickets, Status::InReview);
    if rows.is_empty() {
        return "_Aucun ticket._\n".to_string();
    }
    let mut s =
        String::from("| Clé | Source | Service | Résumé | PR | Âge |\n|---|---|---|---|---|---|\n");
    for t in rows {
        let pr = match &t.autopilot.pr {
            Some(pr) => {
                let label = pr
                    .number
                    .map(|n| format!("#{n}"))
                    .unwrap_or_else(|| "PR".to_string());
                format!("[{label}]({})", pr.url)
            }
            None => "-".to_string(),
        };
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            ticket_link(t),
            t.source,
            t.service,
            inline(&t.summary),
            pr,
            human_age(now, t.updated),
        ));
    }
    s
}

/// **Bloqués**: with `autopilot.blocked_reason`.
fn blocked_table(tickets: &[&Ticket], now: DateTime<Utc>) -> String {
    let rows = sorted_by_dequeue_order(tickets, Status::Blocked);
    if rows.is_empty() {
        return "_Aucun ticket._\n".to_string();
    }
    let mut s = String::from(
        "| Clé | Source | Service | Résumé | Raison | Âge |\n|---|---|---|---|---|---|\n",
    );
    for t in rows {
        let reason = t.autopilot.blocked_reason.as_deref().unwrap_or("-");
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            ticket_link(t),
            t.source,
            t.service,
            inline(&t.summary),
            inline(reason),
            human_age(now, t.updated),
        ));
    }
    s
}

/// **Terminés (7 j)**: `done` tickets whose `updated` falls in the last 7
/// days.
fn done_recent_table(tickets: &[&Ticket], now: DateTime<Utc>) -> String {
    let cutoff = now - chrono::Duration::days(7);
    let mut rows: Vec<&Ticket> = tickets
        .iter()
        .copied()
        .filter(|t| t.status == Status::Done && t.updated >= cutoff)
        .collect();
    rows.sort_by_key(|t| order::sort_key(t));
    if rows.is_empty() {
        return "_Aucun ticket._\n".to_string();
    }
    let mut s = String::from(
        "| Clé | Source | Service | Résumé | Terminé il y a |\n|---|---|---|---|---|\n",
    );
    for t in rows {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            ticket_link(t),
            t.source,
            t.service,
            inline(&t.summary),
            human_age(now, t.updated),
        ));
    }
    s
}

/// One row per service seen among `tickets`, with a counter per non-terminal
/// (+`done`) status. `cancelled` is not counted — it is neither an active
/// nor a "just finished" state worth surfacing on the board.
fn per_service_table(tickets: &[&Ticket]) -> String {
    let mut counts: std::collections::BTreeMap<&str, [usize; 5]> =
        std::collections::BTreeMap::new();
    for t in tickets {
        let entry = counts.entry(t.service.as_str()).or_insert([0usize; 5]);
        match t.status {
            Status::Todo | Status::Queued => entry[0] += 1,
            Status::InProgress => entry[1] += 1,
            Status::InReview => entry[2] += 1,
            Status::Blocked => entry[3] += 1,
            Status::Done => entry[4] += 1,
            Status::Cancelled => {}
        }
    }
    if counts.is_empty() {
        return "_Aucun ticket._\n".to_string();
    }
    let mut s = String::from(
        "| Service | À faire | En cours | En revue | Bloqués | Terminés |\n|---|---|---|---|---|---|\n",
    );
    for (svc, c) in counts {
        s.push_str(&format!(
            "| {svc} | {} | {} | {} | {} | {} |\n",
            c[0], c[1], c[2], c[3], c[4]
        ));
    }
    s
}

const LEGEND: &str = "\
- `#aidp/ticket` — toute note ticket.\n\
- `#aidp/ticket/<status>` — par statut (`todo`, `queued`, `in-progress`, `in-review`, `done`, `blocked`, `cancelled`).\n\
- `#service/<service>` — par service.\n\
- `#source/<source>` — `github` ou `local`.\n";

fn sorted_by_dequeue_order<'a>(tickets: &[&'a Ticket], status: Status) -> Vec<&'a Ticket> {
    let mut rows: Vec<&Ticket> = tickets
        .iter()
        .copied()
        .filter(|t| t.status == status)
        .collect();
    rows.sort_by_key(|t| order::sort_key(t));
    rows
}

/// An Obsidian wikilink to a ticket note: `[[TODO/<service>/<KEY>|<KEY>]]` —
/// the vault-relative path without the `.md` suffix. A local one-line helper
/// rather than importing anything for it.
///
/// Unlike `summary`/`blocked_reason` above, `t.service` and `t.key` are
/// interpolated here with no `inline()` escaping. That is safe only because
/// `crate::safe_name::check` refuses any value containing `[`, `]`, `|`, `#`
/// or a backtick — the five characters that could close this wikilink early
/// or splice arbitrary Markdown into it — before a `Ticket`'s `key` or
/// `service` can ever reach this function. That check runs at every place a
/// `Ticket` is built from stored or hand-edited data: `note::parse`, and
/// `TicketStore::save`, `get`, `path_of`, `move_to_service` and
/// `archive_entries`. Loosening `safe_name::check` to accept any of those
/// five characters would reopen Markdown injection in the generated
/// `_TODO.md` and `_<service>.md`.
fn ticket_link(t: &Ticket) -> String {
    format!("[[{TODO_DIR}/{}/{}|{}]]", t.service, t.key, t.key)
}

/// Flatten to one line and escape table-breaking `|`: a summary or blocked
/// reason dropped verbatim into a table cell would otherwise corrupt the
/// row (or, for a stray newline, the whole table).
fn inline(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
        .replace('|', "\\|")
        .trim()
        .to_string()
}

/// A budget fraction (`0.87`) as a whole-percent string (`"87 %"`).
fn format_budget_pct(fraction: f64) -> String {
    format!("{:.0} %", (fraction * 100.0).round())
}

/// A budget reserve as a two-decimal, comma-separated string (`"0,10"`),
/// matching this vault's French Markdown throughout the rest of the module.
fn format_budget_reserve(reserve: f64) -> String {
    format!("{reserve:.2}").replace('.', ",")
}

/// Coarse, human age: minutes under an hour, hours under a day, otherwise
/// days. Display-only — nothing in this crate parses it back.
fn human_age(now: DateTime<Utc>, since: DateTime<Utc>) -> String {
    let secs = now.signed_duration_since(since).num_seconds().max(0);
    if secs < 3600 {
        format!("{}m", (secs / 60).max(if secs > 0 { 1 } else { 0 }))
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}j", secs / 86_400)
    }
}

/// Write only on a real change. Copied (not imported — it is private there)
/// from `write_if_changed` in `aidp-core/src/obsidian.rs`: rewriting
/// identical bytes on every `aidp` command that merely reads a ticket would
/// make Obsidian Sync re-upload (and the file-recovery plugin snapshot)
/// every generated note each time.
/// Returns `false` when the bytes were already there, `true` once written.
fn write_if_changed(path: &Path, content: &str) -> std::io::Result<bool> {
    if fs::read_to_string(path)
        .map(|old| old == content)
        .unwrap_or(false)
    {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(true)
}

#[cfg(test)]
#[path = "../tests/unit/index_tests.rs"]
mod index_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Autopilot, IssueType, Priority, Source};
    use aidp_testkit::TempWorkspace;
    use std::path::PathBuf;

    fn ticket(key: &str, source: Source, priority: Priority, created: DateTime<Utc>) -> Ticket {
        Ticket {
            key: key.to_string(),
            source,
            r#type: IssueType::Task,
            summary: format!("Summary for {key}"),
            status: Status::Todo,
            priority,
            priority_rank: priority.rank(),
            service: "svc".to_string(),
            template: None,
            track: None,
            labels: Vec::new(),
            components: Vec::new(),
            epic: None,
            assignee: None,
            created,
            updated: created,
            github: None,
            route: None,
            flow: None,
            milestone: None,
            autopilot: Autopilot::default(),
            tags: vec![
                "aidp/ticket".to_string(),
                "aidp/ticket/todo".to_string(),
                "service/svc".to_string(),
                format!("source/{}", source.as_str()),
            ],
        }
    }

    #[test]
    fn board_lists_tickets_in_dequeue_order() {
        let _workspace = TempWorkspace::new("order");
        let vault = _workspace.path();
        let base = Utc::now();
        let tickets = vec![
            ticket("LOC-1", Source::Local, Priority::Medium, base),
            ticket("GH-1", Source::Github, Priority::Medium, base),
            ticket(
                "GH-2",
                Source::Github,
                Priority::Highest,
                base + chrono::Duration::seconds(5),
            ),
        ];

        let board = render_board(&tickets, vault);

        let idx_2 = board.find("GH-2").expect("GH-2 listed");
        let idx_1 = board.find("GH-1").expect("GH-1 listed");
        let idx_loc = board.find("LOC-1").expect("LOC-1 listed");
        assert!(idx_2 < idx_1, "Highest priority must sort before Medium");
        assert!(
            idx_1 < idx_loc,
            "GitHub must sort before Local at equal priority"
        );
    }

    #[test]
    fn board_has_a_dispatcher_placeholder_when_no_state_file() {
        let _workspace = TempWorkspace::new("dispatcher-placeholder");
        let vault = _workspace.path();

        let board = render_board(&[], vault);

        assert!(
            board.contains("_Le dispatcher n'a jamais tourné (pas de `dispatcher.json`)._"),
            "missing dispatcher placeholder:\n{board}"
        );
    }

    #[test]
    fn dispatcher_section_renders_a_populated_state_file() {
        let _workspace = TempWorkspace::new("dispatcher-populated");
        let vault = _workspace.path();
        let state_dir = vault.join(TODO_DIR).join(STATE_DIR);
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(
            state_dir.join("dispatcher.json"),
            r#"{
              "mode": "normal",
              "paused": false,
              "waitUntil": null,
              "lastTickAt": "2026-09-14T09:08:09Z",
              "lastReason": "timer",
              "slots": { "used": 2, "max": 5 },
              "running": [
                { "key": "PIXEL-5677", "sessionId": "s-1", "project": "cheat-alerts-service", "startedAt": "2026-09-14T09:00:00Z" }
              ],
              "lastPlan": {
                "launched": ["PIXEL-5677"],
                "skipped": [
                  { "key": "LOC-2", "reason": "project-busy" },
                  { "key": "LOC-3", "reason": "no-slot" }
                ]
              }
            }"#,
        )
        .unwrap();

        let now: DateTime<Utc> = "2026-09-14T09:10:09Z".parse().unwrap();
        let section = dispatcher_section(vault, now);

        assert!(section.contains('`'), "mode should render, got:\n{section}");
        assert!(section.contains("normal"), "mode value missing:\n{section}");
        assert!(section.contains("2/5"), "slots line missing:\n{section}");
        assert!(section.contains("timer"), "last reason missing:\n{section}");
        assert!(
            section.contains("PIXEL-5677"),
            "running row missing:\n{section}"
        );
        assert!(
            section.contains("cheat-alerts-service"),
            "running project missing:\n{section}"
        );
        assert!(
            section.contains("s-1"),
            "running session missing:\n{section}"
        );
        assert!(
            section.contains("LOC-2") && section.contains("project-busy"),
            "first skip reason missing:\n{section}"
        );
        assert!(
            section.contains("LOC-3") && section.contains("no-slot"),
            "second skip reason missing:\n{section}"
        );
        assert!(
            !section.trim_start().starts_with("```"),
            "a parseable file must not fall back to the raw fenced dump:\n{section}"
        );

        let _ = fs::remove_dir_all(vault);
    }

    /// The test that would have caught the writer's shape
    /// (`crate::dispatcher_state::DispatcherState`, in `aidp-gateway`
    /// before this fix and now canonically here) and this module's own
    /// private, tolerant reader `DispatcherState` silently drifting apart
    /// (F-review__review_maintainability-4): serialize a fully populated
    /// writer value, then parse that exact JSON through the reader shape,
    /// and assert every field the reader carries matches.
    #[test]
    fn writer_shape_round_trips_through_the_tolerant_reader() {
        let started_at: DateTime<Utc> = "2026-09-14T09:00:00Z".parse().unwrap();
        let last_tick_at: DateTime<Utc> = "2026-09-14T09:08:09Z".parse().unwrap();
        let resets_at_5h: DateTime<Utc> = "2026-09-14T14:35:00Z".parse().unwrap();
        let observed_at: DateTime<Utc> = "2026-09-14T09:08:00Z".parse().unwrap();

        let written = crate::dispatcher_state::DispatcherState {
            mode: "normal".to_string(),
            paused: false,
            wait_until: None,
            last_tick_at: last_tick_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            last_reason: "timer".to_string(),
            slots: crate::dispatcher_state::SlotsState { used: 2, max: 5 },
            running: vec![crate::dispatcher_state::RunningEntry {
                key: "PIXEL-5677".to_string(),
                session_id: "s-1".to_string(),
                project: "cheat-alerts-service".to_string(),
                started_at: started_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                warnings: Vec::new(),
            }],
            last_plan: crate::dispatcher_state::LastPlan {
                launched: vec!["PIXEL-5677".to_string()],
                skipped: vec![crate::dispatcher_state::SkippedEntry {
                    key: "LOC-2".to_string(),
                    reason: crate::models::enums::SkipReason::ProjectBusy
                        .as_str()
                        .to_string(),
                    detail: None,
                }],
            },
            budget: crate::dispatcher_state::BudgetState {
                verdict: "wait".to_string(),
                max_launches: 0,
                why: "fenêtre 5 h à 87 % ≥ 80 %".to_string(),
                utilization_5h: 0.87,
                utilization_7d: 0.31,
                limit_5h: 0.80,
                limit_7d: 0.90,
                reserve_5h: 0.10,
                reserve_7d: 0.02,
                unreflected: 1,
                resets_at_5h: Some(resets_at_5h.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                wait_window: Some("5h".to_string()),
                observed_at: Some(observed_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            },
        };

        let json = serde_json::to_string(&written).expect("writer shape must serialize");
        let read: DispatcherState =
            serde_json::from_str(&json).expect("writer JSON must parse through the reader shape");

        assert_eq!(read.mode, written.mode);
        assert_eq!(read.paused, written.paused);
        assert_eq!(read.wait_until, None);
        assert_eq!(read.last_tick_at, Some(last_tick_at));
        assert_eq!(
            read.last_reason.as_deref(),
            Some(written.last_reason.as_str())
        );
        assert_eq!(read.slots.used, written.slots.used as u32);
        assert_eq!(read.slots.max, written.slots.max as u32);
        assert_eq!(read.running.len(), written.running.len());
        assert_eq!(read.running[0].key, written.running[0].key);
        assert_eq!(read.running[0].session_id, written.running[0].session_id);
        assert_eq!(read.running[0].project, written.running[0].project);
        assert_eq!(read.running[0].started_at, Some(started_at));
        assert_eq!(read.last_plan.launched, written.last_plan.launched);
        assert_eq!(read.last_plan.skipped.len(), 1);
        assert_eq!(read.last_plan.skipped[0].key, "LOC-2");
        assert_eq!(read.last_plan.skipped[0].reason, "project-busy");

        assert_eq!(read.budget.verdict, written.budget.verdict);
        assert_eq!(read.budget.max_launches, written.budget.max_launches);
        assert_eq!(read.budget.why, written.budget.why);
        assert_eq!(read.budget.utilization_5h, written.budget.utilization_5h);
        assert_eq!(read.budget.utilization_7d, written.budget.utilization_7d);
        assert_eq!(read.budget.limit_5h, written.budget.limit_5h);
        assert_eq!(read.budget.limit_7d, written.budget.limit_7d);
        assert_eq!(read.budget.reserve_5h, written.budget.reserve_5h);
        assert_eq!(read.budget.reserve_7d, written.budget.reserve_7d);
        assert_eq!(read.budget.unreflected, written.budget.unreflected);
        assert_eq!(read.budget.resets_at_5h, Some(resets_at_5h));
        assert_eq!(read.budget.wait_window, written.budget.wait_window);
        assert_eq!(read.budget.observed_at, Some(observed_at));
    }

    /// A `dispatcher.json` written before this lot has no `"budget"` key at
    /// all. It must still deserialize (the writer-shape test in
    /// `dispatcher_state.rs` proves the same for the writer's own type), and
    /// `render_dispatcher_state` must render the honest fallback line rather
    /// than a table of zeroes — the empty `verdict` string is exactly what
    /// distinguishes "never ticked" from "ticked, verdict happens to be
    /// zero-valued".
    #[test]
    fn dispatcher_state_without_budget_key_renders_budget_inconnu() {
        let json = serde_json::json!({
            "mode": "normal",
            "paused": false,
            "waitUntil": null,
            "lastTickAt": "2026-09-14T09:08:09Z",
            "lastReason": "timer",
            "slots": { "used": 2, "max": 5 },
            "running": [],
            "lastPlan": { "launched": [], "skipped": [] }
        });

        let state: DispatcherState = serde_json::from_value(json)
            .expect("a pre-lot-10a dispatcher.json (no budget key) must still deserialize");
        let section = render_dispatcher_state(&state, Utc::now());

        assert!(
            section.contains("_Budget inconnu (aucun tick budgétaire depuis le démarrage)._"),
            "missing budget-unknown fallback line:\n{section}"
        );
        assert!(
            !section.contains("| Fenêtre |"),
            "must not render a zero table when no budget tick has happened:\n{section}"
        );
    }

    #[test]
    fn dispatcher_section_falls_back_to_a_raw_dump_when_unparseable() {
        let _workspace = TempWorkspace::new("dispatcher-malformed");
        let vault = _workspace.path();
        let state_dir = vault.join(TODO_DIR).join(STATE_DIR);
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(state_dir.join("dispatcher.json"), "{ not valid json ").unwrap();

        let section = dispatcher_section(vault, Utc::now());

        assert!(
            section.contains("```"),
            "malformed file should still render a fenced block:\n{section}"
        );
        assert!(
            section.contains("not valid json"),
            "raw content should be preserved verbatim:\n{section}"
        );
    }

    #[test]
    fn refresh_is_idempotent() {
        let _workspace = TempWorkspace::new("refresh-idempotent");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join(TODO_DIR).join("svc")).unwrap();
        let tickets = vec![ticket("LOC-1", Source::Local, Priority::Medium, Utc::now())];

        assert!(refresh(vault, &tickets).failed.is_empty());
        let board_first = fs::read_to_string(vault.join(TODO_DIR).join("_TODO.md")).unwrap();
        let svc_first =
            fs::read_to_string(vault.join(TODO_DIR).join("svc").join("_svc.md")).unwrap();

        assert!(refresh(vault, &tickets).failed.is_empty());
        let board_second = fs::read_to_string(vault.join(TODO_DIR).join("_TODO.md")).unwrap();
        let svc_second =
            fs::read_to_string(vault.join(TODO_DIR).join("svc").join("_svc.md")).unwrap();

        assert_eq!(
            board_first, board_second,
            "refresh on unchanged tickets must not change the board's bytes"
        );
        assert_eq!(
            svc_first, svc_second,
            "refresh on unchanged tickets must not change the service index's bytes"
        );
    }

    #[test]
    fn only_underscore_notes_are_written_here() {
        let _workspace = TempWorkspace::new("only-underscore");
        let vault = _workspace.path();
        let store = crate::store::TicketStore::open(vault).expect("open");
        let mut t = ticket("LOC-1", Source::Local, Priority::Medium, Utc::now());
        t.service = "svc".to_string();
        store
            .save(&mut t, "# LOC-1\n", &serde_yaml::Mapping::new())
            .expect("save");

        let mut underscore_files = Vec::new();
        collect_underscore_files(&vault.join(TODO_DIR), &mut underscore_files);
        let mut names: Vec<String> = underscore_files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();

        assert_eq!(
            names,
            vec!["_TODO.md".to_string(), "_svc.md".to_string()],
            "only _TODO.md and the service indexes may exist under TODO/, anywhere"
        );

        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn breakout_payloads_are_refused_before_they_can_reach_ticket_link() {
        for payload in ["Z]] **bold** [[W", "a|b", "a#b"] {
            assert!(
                crate::safe_name::check("key", payload).is_err(),
                "`{payload}` should be refused as a key"
            );
            assert!(
                crate::safe_name::check("service", payload).is_err(),
                "`{payload}` should be refused as a service"
            );
        }

        let t = ticket("LOC-1", Source::Local, Priority::Medium, Utc::now());
        let link = ticket_link(&t);
        assert_eq!(link, "[[TODO/svc/LOC-1|LOC-1]]");
        assert_eq!(
            link.matches("[[").count(),
            1,
            "exactly one opening wikilink delimiter: {link}"
        );
        assert_eq!(
            link.matches("]]").count(),
            1,
            "exactly one closing wikilink delimiter: {link}"
        );
    }

    fn collect_underscore_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_underscore_files(&path, out);
            } else if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.starts_with('_') {
                    out.push(path.clone());
                }
            }
        }
    }
}
