//! Mirrors the open issues of the configured GitHub repositories onto
//! `<vault>/TODO/<service>/<KEY>.md` (plan §3.2, G1), and writes each
//! ticket's state back as its single `aidp:*` label (G2).
//!
//! Contract kept from the retired Jira sync: an unknown issue is created, a
//! known one updated (fully while it is not started, only its metadata once
//! it is), an issue closed without being delivered cancels its mirror, and an
//! issue no routing rule matches is counted `unrouted`. The local notes
//! section of a mirror survives every update.
//!
//! The status of a mirror not yet started follows its issue:
//!
//! 1. the skip label holds it (`blocked: skip`); an issue no rule routes waits
//!    on routing (`blocked: needs-routing`); an issue with no `CA-n` criterion
//!    waits on triage (`blocked: needs-refine`) — both show `aidp:triage`;
//! 2. otherwise its state label decides, but only when that label changed on
//!    GitHub since the last sync (or the mirror is new, or a gate above was
//!    just cleared): the label is a human intent, the local status is never
//!    overwritten by a label the sync itself wrote. `aidp:ready` makes it
//!    `todo`; no label waits for `aidp:ready` when `github.ready_required`
//!    (`blocked: not-ready`), else `todo`; any other state label hands the
//!    ticket to that state (`blocked: label:<label>`), so an issue already
//!    worked outside aidp is never dispatched again.
//!
//! Every port call goes through [`GithubPort`]; this module has no process
//! or network access of its own.

use crate::blockers::{self, BodyReference};
use crate::config::{GithubConfig, GithubRepo};
use crate::ledger::{self, Event};
use crate::model::{guard, Actor, Autopilot, IssueType, Phase, Priority, Source, Status, Ticket};
use crate::models::blocker_link::BlockerLink;
use crate::models::constants::github::{
    ACCEPTANCE_CRITERION_PATTERN, BUG_LABEL, CLOSED_REASON, CLOSED_UNDONE_REASON, CLOSES_PREFIX,
    CLOSING_KEYWORD_PATTERN, DEPENDENCY_THREADS, GATE_REASON_PREFIX, LABEL_CHANGED_PREFIX,
    NEEDS_REFINE_REASON, NEEDS_ROUTING_REASON, NOTES_LOCALES_HEADING, NOT_READY_REASON,
    PRIORITY_LABEL_PREFIX, SKIP_REASON, SYNC_STATE_FILE, VALID_REGEX,
};
use crate::models::enums::issue_state::IssueState;
use crate::models::enums::state_label::StateLabel;
use crate::models::github_issue::{GithubIssue, IssueDependency};
use crate::models::github_meta::GithubMeta;
use crate::models::github_port::GithubPort;
use crate::note;
use crate::routing::{route, RouteFacts};
use crate::state_labels::{self, LabelIntent, WantedLabel};
use crate::store::{write_atomic, ListFilter, TicketStore};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

/// Vault folder holding the ticket notes.
const TODO_DIR: &str = "TODO";

/// Hidden folder under `TODO/` holding sync state.
const STATE_DIR: &str = ".state";

/// The source a sync report names.
const SOURCE_GITHUB: &str = "github";

/// Blocked reasons the sync owns: a mirror blocked for one of them is re-evaluated each sync.
const GATE_REASONS: [&str; 4] = [
    NEEDS_ROUTING_REASON,
    NEEDS_REFINE_REASON,
    SKIP_REASON,
    NOT_READY_REASON,
];

/// Sync options.
#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    /// Compute the report, write nothing (no note, no label).
    pub dry_run: bool,
    /// Only this repository (`owner/name`); one not configured is synced with the defaults.
    pub repo: Option<String>,
    /// Overrides `github.limit` for this run.
    pub limit: Option<u32>,
}

/// The outcome of one [`sync`], persisted to `TODO/.state/github-sync.json` (not on a dry run).
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub source: &'static str,
    pub repos: Vec<String>,
    pub created: u32,
    pub updated: u32,
    pub cancelled: u32,
    pub unrouted: u32,
    pub triaged: u32,
    pub unchanged: u32,
    pub labels_pushed: u32,
    pub errors: Vec<String>,
    pub duration_ms: u64,
    /// State labels seen on the open issues, to tune `status_labels`.
    pub labels_seen: BTreeMap<String, u32>,
    pub dry_run: bool,
    pub last_sync_at: DateTime<Utc>,
}

