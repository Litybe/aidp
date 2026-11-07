//! Squad pull request inbox served by the daemon: one cache of the unfiltered
//! fetch, per-request filters, and the marks route.

use crate::attention::signals;
use crate::bus::Bus;
use crate::flows::FlowEngine;
use crate::models::attention_context::AttentionContext;
use crate::models::attention_event::AttentionEvent;
use crate::models::attention_facts::AttentionFacts;
use crate::models::attention_read_request::AttentionReadRequest;
use crate::models::enums::ApiStatus;
use crate::models::inbox_changed_event::InboxChangedEvent;
use crate::models::pr_attention_record::PrAttentionRecord;
use crate::models::pr_flow_ref::PrFlowRef;
use crate::models::pr_mark_request::PrMarkRequest;
use crate::models::pr_mark_response::PrMarkResponse;
use crate::models::squad_pr_row::SquadPrRow;
use crate::models::squad_prs_query::SquadPrsQuery;
use crate::models::squad_prs_response::SquadPrsResponse;
use crate::models::ApiError;
use crate::pr_attention::PrAttention;
use crate::pr_marks::PrMarks;
use crate::projects::Projects;
use crate::prs::PrRouteError;
use crate::store::{ensure_dir, read_json, write_json};
use crate::sync::lock;
use aidp_core::clock::{Clock, SystemClock};
use aidp_integrations::gh::{in_dir, repo_slug};
use aidp_integrations::models::enums::bots_filter::BotsFilter;
use aidp_integrations::models::enums::lane::Lane;
use aidp_integrations::models::pr_mark::PrMark;
use aidp_integrations::models::squad_config::SquadConfig;
use aidp_integrations::models::squad_pr::SquadPr;
use aidp_integrations::models::squad_prs::SquadPrs;
use aidp_integrations::notifications::load_notifications_pr;
use aidp_integrations::pr::{
    merge_gate, ChecksVerdict, GateFacts, Mergeable, PrState, ReviewDecision, StatusContextState,
};
use aidp_integrations::squad::{
    classify, is_bot, load_config, mark_key, me_login, my_review_state, squad_prs, squad_repos,
    OsGhRunner,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

pub const SQUAD_TTL_MS: i64 = 120_000;
/// Storage instant of an invalidated entry: older than any TTL, newer than no request.
const INVALIDATED_AT: i64 = i64::MIN;
pub const INBOX_CHANGED_EVENT: &str = "pr.inbox_changed";
pub const ATTENTION_EVENT: &str = "pr.attention";
pub const SQUAD_REPOS_CACHE_DIR: &str = "squad";
/// Last fetch, kept across daemon restarts so a restart lists at once and refreshes incrementally.
pub const SQUAD_SNAPSHOT_FILE: &str = "squad-prs.json";
pub const WEBHOOK_EVENTS: &[&str] = &[
    EVENT_PULL_REQUEST,
    EVENT_PULL_REQUEST_REVIEW,
    EVENT_PULL_REQUEST_REVIEW_COMMENT,
    EVENT_PULL_REQUEST_REVIEW_THREAD,
    EVENT_CHECK_SUITE,
    EVENT_ISSUE_COMMENT,
];

const EVENT_PULL_REQUEST: &str = "pull_request";
const EVENT_PULL_REQUEST_REVIEW: &str = "pull_request_review";
const EVENT_PULL_REQUEST_REVIEW_COMMENT: &str = "pull_request_review_comment";
const EVENT_PULL_REQUEST_REVIEW_THREAD: &str = "pull_request_review_thread";
const EVENT_CHECK_SUITE: &str = "check_suite";
const EVENT_ISSUE_COMMENT: &str = "issue_comment";
const CHANGES_REQUESTED_DETAIL: &str = "reviewers";
const BRANCH_PREFIX: &str = "feat/";
const MESSAGE_NOT_IN_INBOX: &str = "pull request not in the squad inbox";

/// Source of the unfiltered squad fetch; replaceable in tests. `previous` is the last
/// fetch, whose unchanged pull requests need not be read again.
pub trait SquadFetcher: Send + Sync {
    fn fetch(&self, previous: Option<&SquadPrs>) -> Result<SquadPrs, String>;
}

/// The real fetcher: `gh`, with the daemon's registered projects as the fallback repos.
pub struct GhSquadFetcher {
    projects: Arc<Projects>,
    cache_dir: PathBuf,
    me: Mutex<Option<String>>,
    fallback: Mutex<Option<Vec<String>>>,
}

impl GhSquadFetcher {
    pub fn new(projects: Arc<Projects>, state_dir: &Path) -> Self {
        GhSquadFetcher {
            projects,
            cache_dir: state_dir.join(SQUAD_REPOS_CACHE_DIR),
            me: Mutex::new(None),
            fallback: Mutex::new(None),
        }
    }

    fn me(&self) -> Result<String, String> {
        if let Some(me) = lock(&self.me).clone() {
            return Ok(me);
        }
        let login = me_login(&OsGhRunner)?;
        *lock(&self.me) = Some(login.clone());
        Ok(login)
    }

    fn fallback_repos(&self) -> Vec<String> {
        if let Some(repos) = lock(&self.fallback).clone() {
            return repos;
        }
        let repos: Vec<String> = self
            .projects
            .list()
            .iter()
            .filter_map(|project| in_dir(Path::new(&project.path), || repo_slug(None)))
            .collect();
        *lock(&self.fallback) = Some(repos.clone());
        repos
    }
}

impl SquadFetcher for GhSquadFetcher {
    fn fetch(&self, previous: Option<&SquadPrs>) -> Result<SquadPrs, String> {
        let me = self.me()?;
        let clock = SystemClock;
        let (repos, degraded) = squad_repos(
            &load_config(),
            &self.fallback_repos(),
            &self.cache_dir,
            &clock,
            &OsGhRunner,
        );
        let mut data = squad_prs(&repos, &me, &OsGhRunner, &clock, previous);
        data.degraded.splice(0..0, degraded);
        Ok(data)
    }
}

/// One cache of the full squad fetch, valid [`SQUAD_TTL_MS`]. One fetch runs at a time:
/// callers that wait for it reuse its result instead of reading GitHub again.
pub struct SquadPrsCache {
    clock: Box<dyn Clock>,
    fetcher: Box<dyn SquadFetcher>,
    entry: Mutex<Option<(Arc<SquadPrs>, i64)>>,
    /// Held for the whole fetch; the fetch itself never holds `entry`.
    fetching: Mutex<()>,
    /// Bumped by every invalidation: a fetch started before one stores its result as stale.
    generation: AtomicU64,
    /// Where every fetch is saved, when the cache outlives the process.
    snapshot: Option<PathBuf>,
}

impl SquadPrsCache {
    pub fn new(clock: Box<dyn Clock>, fetcher: Box<dyn SquadFetcher>) -> Self {
        SquadPrsCache {
            clock,
            fetcher,
            entry: Mutex::new(None),
            fetching: Mutex::new(()),
            generation: AtomicU64::new(0),
            snapshot: None,
        }
    }

    /// A cache that starts from the fetch saved at `snapshot`, served as stale, and saves every fetch there.
    pub fn persisted(
        clock: Box<dyn Clock>,
        fetcher: Box<dyn SquadFetcher>,
        snapshot: PathBuf,
    ) -> Self {
        let saved: Option<SquadPrs> = read_json(&snapshot, None);
        let cache = SquadPrsCache {
            snapshot: Some(snapshot),
            ..SquadPrsCache::new(clock, fetcher)
        };
        *lock(&cache.entry) = saved.map(|data| (Arc::new(data), INVALIDATED_AT));
        cache
    }

    fn save(&self, data: &SquadPrs) {
        let Some(path) = &self.snapshot else {
            return;
        };
        if let Some(dir) = path.parent() {
            ensure_dir(dir);
        }
        if let Err(error) = write_json(path, data) {
            tracing::warn!(%error, "squad inbox snapshot not saved");
        }
    }

    fn is_fresh(&self, stored_at: i64) -> bool {
        self.clock.now_ms().saturating_sub(stored_at) < SQUAD_TTL_MS
    }

    /// True when a fetch is cached but older than [`SQUAD_TTL_MS`] or invalidated.
    pub fn is_stale(&self) -> bool {
        lock(&self.entry)
            .as_ref()
            .is_some_and(|(_, stored_at)| !self.is_fresh(*stored_at))
    }

    /// The cached fetch even when stale, so a list answers at once; fetches only when nothing is cached.
    pub fn get_or_stale(&self) -> Result<Arc<SquadPrs>, String> {
        if let Some((data, _)) = lock(&self.entry).clone() {
            return Ok(data);
        }
        self.get()
    }

    /// Fetches with the fetch lock held, unless a fetch stored after `requested_at` already answers.
    fn fetch_locked(&self, requested_at: i64) -> Result<(Arc<SquadPrs>, bool), String> {
        let cached = lock(&self.entry).clone();
        if let Some((data, stored_at)) = &cached {
            if *stored_at >= requested_at {
                return Ok((data.clone(), false));
            }
        }
        let started = self.generation.load(Ordering::SeqCst);
        let previous = cached.as_ref().map(|(data, _)| data.as_ref());
        let data = Arc::new(self.fetcher.fetch(previous)?);
        let current = self.generation.load(Ordering::SeqCst) == started;
        let stored_at = if current {
            self.clock.now_ms()
        } else {
            INVALIDATED_AT
        };
        *lock(&self.entry) = Some((data.clone(), stored_at));
        self.save(&data);
        Ok((data, current))
    }

    /// A fetch that answers a read made at `requested_at`, waiting for the one running if any.
    fn fetch_shared(&self, requested_at: i64) -> Result<Arc<SquadPrs>, String> {
        let _fetching = lock(&self.fetching);
        self.fetch_locked(requested_at).map(|(data, _)| data)
    }

    /// Fetches again unless a fetch already runs; true when this call stored a current result.
    pub fn refresh_once(&self) -> bool {
        let _fetching = match self.fetching.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return false,
        };
        match self.fetch_locked(self.clock.now_ms()) {
            Ok((_, stored)) => stored,
            Err(message) => {
                tracing::warn!(%message, "squad pull requests refresh failed");
                false
            }
        }
    }

    /// A fetch made after this call, for a reader who asked for fresh data.
    pub fn refresh_now(&self) -> Result<Arc<SquadPrs>, String> {
        self.fetch_shared(self.clock.now_ms())
    }

    /// The cached fetch while fresh, else a new fetch, stored.
    pub fn get(&self) -> Result<Arc<SquadPrs>, String> {
        let requested_at = self.clock.now_ms();
        if let Some((data, stored_at)) = lock(&self.entry).clone() {
            if self.is_fresh(stored_at) {
                return Ok(data);
            }
        }
        self.fetch_shared(requested_at)
    }

    /// Marks the cache stale, keeping it to answer at once, and tells subscribers the inbox changed.
    pub fn invalidate(&self, bus: &Bus) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some((_, stored_at)) = lock(&self.entry).as_mut() {
            *stored_at = INVALIDATED_AT;
        }
        self.notify_changed(bus);
    }

    pub fn notify_changed(&self, bus: &Bus) {
        if let Ok(payload) = serde_json::to_value(InboxChangedEvent {}) {
            bus.emit(INBOX_CHANGED_EVENT, payload);
        }
    }
}