impl Report {
    fn new(now: DateTime<Utc>, dry_run: bool) -> Report {
        Report {
            source: SOURCE_GITHUB,
            repos: Vec::new(),
            created: 0,
            updated: 0,
            cancelled: 0,
            unrouted: 0,
            triaged: 0,
            unchanged: 0,
            labels_pushed: 0,
            errors: Vec::new(),
            duration_ms: 0,
            labels_seen: BTreeMap::new(),
            dry_run,
            last_sync_at: now,
        }
    }
}

/// The outcome of [`label_ticket`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LabelOutcome {
    pub key: String,
    pub label: String,
    pub changed: bool,
    pub removed: Vec<String>,
}

/// What the gates of an issue say, before its state label is read.
#[derive(Debug, Clone, Copy)]
struct Gates {
    routed: bool,
    has_criteria: bool,
}

/// The repositories a sync covers: `opts.repo` alone, else every configured one.
fn repos_to_sync(cfg: &GithubConfig, opts: &SyncOptions) -> Vec<GithubRepo> {
    match &opts.repo {
        Some(only) => vec![cfg
            .repo(only)
            .cloned()
            .unwrap_or_else(|| GithubRepo::named(only))],
        None => cfg.repos.clone(),
    }
}

/// Pulls every covered repository, then pushes the state labels of its mirrors.
/// Per-issue problems land in [`Report::errors`]; the run continues.
pub fn sync(
    store: &TicketStore,
    port: &dyn GithubPort,
    opts: &SyncOptions,
) -> anyhow::Result<Report> {
    let start = std::time::Instant::now();
    let now = Utc::now();
    let cfg = store.config().github.clone();
    let mut report = Report::new(now, opts.dry_run);
    let repos = repos_to_sync(&cfg, opts);
    let limit = opts.limit.unwrap_or(cfg.limit).max(1);

    for repo in &repos {
        report.repos.push(repo.repo.clone());
        match port.list_open(&repo.repo, limit) {
            Ok(issues) => {
                let complete = issues.len() < limit as usize;
                pull_repo(
                    store,
                    port,
                    &cfg,
                    repo,
                    &issues,
                    complete,
                    opts,
                    now,
                    &mut report,
                );
            }
            Err(e) => report.errors.push(format!("{}: {e:#}", repo.repo)),
        }
    }
    if !opts.dry_run {
        let (pushed, errors) = push_labels(store, port, &repos);
        report.labels_pushed = pushed;
        report.errors.extend(errors);
    }

    report.duration_ms = start.elapsed().as_millis() as u64;
    if !opts.dry_run {
        persist(store, &mut report);
    }
    Ok(report)
}

fn persist(store: &TicketStore, report: &mut Report) {
    let path = store
        .vault_root()
        .join(TODO_DIR)
        .join(STATE_DIR)
        .join(SYNC_STATE_FILE);
    match serde_json::to_string_pretty(&report) {
        Ok(text) => {
            if let Err(e) = write_atomic(&path, &text) {
                report
                    .errors
                    .push(format!("failed to write {SYNC_STATE_FILE}: {e}"));
            }
        }
        Err(e) => report
            .errors
            .push(format!("failed to serialize {SYNC_STATE_FILE}: {e}")),
    }
}

/// Mirrors one repository's open issues, then settles the mirrors no longer listed.
#[allow(clippy::too_many_arguments)]
fn pull_repo(
    store: &TicketStore,
    port: &dyn GithubPort,
    cfg: &GithubConfig,
    repo: &GithubRepo,
    issues: &[GithubIssue],
    complete: bool,
    opts: &SyncOptions,
    now: DateTime<Utc>,
    report: &mut Report,
) {
    let open: HashSet<u64> = issues.iter().map(|issue| issue.number).collect();
    let native = if cfg.native_dependencies {
        native_dependencies(port, &repo.repo, issues, report)
    } else {
        HashMap::new()
    };
    for issue in issues {
        if let Some(label) = state_labels::current(&issue.labels, cfg) {
            *report.labels_seen.entry(label).or_insert(0) += 1;
        }
        let lookup = BlockerLookup {
            port,
            cfg,
            repo,
            open: &open,
            complete,
        };
        let blocked_by = lookup.resolve(issue, native.get(&issue.number));
        if let Err(e) = apply_issue(store, cfg, repo, issue, blocked_by, opts, now, report) {
            report
                .errors
                .push(format!("{}: {e:#}", repo.key_of(issue.number)));
        }
    }
    settle_absent(store, port, cfg, repo, &open, opts, now, report);
}

/// The native `blocked_by` dependencies of every issue, read on a few threads at once.
fn native_dependencies(
    port: &dyn GithubPort,
    repo: &str,
    issues: &[GithubIssue],
    report: &mut Report,
) -> HashMap<u64, Vec<IssueDependency>> {
    let numbers: Vec<u64> = issues.iter().map(|issue| issue.number).collect();
    let chunk = numbers.len().div_ceil(DEPENDENCY_THREADS).max(1);
    let results: Vec<(u64, anyhow::Result<Vec<IssueDependency>>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = numbers
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|number| (*number, port.blocked_by(repo, *number)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    let mut out = HashMap::new();
    for (number, result) in results {
        match result {
            Ok(dependencies) => {
                out.insert(number, dependencies);
            }
            Err(e) => report
                .errors
                .push(format!("{repo}#{number}: dependencies: {e:#}")),
        }
    }
    out
}

/// Resolves the blockers of an issue to mirror keys and states.
struct BlockerLookup<'a> {
    port: &'a dyn GithubPort,
    cfg: &'a GithubConfig,
    repo: &'a GithubRepo,
    open: &'a HashSet<u64>,
    complete: bool,
}

impl BlockerLookup<'_> {
    fn resolve(
        &self,
        issue: &GithubIssue,
        native: Option<&Vec<IssueDependency>>,
    ) -> Vec<BlockerLink> {
        let mut links: Vec<BlockerLink> = Vec::new();
        let push = |link: BlockerLink, links: &mut Vec<BlockerLink>| match links
            .iter_mut()
            .find(|known| known.key == link.key)
        {
            Some(known) => known.state = link.state,
            None => links.push(link),
        };
        for reference in blockers::body_references(&issue.body) {
            if let Some(link) = self.body_link(issue.number, &reference) {
                push(link, &mut links);
            }
        }
        for dependency in native.into_iter().flatten() {
            let Some(target) = self.target_repo(&dependency.repo) else {
                continue;
            };
            push(
                BlockerLink {
                    key: target.key_of(dependency.number),
                    state: dependency.state,
                },
                &mut links,
            );
        }
        links
    }

    /// The configured repository named `repo` (the synced one included).
    fn target_repo(&self, repo: &str) -> Option<GithubRepo> {
        if repo.eq_ignore_ascii_case(&self.repo.repo) {
            return Some(self.repo.clone());
        }
        self.cfg.repo(repo).cloned()
    }

    fn body_link(&self, own: u64, reference: &BodyReference) -> Option<BlockerLink> {
        let repo_name = reference.repo.as_deref().unwrap_or(&self.repo.repo);
        let target = self.target_repo(repo_name)?;
        let same = target.repo.eq_ignore_ascii_case(&self.repo.repo);
        if same && reference.number == own {
            return None;
        }
        let state = if same && self.open.contains(&reference.number) {
            IssueState::Open
        } else if same && self.complete {
            IssueState::Closed
        } else {
            match self.port.view(&target.repo, reference.number) {
                Ok(Some(found)) => found.state,
                Ok(None) => IssueState::Closed,
                Err(_) => IssueState::Open,
            }
        };
        Some(BlockerLink {
            key: target.key_of(reference.number),
            state,
        })
    }
}

/// True when the body names at least one `CA-n` acceptance criterion.
pub fn has_acceptance_criteria(body: &str) -> bool {
    Regex::new(ACCEPTANCE_CRITERION_PATTERN)
        .expect(VALID_REGEX)
        .is_match(body)
}