/// True for the GitHub events that can change the inbox.
pub fn is_inbox_event(event: &str) -> bool {
    WEBHOOK_EVENTS.contains(&event)
}

fn route_error(status: ApiStatus, message: impl ToString) -> PrRouteError {
    PrRouteError {
        error: ApiError {
            status,
            message: message.to_string(),
        },
        reason: None,
    }
}

fn matches_text(row: &SquadPrRow, needle: &str) -> bool {
    [&row.title, &row.repo, &row.author]
        .iter()
        .any(|field| field.to_lowercase().contains(needle))
}

fn accepts_bots(row: &SquadPrRow, bots: BotsFilter, config: &SquadConfig) -> bool {
    match bots {
        BotsFilter::Show => true,
        BotsFilter::Hide => !is_bot(&row.author, config),
        BotsFilter::Only => is_bot(&row.author, config),
    }
}

fn matches_query(row: &SquadPrRow, query: &SquadPrsQuery, config: &SquadConfig) -> bool {
    let repo_ok = query
        .repo
        .as_ref()
        .is_none_or(|repo| row.repo.eq_ignore_ascii_case(repo));
    let author_ok = query
        .author
        .as_ref()
        .is_none_or(|author| row.author.eq_ignore_ascii_case(author));
    let text_ok = query
        .text
        .as_ref()
        .is_none_or(|text| matches_text(row, &text.to_lowercase()));
    repo_ok && author_ok && text_ok && accepts_bots(row, query.bots, config)
}