/// The priority the `prio:*` labels give, `Medium` without one.
pub fn priority_of(labels: &[String]) -> Priority {
    labels
        .iter()
        .find_map(|label| {
            let lower = label.to_ascii_lowercase();
            let value = lower.strip_prefix(PRIORITY_LABEL_PREFIX)?;
            Priority::from_str(value).ok()
        })
        .unwrap_or(Priority::Medium)
}

fn issue_type_of(labels: &[String]) -> IssueType {
    if labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case(BUG_LABEL))
    {
        IssueType::Bug
    } else {
        IssueType::Task
    }
}

/// `body` ending with a closing keyword for issue `number`: unchanged when it already closes it.
pub fn with_closing_line(body: &str, number: u64) -> String {
    let pattern = CLOSING_KEYWORD_PATTERN.replace("{n}", &number.to_string());
    if Regex::new(&pattern).is_ok_and(|re| re.is_match(body)) {
        return body.to_string();
    }
    let trimmed = body.trim_end();
    if trimmed.is_empty() {
        return format!("{CLOSES_PREFIX}{number}\n");
    }
    format!("{trimmed}\n\n{CLOSES_PREFIX}{number}\n")
}

/// The mirror body: the issue title, its body, then the local notes section.
fn build_body(key: &str, issue: &GithubIssue) -> String {
    let body = note::normalise_body(&issue.body);
    let body = body.trim();
    let mut out = format!("# {key} - {}\n\n", issue.title.trim());
    if !body.is_empty() {
        out.push_str(body);
        out.push_str("\n\n");
    }
    out.push_str(NOTES_LOCALES_HEADING);
    out.push('\n');
    out
}

/// Splits `body` at the local notes heading; the second part keeps the heading, byte for byte.
fn split_notes_locales(body: &str) -> (&str, Option<&str>) {
    let mut offset = 0usize;
    for line in body.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']).trim() == NOTES_LOCALES_HEADING {
            return (&body[..offset], Some(&body[offset..]));
        }
        offset += line.len();
    }
    (body, None)
}

/// The fresh body with the local notes of `old_body` kept.
fn merged_body(fresh: &str, old_body: &str) -> String {
    let (before, _) = split_notes_locales(fresh);
    match split_notes_locales(old_body).1 {
        Some(notes) => format!("{before}{notes}"),
        None => fresh.to_string(),
    }
}

/// The status a state label asks for, once the gates pass.
fn target_for(intent: &LabelIntent, cfg: &GithubConfig) -> (Status, Option<String>) {
    match intent {
        LabelIntent::Unlabelled | LabelIntent::Triage if cfg.ready_required => {
            (Status::Blocked, Some(NOT_READY_REASON.to_string()))
        }
        LabelIntent::Unlabelled
        | LabelIntent::Triage
        | LabelIntent::Ready
        | LabelIntent::ToTest => (Status::Todo, None),
        LabelIntent::Skip => (Status::Blocked, Some(SKIP_REASON.to_string())),
        LabelIntent::Held(label) => (Status::Blocked, Some(state_labels::held_reason(label))),
    }
}

/// The status the gates impose, `None` when every gate passes.
fn gate_target(intent: &LabelIntent, gates: Gates) -> Option<(Status, Option<String>)> {
    let blocked = |reason: &str| Some((Status::Blocked, Some(reason.to_string())));
    if *intent == LabelIntent::Skip {
        return blocked(SKIP_REASON);
    }
    if !gates.routed {
        return blocked(NEEDS_ROUTING_REASON);
    }
    if !gates.has_criteria {
        return blocked(NEEDS_REFINE_REASON);
    }
    None
}

/// The status of a mirror not yet started. `current` is `None` for a new mirror.
fn decide(
    current: Option<(Status, Option<&str>)>,
    label_changed: bool,
    intent: &LabelIntent,
    gates: Gates,
    cfg: &GithubConfig,
) -> (Status, Option<String>) {
    if let Some(gated) = gate_target(intent, gates) {
        return gated;
    }
    match current {
        None | Some((Status::Cancelled, _)) => target_for(intent, cfg),
        Some(_) if label_changed => target_for(intent, cfg),
        Some((Status::Blocked, Some(reason))) if GATE_REASONS.contains(&reason) => {
            target_for(intent, cfg)
        }
        Some((status, reason)) => (status, reason.map(str::to_string)),
    }
}

/// True when the sync may move the mirror's status: not started, no session or gate alive.
fn is_editable(ticket: &Ticket) -> bool {
    let gate_waiting = ticket
        .autopilot
        .blocked_reason
        .as_deref()
        .is_some_and(|reason| reason.starts_with("gate:"));
    matches!(
        ticket.status,
        Status::Todo | Status::Queued | Status::Blocked | Status::Cancelled
    ) && ticket.autopilot.session_id.is_none()
        && !gate_waiting
}

fn journal(store: &TicketStore, event: Event) {
    ledger::append(store.vault_root(), &event);
}

fn event(
    key: &str,
    from: Option<Status>,
    to: Status,
    reason: Option<String>,
    ts: DateTime<Utc>,
) -> Event {
    Event {
        ts,
        key: key.to_string(),
        from,
        to,
        actor: Actor::Sync,
        reason,
        session_id: None,
    }
}

/// Moves `ticket` to `to` through the matrix, via `todo` when a reopened mirror needs it.
fn transition(
    ticket: &mut Ticket,
    to: Status,
    reason: Option<String>,
    journal_reason: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<Event>> {
    let from = ticket.status;
    let mut events = Vec::new();
    if from != to {
        if guard(from, to, Actor::Sync, false).is_err()
            && guard(from, Status::Todo, Actor::Sync, false).is_ok()
            && guard(Status::Todo, to, Actor::Sync, false).is_ok()
        {
            events.push(event(
                &ticket.key,
                Some(from),
                Status::Todo,
                Some(journal_reason.to_string()),
                now,
            ));
            events.push(event(
                &ticket.key,
                Some(Status::Todo),
                to,
                Some(journal_reason.to_string()),
                now,
            ));
        } else {
            guard(from, to, Actor::Sync, false)?;
            events.push(event(
                &ticket.key,
                Some(from),
                to,
                Some(journal_reason.to_string()),
                now,
            ));
        }
        ticket.status = to;
    }
    ticket.autopilot.blocked_reason = if to == Status::Blocked { reason } else { None };
    Ok(events)
}

/// The journal reason of a sync-decided status: the label for a label change, else the gate.
fn journal_reason(label_changed: bool, label: Option<&str>, reason: Option<&str>) -> String {
    if label_changed {
        return format!("{LABEL_CHANGED_PREFIX} {}", label.unwrap_or("-"));
    }
    format!("{GATE_REASON_PREFIX} {}", reason.unwrap_or("ready"))
}

/// Creates or updates the mirror of one open issue.
#[allow(clippy::too_many_arguments)]
fn apply_issue(
    store: &TicketStore,
    cfg: &GithubConfig,
    repo: &GithubRepo,
    issue: &GithubIssue,
    blocked_by: Vec<BlockerLink>,
    opts: &SyncOptions,
    now: DateTime<Utc>,
    report: &mut Report,
) -> anyhow::Result<()> {
    let key = repo.key_of(issue.number);
    let issue_type = issue_type_of(&issue.labels);
    let facts = RouteFacts {
        key: key.clone(),
        summary: issue.title.clone(),
        issuetype: Some(issue_type_name(issue_type).to_string()),
        labels: issue.labels.clone(),
        components: Vec::new(),
        epic: None,
        body: issue.body.clone(),
    };
    let routed = route(&facts, store.routing());
    let gates = Gates {
        routed: routed.is_some(),
        has_criteria: has_acceptance_criteria(&issue.body),
    };
    let label = state_labels::current(&issue.labels, cfg);
    let intent = state_labels::intent(label.as_deref(), cfg);
    let meta = GithubMeta {
        repo: repo.repo.clone(),
        number: issue.number,
        url: issue.url.clone(),
        state: issue.state,
        synced_at: Some(now),
        blocked_by,
    };
    let fresh_body = build_body(&key, issue);

    if store.path_of(&key).is_none() {
        let (status, reason) = decide(None, true, &intent, gates, cfg);
        let mut ticket = Ticket {
            key: key.clone(),
            source: Source::Github,
            r#type: issue_type,
            summary: issue.title.clone(),
            status,
            priority: priority_of(&issue.labels),
            priority_rank: 0,
            service: repo.service(),
            template: routed.as_ref().and_then(|r| r.template.clone()),
            track: routed.as_ref().and_then(|r| r.track),
            route: routed.as_ref().map(|r| r.route.clone()),
            flow: routed.as_ref().and_then(|r| r.flow.clone()),
            labels: issue.labels.clone(),
            components: Vec::new(),
            epic: None,
            assignee: None,
            milestone: issue.milestone.clone(),
            created: issue.created_at.unwrap_or(now),
            updated: issue.updated_at.unwrap_or(now),
            github: Some(meta),
            autopilot: Autopilot {
                blocked_reason: reason.clone(),
                ..Autopilot::default()
            },
            tags: Vec::new(),
        };
        report.created += 1;
        if !gates.routed {
            report.unrouted += 1;
        }
        if reason.as_deref() == Some(NEEDS_REFINE_REASON) {
            report.triaged += 1;
        }
        if opts.dry_run {
            return Ok(());
        }
        store.save(&mut ticket, &fresh_body, &serde_yaml::Mapping::new())?;
        let why = journal_reason(false, label.as_deref(), reason.as_deref());
        journal(store, event(&key, None, ticket.status, Some(why), now));
        return Ok(());
    }

    let (old, old_body, extra) = store.get(&key)?;
    let mut ticket = old.clone();
    let previous_label = state_labels::current(&old.labels, cfg);
    let label_changed = previous_label.as_deref().map(str::to_ascii_lowercase)
        != label.as_deref().map(str::to_ascii_lowercase);
    let mut body = old_body.clone();
    let mut events = Vec::new();

    ticket.summary = issue.title.clone();
    ticket.priority = priority_of(&issue.labels);
    ticket.labels = issue.labels.clone();
    ticket.milestone = issue.milestone.clone();
    ticket.github = Some(meta);
    if is_editable(&old) {
        ticket.r#type = issue_type;
        ticket.route = routed.as_ref().map(|r| r.route.clone());
        if let Some(r) = &routed {
            ticket.template = r.template.clone();
            ticket.track = r.track;
            ticket.flow = r.flow.clone();
        }
        body = merged_body(&fresh_body, &old_body);
        let (status, reason) = decide(
            Some((old.status, old.autopilot.blocked_reason.as_deref())),
            label_changed,
            &intent,
            gates,
            cfg,
        );
        let why = journal_reason(label_changed, label.as_deref(), reason.as_deref());
        let triage_now = reason.as_deref() == Some(NEEDS_REFINE_REASON)
            && old.autopilot.blocked_reason.as_deref() != Some(NEEDS_REFINE_REASON);
        events = transition(&mut ticket, status, reason, &why, now)?;
        if triage_now {
            report.triaged += 1;
        }
    }
    if label_changed {
        let held_by_label = is_editable(&old)
            || (old.status == Status::InProgress && old.autopilot.session_id.is_none());
        if held_by_label {
            if intent == LabelIntent::ToTest {
                ticket.autopilot.set_phase(Phase::ToTest, now);
            } else if ticket.autopilot.phase() == Phase::ToTest {
                ticket.autopilot.phase = None;
                ticket.autopilot.phase_at = None;
            }
        }
    }

    let mut compared = ticket.clone();
    if let (Some(fresh), Some(previous)) = (compared.github.as_mut(), old.github.as_ref()) {
        fresh.synced_at = previous.synced_at;
    }
    if compared == old && body == old_body {
        report.unchanged += 1;
        return Ok(());
    }
    report.updated += 1;
    if opts.dry_run {
        return Ok(());
    }
    store.save(&mut ticket, &body, &extra)?;
    for e in events {
        journal(store, e);
    }
    Ok(())
}

fn issue_type_name(issue_type: IssueType) -> &'static str {
    crate::models::enums::cli_kind::IssueTypeName(issue_type).as_str()
}