fn lane_id(lane: Lane) -> String {
    serde_json::to_value(lane)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Rows of the cached fetch with lanes and marks; an ignored pull request has no lane and is dropped.
fn rows_of(data: &SquadPrs, marks: &BTreeMap<String, PrMark>) -> Vec<SquadPrRow> {
    data.prs
        .iter()
        .filter_map(|pr| {
            let mark = marks.get(&mark_key(&pr.repo, pr.number));
            let lanes = classify(pr, &data.me, mark);
            if lanes.is_empty() {
                return None;
            }
            let seen = mark.is_some_and(|mark| mark.is_seen(&pr.head_sha));
            let ignored = mark.is_some_and(|mark| mark.ignored);
            let state = my_review_state(pr, &data.me);
            Some(SquadPrRow::new(pr, lanes, state, seen, ignored))
        })
        .collect()
}

/// The response for `query`; lane counts ignore the lane filter so each column keeps its total.
pub fn build_response(
    data: &SquadPrs,
    marks: &BTreeMap<String, PrMark>,
    query: &SquadPrsQuery,
    config: &SquadConfig,
    flow_of: impl Fn(&SquadPrRow) -> Option<PrFlowRef>,
) -> SquadPrsResponse {
    let candidates: Vec<SquadPrRow> = rows_of(data, marks)
        .into_iter()
        .filter(|row| matches_query(row, query, config))
        .collect();
    let mut lanes: BTreeMap<String, usize> = BTreeMap::new();
    for row in &candidates {
        for lane in &row.lanes {
            *lanes.entry(lane_id(*lane)).or_default() += 1;
        }
    }
    let prs = candidates
        .into_iter()
        .filter(|row| query.lane.is_none_or(|lane| row.lanes.contains(&lane)))
        .map(|mut row| {
            row.flow = flow_of(&row);
            row
        })
        .collect();
    SquadPrsResponse {
        prs,
        lanes,
        fetched_at: data.fetched_at.clone(),
        degraded: data.degraded.clone(),
    }
}

fn cached_data(cache: &SquadPrsCache) -> Result<Arc<SquadPrs>, PrRouteError> {
    cache
        .get()
        .map_err(|message| route_error(ApiStatus::Upstream, message))
}

/// The cached rows even when stale: the caller refreshes them in the background.
fn listed_data(cache: &SquadPrsCache) -> Result<Arc<SquadPrs>, PrRouteError> {
    cache
        .get_or_stale()
        .map_err(|message| route_error(ApiStatus::Upstream, message))
}

/// Ticket key to current phase for every flow that has not reached a terminal status.
pub fn active_phases(flows: Option<&FlowEngine>) -> BTreeMap<String, String> {
    flows
        .map(|engine| {
            engine
                .list(None, None)
                .into_iter()
                .filter(|entry| !entry.status.is_terminal())
                .map(|entry| {
                    let phase = entry
                        .current
                        .unwrap_or_else(|| entry.status.as_wire().to_string());
                    (entry.key, phase)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The flow attached to a `feat/<KEY>` branch when that ticket has an active flow.
pub fn flow_ref(phases: &BTreeMap<String, String>, head_ref: &str) -> Option<PrFlowRef> {
    let key = head_ref.strip_prefix(BRANCH_PREFIX)?;
    phases.get(key).map(|phase| PrFlowRef {
        key: key.to_string(),
        phase: phase.clone(),
    })
}

/// `GET /api/prs`: cached rows filtered by `query`.
pub fn handle_list(
    cache: &SquadPrsCache,
    marks: &PrMarks,
    query: &SquadPrsQuery,
    flows: Option<&FlowEngine>,
) -> Result<Value, PrRouteError> {
    let data = listed_data(cache)?;
    let phases = active_phases(flows);
    let response = build_response(&data, &marks.all(), query, &load_config(), |row| {
        flow_ref(&phases, &row.head_ref)
    });
    serde_json::to_value(response).map_err(|error| route_error(ApiStatus::Internal, error))
}

/// `POST /api/prs/refresh`: fetches again (or waits for the fetch already running), tells
/// subscribers, then records new attention events. A failed fetch lists the cached inbox.
pub fn handle_refresh(
    cache: &SquadPrsCache,
    marks: &PrMarks,
    attention: &PrAttention,
    bus: &Bus,
    flows: Option<&FlowEngine>,
) -> Result<Value, PrRouteError> {
    match cache.refresh_now() {
        Ok(_) => cache.notify_changed(bus),
        Err(message) => tracing::warn!(%message, "squad pull requests refresh failed"),
    }
    let listed = handle_list(cache, marks, &SquadPrsQuery::default(), flows);
    refresh_attention(cache, marks, attention, bus);
    listed
}

/// Records attention events from the cached fetch; a failed fetch changes nothing.
pub fn refresh_attention(
    cache: &SquadPrsCache,
    marks: &PrMarks,
    attention: &PrAttention,
    bus: &Bus,
) {
    let Ok(data) = cache.get() else {
        return;
    };
    let squad = load_config();
    let config = load_notifications_pr();
    let context = AttentionContext {
        config: &config,
        bot_authors: &squad.bot_authors,
    };
    process_attention(&data, &marks.all(), attention, bus, &context);
}

fn checks_verdict_of(state: Option<StatusContextState>) -> ChecksVerdict {
    match state {
        Some(StatusContextState::Success) => ChecksVerdict::Passed,
        Some(StatusContextState::Pending | StatusContextState::Expected) => ChecksVerdict::Pending,
        Some(StatusContextState::Error | StatusContextState::Failure) => ChecksVerdict::Failed,
        Some(StatusContextState::Unknown) | None => ChecksVerdict::None,
    }
}

fn gate_ok(pr: &SquadPr) -> bool {
    merge_gate(&GateFacts {
        state: PrState::Open,
        is_draft: pr.is_draft,
        review_decision: pr.review_decision.unwrap_or(ReviewDecision::Unknown),
        reviews_degraded: false,
        checks_verdict: checks_verdict_of(pr.checks_state),
        checks_degraded: false,
        mergeable: pr.mergeable,
        unresolved_threads: pr.unresolved_threads,
        require_thread_resolution: false,
        review_decision_from_github: pr.review_decision.is_some(),
        required_approvals: None,
        approving_reviewers: 0,
    })
    .ok
}

/// The latest of the viewer's seen mark and the viewer's own newest comment.
fn last_seen_of(pr: &SquadPr, me: &str, marks: &BTreeMap<String, PrMark>) -> Option<String> {
    let seen = marks
        .get(&mark_key(&pr.repo, pr.number))
        .and_then(|mark| mark.seen_at.clone());
    let own_comment = pr
        .recent_comments
        .iter()
        .filter(|comment| comment.author == me)
        .map(|comment| comment.created_at.clone())
        .max();
    seen.into_iter().chain(own_comment).max()
}

fn facts_of(
    pr: &SquadPr,
    me: &str,
    marks: &BTreeMap<String, PrMark>,
    context: &AttentionContext,
) -> AttentionFacts {
    let changes_requested = pr.review_decision == Some(ReviewDecision::ChangesRequested);
    AttentionFacts {
        repo: pr.repo.clone(),
        number: pr.number,
        head_sha: pr.head_sha.clone(),
        me: me.to_string(),
        author: pr.author.clone(),
        last_seen: last_seen_of(pr, me, marks),
        comments: pr.recent_comments.clone(),
        bot_authors: context.bot_authors.to_vec(),
        changes_requested_by: changes_requested
            .then(|| CHANGES_REQUESTED_DETAIL.to_string())
            .into_iter()
            .collect(),
        review_requests: pr.review_requests.clone(),
        conflict: pr.mergeable == Mergeable::Conflicting,
        gate_ok: gate_ok(pr),
    }
}

fn concerns_viewer(pr: &SquadPr, me: &str) -> bool {
    pr.author == me || pr.review_requests.iter().any(|login| login == me)
}

fn already_evaluated(store: &PrAttention, pr: &SquadPr) -> bool {
    store
        .list(false)
        .iter()
        .any(|record| record.repo == pr.repo && record.number == pr.number)
}

fn publish_attention(bus: &Bus, record: &PrAttentionRecord) {
    let event = AttentionEvent {
        fingerprint: record.fingerprint.clone(),
        repo: record.repo.clone(),
        number: record.number,
        signal: record.signal,
        first_seen: record.first_seen.clone(),
    };
    if let Ok(payload) = serde_json::to_value(event) {
        bus.emit(ATTENTION_EVENT, payload);
    }
}

/// Stores the new attention signals of the viewer's PRs and publishes the notifying ones.
/// A PR first seen with no mark and no own comment is stored as read.
pub fn process_attention(
    data: &SquadPrs,
    marks: &BTreeMap<String, PrMark>,
    store: &PrAttention,
    bus: &Bus,
    context: &AttentionContext,
) {
    let now = crate::time::now_iso();
    for pr in data.prs.iter().filter(|pr| concerns_viewer(pr, &data.me)) {
        let facts = facts_of(pr, &data.me, marks, context);
        let baseline = facts.last_seen.is_none() && !already_evaluated(store, pr);
        let computed = signals(&facts, context.config);
        let created = store.upsert_new(&computed, &pr.repo, pr.number, &pr.head_sha, &now);
        if baseline {
            let fingerprints = created
                .iter()
                .map(|record| record.fingerprint.clone())
                .collect();
            store.mark_read(&AttentionReadRequest {
                fingerprints: Some(fingerprints),
                all: None,
            });
            continue;
        }
        for record in created.iter().filter(|record| record.notified) {
            publish_attention(bus, record);
        }
    }
}

fn head_sha_of(cache: &SquadPrsCache, repo: &str, number: u64) -> Result<String, PrRouteError> {
    cached_data(cache)?
        .prs
        .iter()
        .find(|pr| pr.repo == repo && pr.number == number)
        .map(|pr| pr.head_sha.clone())
        .ok_or_else(|| route_error(ApiStatus::NotFound, MESSAGE_NOT_IN_INBOX))
}

fn apply_seen(mark: &mut PrMark, seen: Option<bool>, head_sha: Option<String>) {
    match (seen, head_sha) {
        (Some(true), Some(sha)) => {
            mark.seen_sha = Some(sha);
            mark.seen_at = Some(crate::time::now_iso());
        }
        (Some(false), _) => {
            mark.seen_sha = None;
            mark.seen_at = None;
        }
        _ => {}
    }
}

/// `POST /api/prs/marks`: stores the seen mark (at the current head sha) and the ignored flag.
pub fn handle_mark(
    cache: &SquadPrsCache,
    marks: &PrMarks,
    bus: &Bus,
    request: PrMarkRequest,
) -> Result<Value, PrRouteError> {
    let head_sha = match request.seen {
        Some(true) => Some(head_sha_of(cache, &request.repo, request.number)?),
        _ => None,
    };
    let key = mark_key(&request.repo, request.number);
    let updated = marks
        .update(&key, |mark| {
            apply_seen(mark, request.seen, head_sha);
            if let Some(ignored) = request.ignored {
                mark.ignored = ignored;
            }
        })
        .map_err(|error| route_error(ApiStatus::Io, error))?;
    cache.notify_changed(bus);
    serde_json::to_value(PrMarkResponse {
        repo: request.repo,
        number: request.number,
        mark: updated,
    })
    .map_err(|error| route_error(ApiStatus::Io, error))
}

#[cfg(test)]
#[path = "../tests/unit/squad_prs_tests.rs"]
mod tests;