/// Settles the mirrors of `repo` that the open listing no longer holds.
#[allow(clippy::too_many_arguments)]
fn settle_absent(
    store: &TicketStore,
    port: &dyn GithubPort,
    cfg: &GithubConfig,
    repo: &GithubRepo,
    open: &HashSet<u64>,
    opts: &SyncOptions,
    now: DateTime<Utc>,
    report: &mut Report,
) {
    let mirrors = store.list(&ListFilter {
        source: Some(Source::Github),
        ..ListFilter::default()
    });
    for ticket in mirrors {
        let Some(meta) = ticket.github.as_ref() else {
            continue;
        };
        if !meta.repo.eq_ignore_ascii_case(&repo.repo) || open.contains(&meta.number) {
            continue;
        }
        if matches!(ticket.status, Status::Done | Status::Cancelled)
            && meta.state == IssueState::Closed
        {
            continue;
        }
        let number = meta.number;
        match port.view(&repo.repo, number) {
            Ok(Some(issue)) if issue.state == IssueState::Open => {
                let lookup = BlockerLookup {
                    port,
                    cfg,
                    repo,
                    open,
                    complete: false,
                };
                let blocked_by = lookup.resolve(&issue, None);
                if let Err(e) = apply_issue(store, cfg, repo, &issue, blocked_by, opts, now, report)
                {
                    report.errors.push(format!("{}: {e:#}", ticket.key));
                }
            }
            Ok(_) => {
                if let Err(e) = apply_closed(store, &ticket.key, opts, now, report) {
                    report.errors.push(format!("{}: {e:#}", ticket.key));
                }
            }
            Err(e) => report.errors.push(format!("{}: {e:#}", ticket.key)),
        }
    }
}

/// The issue of `key` is closed (or gone): a mirror not started is cancelled, one waiting on
/// its merge is done, a running one only records the closed state.
fn apply_closed(
    store: &TicketStore,
    key: &str,
    opts: &SyncOptions,
    now: DateTime<Utc>,
    report: &mut Report,
) -> anyhow::Result<()> {
    let (old, body, extra) = store.get(key)?;
    let mut ticket = old.clone();
    if let Some(meta) = ticket.github.as_mut() {
        meta.state = IssueState::Closed;
        meta.synced_at = Some(now);
    }
    let (to, reason) = if is_editable(&old) && old.status != Status::Cancelled {
        (Some(Status::Cancelled), CLOSED_UNDONE_REASON)
    } else if old.status == Status::InReview {
        (Some(Status::Done), CLOSED_REASON)
    } else {
        (None, CLOSED_REASON)
    };
    let events = match to {
        Some(to) => transition(&mut ticket, to, None, reason, now)?,
        None => Vec::new(),
    };
    if to == Some(Status::Cancelled) {
        report.cancelled += 1;
    } else {
        report.updated += 1;
    }
    if opts.dry_run {
        return Ok(());
    }
    store.save(&mut ticket, &body, &extra)?;
    for e in events {
        journal(store, e);
    }
    Ok(())
}

/// Writes the state label of every mirror of `repos` whose labels do not show its status yet.
/// Returns how many issues were relabelled and the per-issue errors.
pub fn push_labels(
    store: &TicketStore,
    port: &dyn GithubPort,
    repos: &[GithubRepo],
) -> (u32, Vec<String>) {
    let cfg = &store.config().github;
    let mut pushed = 0;
    let mut errors = Vec::new();
    let mirrors = store.list(&ListFilter {
        source: Some(Source::Github),
        ..ListFilter::default()
    });
    for ticket in mirrors {
        let Some(meta) = ticket.github.as_ref() else {
            continue;
        };
        if !repos
            .iter()
            .any(|r| r.repo.eq_ignore_ascii_case(&meta.repo))
        {
            continue;
        }
        let wanted = state_labels::wanted(&ticket, cfg);
        let Some(change) = state_labels::edit(&ticket.labels, &wanted, cfg) else {
            continue;
        };
        match write_labels(store, port, &ticket.key, &meta.repo, meta.number, &change) {
            Ok(()) => pushed += 1,
            Err(e) => errors.push(format!(
                "{}: labels {:?}: {e:#} (missing labels: `aidp gh labels-migrate --repo {} --apply`)",
                ticket.key, change.add, meta.repo
            )),
        }
    }
    (pushed, errors)
}

/// [`push_labels`] over every configured repository: the daemon's write-back after a transition.
pub fn push_configured_labels(store: &TicketStore, port: &dyn GithubPort) -> (u32, Vec<String>) {
    let repos = store.config().github.repos.clone();
    if repos.is_empty() {
        return (0, Vec::new());
    }
    push_labels(store, port, &repos)
}

/// Applies `change` on GitHub, then records the new labels on the mirror.
fn write_labels(
    store: &TicketStore,
    port: &dyn GithubPort,
    key: &str,
    repo: &str,
    number: u64,
    change: &state_labels::LabelEdit,
) -> anyhow::Result<()> {
    port.edit_labels(repo, number, &change.add, &change.remove)?;
    let (mut ticket, body, extra) = store.get(key)?;
    ticket.labels = state_labels::apply(&ticket.labels, change);
    store.save(&mut ticket, &body, &extra)?;
    Ok(())
}

/// The GitHub coordinates of a mirror, refused for a local ticket.
fn github_of(store: &TicketStore, key: &str) -> anyhow::Result<(Ticket, GithubMeta)> {
    let (ticket, _, _) = store.get(key)?;
    let meta = ticket.github.clone().ok_or_else(|| {
        crate::ticket_error::TicketError::Conflict(format!(
            "`{key}` is not a GitHub mirror (source: {})",
            ticket.source
        ))
    })?;
    Ok((ticket, meta))
}

/// The label `to-test` puts the ticket in the `to-test` phase, `testing` in `testing`; any other
/// label leaves the phase alone. True when the ticket changed.
fn set_phase_of_label(ticket: &mut Ticket, label: &str, cfg: &GithubConfig) -> bool {
    let phase = if label.eq_ignore_ascii_case(&StateLabel::ToTest.label(&cfg.label_prefix)) {
        Phase::ToTest
    } else if label.eq_ignore_ascii_case(&StateLabel::Testing.label(&cfg.label_prefix)) {
        Phase::Testing
    } else {
        return false;
    };
    if ticket.autopilot.phase() == phase && ticket.autopilot.phase.is_some() {
        return false;
    }
    ticket.autopilot.set_phase(phase, Utc::now());
    true
}

/// Sets the single state label `state` (a suffix such as `review`, or a full label) on the
/// issue of mirror `key`: the other state labels go, the mirror records the result.
pub fn label_ticket(
    store: &TicketStore,
    port: &dyn GithubPort,
    key: &str,
    state: &str,
) -> anyhow::Result<LabelOutcome> {
    let cfg = &store.config().github;
    let label = cfg.state_label(state).ok_or_else(|| {
        crate::ticket_error::TicketError::Config(format!(
            "unknown state label `{state}`: expected one of {}",
            cfg.known_state_labels().join(", ")
        ))
    })?;
    let (ticket, meta) = github_of(store, key)?;
    let current = match port.view(&meta.repo, meta.number) {
        Ok(Some(issue)) => issue.labels,
        _ => ticket.labels.clone(),
    };
    let change = state_labels::edit(&current, &WantedLabel::Exactly(label.clone()), cfg);
    let Some(change) = change else {
        let (mut fresh, body, extra) = store.get(key)?;
        if set_phase_of_label(&mut fresh, &label, cfg) {
            store.save(&mut fresh, &body, &extra)?;
        }
        return Ok(LabelOutcome {
            key: key.to_string(),
            label,
            changed: false,
            removed: Vec::new(),
        });
    };
    port.edit_labels(&meta.repo, meta.number, &change.add, &change.remove)?;
    let (mut fresh, body, extra) = store.get(key)?;
    fresh.labels = state_labels::apply(&current, &change);
    set_phase_of_label(&mut fresh, &label, cfg);
    store.save(&mut fresh, &body, &extra)?;
    Ok(LabelOutcome {
        key: key.to_string(),
        label,
        changed: true,
        removed: change.remove,
    })
}

/// Posts `body` as a comment on the issue of mirror `key`; returns the comment URL if any.
pub fn comment_ticket(
    store: &TicketStore,
    port: &dyn GithubPort,
    key: &str,
    body: &str,
) -> anyhow::Result<Option<String>> {
    let (_, meta) = github_of(store, key)?;
    port.comment(&meta.repo, meta.number, body)
}

#[cfg(test)]
#[path = "../tests/unit/github_sync_tests.rs"]
mod tests;
