//! Squad inbox cache, filters and marks store.

use super::*;
use crate::pr_marks::{PrMarks, MARKS_FILE};
use aidp_integrations::models::notifications_pr_config::NotificationsPrConfig;
use aidp_integrations::models::squad_comment::SquadComment;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::{Duration, Instant};

const ROWS_JSON: &str = include_str!("fixtures/squad_prs/rows.json");
const REVIEW_DECISION_ROWS_JSON: &str =
    include_str!("fixtures/squad_prs/rows_review_decisions.json");
const NUMBER_NO_DECISION: u64 = 1;
const NUMBER_REVIEW_REQUIRED: u64 = 2;
const NUMBER_CHANGES_REQUESTED: u64 = 3;
const GATE_OPEN: bool = true;
const GATE_CLOSED: bool = false;
const START_MS: i64 = 1_000;
const WITHIN_TTL_MS: i64 = 119_000;
const BEYOND_TTL_MS: i64 = 121_000;
const REPO_SVC: &str = "acme/svc";
const REPO_API: &str = "acme/api";
const NUMBER_FIRST_REVIEW: u64 = 1;
const BUS_TYPE_KEY: &str = "type";
const LARGE_FIXTURE_SIZE: u64 = 200;
const LATENCY_BUDGET_MS: u128 = 50;
const HEAD_SHA_ONE: &str = "sha1";
const MOVED_HEAD_SHA: &str = "moved";
const FIXTURE_ROW_COUNT: u64 = 5;
const CONCURRENT_READERS: usize = 4;
const GATE_TIMEOUT: Duration = Duration::from_secs(5);

struct TickingClock(Arc<AtomicI64>);

impl Clock for TickingClock {
    fn now_utc(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(self.0.load(Ordering::SeqCst)).unwrap()
    }
    fn now_rfc3339(&self) -> String {
        self.now_utc().to_rfc3339()
    }
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct CountingFetcher {
    calls: Arc<AtomicU32>,
    data: SquadPrs,
    /// Whether each call received the previous fetch, in call order.
    previous_seen: Arc<Mutex<Vec<bool>>>,
}

impl SquadFetcher for CountingFetcher {
    fn fetch(&self, previous: Option<&SquadPrs>) -> Result<SquadPrs, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        lock(&self.previous_seen).push(previous.is_some());
        Ok(self.data.clone())
    }
}

fn fixture() -> SquadPrs {
    serde_json::from_str(ROWS_JSON).unwrap()
}

fn cache_over(data: SquadPrs, clock: Box<dyn Clock>) -> (SquadPrsCache, Arc<AtomicU32>) {
    let calls = Arc::new(AtomicU32::new(0));
    let fetcher = CountingFetcher {
        calls: calls.clone(),
        data,
        previous_seen: Arc::default(),
    };
    (SquadPrsCache::new(clock, Box::new(fetcher)), calls)
}

fn cache_with_time() -> (SquadPrsCache, Arc<AtomicI64>, Arc<AtomicU32>) {
    let time = Arc::new(AtomicI64::new(START_MS));
    let (cache, calls) = cache_over(fixture(), Box::new(TickingClock(time.clone())));
    (cache, time, calls)
}

fn respond(query: &SquadPrsQuery, marks: &BTreeMap<String, PrMark>) -> SquadPrsResponse {
    build_response(&fixture(), marks, query, &SquadConfig::default(), |_| None)
}

fn numbers(response: &SquadPrsResponse) -> Vec<u64> {
    let mut found: Vec<u64> = response.prs.iter().map(|row| row.number).collect();
    found.sort();
    found
}

fn query_of(pairs: &[(&str, &str)]) -> SquadPrsQuery {
    SquadPrsQuery::from_lookup(|key| {
        pairs
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.to_string())
    })
    .unwrap()
}

fn first_review_key() -> String {
    mark_key(REPO_SVC, NUMBER_FIRST_REVIEW)
}

#[test]
fn two_gets_within_ttl_fetch_once() {
    let (cache, time, calls) = cache_with_time();
    cache.get().unwrap();
    time.fetch_add(WITHIN_TTL_MS, Ordering::SeqCst);
    cache.get().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn get_past_ttl_fetches_again() {
    let (cache, time, calls) = cache_with_time();
    cache.get().unwrap();
    time.fetch_add(BEYOND_TTL_MS, Ordering::SeqCst);
    cache.get().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_stale_entry_is_listed_at_once_then_refreshed_once() {
    let (cache, time, calls) = cache_with_time();
    cache.get().unwrap();
    assert!(!cache.is_stale());
    time.fetch_add(BEYOND_TTL_MS, Ordering::SeqCst);
    assert!(cache.is_stale());
    cache.get_or_stale().unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a stale list must not fetch"
    );
    assert!(cache.refresh_once());
    assert!(!cache.is_stale());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// Waits for the test's go-ahead before answering, so an invalidation can land mid-fetch.
struct GatedFetcher {
    started: std::sync::mpsc::Sender<()>,
    proceed: Mutex<std::sync::mpsc::Receiver<()>>,
    data: SquadPrs,
}

impl SquadFetcher for GatedFetcher {
    fn fetch(&self, _previous: Option<&SquadPrs>) -> Result<SquadPrs, String> {
        let _ = self.started.send(());
        let _ = lock(&self.proceed).recv();
        Ok(self.data.clone())
    }
}

/// Fails by panicking, as a broken `gh` parser would.
struct PanickingFetcher;

impl SquadFetcher for PanickingFetcher {
    fn fetch(&self, _previous: Option<&SquadPrs>) -> Result<SquadPrs, String> {
        panic!("fetch exploded")
    }
}

#[test]
fn an_invalidation_during_a_refresh_leaves_its_result_stale() {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (proceed_tx, proceed_rx) = std::sync::mpsc::channel();
    let time = Arc::new(AtomicI64::new(START_MS));
    let cache = Arc::new(SquadPrsCache::new(
        Box::new(TickingClock(time)),
        Box::new(GatedFetcher {
            started: started_tx,
            proceed: Mutex::new(proceed_rx),
            data: fixture(),
        }),
    ));
    let refreshing = Arc::clone(&cache);
    let refresh = std::thread::spawn(move || refreshing.refresh_once());
    started_rx.recv().unwrap();
    cache.invalidate(&Bus::new());
    proceed_tx.send(()).unwrap();
    assert!(
        !refresh.join().unwrap(),
        "a pre-invalidation fetch is not a current result"
    );
    assert!(cache.is_stale(), "the next list refreshes again");
    assert!(
        cache.get_or_stale().is_ok(),
        "the result still answers at once"
    );
}

#[test]
fn concurrent_readers_of_an_empty_cache_share_one_fetch() {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (proceed_tx, proceed_rx) = std::sync::mpsc::channel();
    let cache = Arc::new(SquadPrsCache::new(
        Box::new(TickingClock(Arc::new(AtomicI64::new(START_MS)))),
        Box::new(GatedFetcher {
            started: started_tx,
            proceed: Mutex::new(proceed_rx),
            data: fixture(),
        }),
    ));
    let readers: Vec<_> = (0..CONCURRENT_READERS)
        .map(|_| {
            let cache = Arc::clone(&cache);
            std::thread::spawn(move || cache.get().map(|data| data.prs.len()))
        })
        .collect();
    started_rx.recv_timeout(GATE_TIMEOUT).unwrap();
    proceed_tx.send(()).unwrap();
    drop(proceed_tx);
    for reader in readers {
        assert_eq!(reader.join().unwrap().unwrap() as u64, FIXTURE_ROW_COUNT);
    }
    assert!(
        started_rx.try_recv().is_err(),
        "the waiting readers reuse the one fetch"
    );
}

#[test]
fn a_later_fetch_receives_the_previous_one() {
    let time = Arc::new(AtomicI64::new(START_MS));
    let previous_seen: Arc<Mutex<Vec<bool>>> = Arc::default();
    let cache = SquadPrsCache::new(
        Box::new(TickingClock(time.clone())),
        Box::new(CountingFetcher {
            calls: Arc::default(),
            data: fixture(),
            previous_seen: previous_seen.clone(),
        }),
    );
    cache.get().unwrap();
    time.fetch_add(BEYOND_TTL_MS, Ordering::SeqCst);
    cache.get().unwrap();
    assert_eq!(*lock(&previous_seen), vec![false, true]);
}

#[test]
fn a_persisted_cache_serves_the_saved_fetch_at_once_then_refreshes_from_it() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join(SQUAD_SNAPSHOT_FILE);
    let time = Arc::new(AtomicI64::new(START_MS));
    let first_calls = Arc::new(AtomicU32::new(0));
    let first = SquadPrsCache::persisted(
        Box::new(TickingClock(time.clone())),
        Box::new(CountingFetcher {
            calls: first_calls,
            data: fixture(),
            previous_seen: Arc::default(),
        }),
        snapshot.clone(),
    );
    first.get().unwrap();
    let calls = Arc::new(AtomicU32::new(0));
    let previous_seen: Arc<Mutex<Vec<bool>>> = Arc::default();
    let restarted = SquadPrsCache::persisted(
        Box::new(TickingClock(time)),
        Box::new(CountingFetcher {
            calls: calls.clone(),
            data: fixture(),
            previous_seen: previous_seen.clone(),
        }),
        snapshot,
    );
    assert!(restarted.is_stale());
    assert_eq!(
        restarted.get_or_stale().unwrap().prs.len() as u64,
        FIXTURE_ROW_COUNT
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the saved fetch answers without GitHub"
    );
    assert!(restarted.refresh_once());
    assert_eq!(*lock(&previous_seen), vec![true]);
}

#[test]
fn a_panicking_refresh_releases_the_refresh_flag() {
    let cache = SquadPrsCache::new(
        Box::new(TickingClock(Arc::new(AtomicI64::new(START_MS)))),
        Box::new(PanickingFetcher),
    );
    let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cache.refresh_once()));
    assert!(first.is_err());
    let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cache.refresh_once()));
    assert!(
        second.is_err(),
        "the second refresh runs instead of being skipped"
    );
}

#[test]
fn an_empty_cache_fetches_even_when_stale_answers_are_allowed() {
    let (cache, _, calls) = cache_with_time();
    assert!(!cache.is_stale());
    cache.get_or_stale().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn fixed_clock_keeps_the_entry_fresh() {
    let clock = aidp_core::clock::FixedClock::from_rfc3339("2026-10-03T09:00:00Z");
    let (cache, calls) = cache_over(fixture(), Box::new(clock));
    cache.get().unwrap();
    cache.get().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn invalidate_keeps_the_entry_forces_a_fetch_and_publishes_the_event() {
    let (cache, _, calls) = cache_with_time();
    let bus = Bus::new();
    let receiver = bus.subscribe();
    cache.get().unwrap();
    cache.invalidate(&bus);
    assert!(cache.is_stale());
    cache.get_or_stale().unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a stale list answers from the kept entry"
    );
    cache.get().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let event = receiver.try_recv().unwrap();
    assert_eq!(event[BUS_TYPE_KEY], INBOX_CHANGED_EVENT);
}

#[test]
fn default_query_hides_bots() {
    let response = respond(&SquadPrsQuery::default(), &BTreeMap::new());
    assert_eq!(numbers(&response), vec![1, 2, 4, 5]);
}

#[test]
fn lane_filter_keeps_counts_of_every_lane() {
    let response = respond(&query_of(&[("lane", "first_review")]), &BTreeMap::new());
    assert_eq!(numbers(&response), vec![1]);
    for lane in [
        "drafts",
        "re_review",
        "first_review",
        "mine",
        "needs_action",
    ] {
        assert_eq!(response.lanes[lane], 1, "lane {lane}");
    }
}

#[test]
fn repo_and_author_filters_narrow_rows() {
    let by_repo = respond(&query_of(&[("repo", REPO_API)]), &BTreeMap::new());
    assert_eq!(numbers(&by_repo), vec![4]);
    let by_author = respond(&query_of(&[("author", "CAROL")]), &BTreeMap::new());
    assert_eq!(numbers(&by_author), vec![4, 5]);
}

#[test]
fn bots_filter_modes() {
    let shown = respond(&query_of(&[("bots", "show")]), &BTreeMap::new());
    assert_eq!(numbers(&shown), vec![1, 2, 3, 4, 5]);
    let only = respond(&query_of(&[("bots", "only")]), &BTreeMap::new());
    assert_eq!(numbers(&only), vec![3]);
}

#[test]
fn text_filter_matches_title_case_insensitively() {
    let response = respond(&query_of(&[("q", "FIX CACHE")]), &BTreeMap::new());
    assert_eq!(numbers(&response), vec![4]);
}

#[test]
fn unknown_lane_value_is_rejected() {
    let result = SquadPrsQuery::from_lookup(|key| (key == "lane").then(|| "nope".to_string()));
    assert!(result.is_err());
}

#[test]
fn ignored_mark_removes_the_row() {
    let mut marks = BTreeMap::new();
    marks.insert(
        first_review_key(),
        PrMark {
            ignored: true,
            ..PrMark::default()
        },
    );
    let response = respond(&SquadPrsQuery::default(), &marks);
    assert_eq!(numbers(&response), vec![2, 4, 5]);
}

#[test]
fn missing_marks_file_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(PrMarks::new(dir.path()).all().is_empty());
}

#[test]
fn corrupt_marks_file_is_empty_without_panic() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(MARKS_FILE), "{not json").unwrap();
    assert!(PrMarks::new(dir.path()).all().is_empty());
}

#[test]
fn seen_mark_resets_when_the_head_moves() {
    let dir = tempfile::tempdir().unwrap();
    let marks = PrMarks::new(dir.path());
    marks
        .update(&first_review_key(), |mark| {
            mark.seen_sha = Some(HEAD_SHA_ONE.to_string())
        })
        .unwrap();
    let stored = marks.all();
    let before = respond(&SquadPrsQuery::default(), &stored);
    assert!(before.prs.iter().find(|row| row.number == 1).unwrap().seen);
    let mut moved = fixture();
    moved.prs[0].head_sha = MOVED_HEAD_SHA.to_string();
    let after = build_response(
        &moved,
        &stored,
        &SquadPrsQuery::default(),
        &SquadConfig::default(),
        |_| None,
    );
    assert!(!after.prs.iter().find(|row| row.number == 1).unwrap().seen);
}

#[test]
fn ignored_mark_persists_across_reload() {
    let dir = tempfile::tempdir().unwrap();
    PrMarks::new(dir.path())
        .update(&first_review_key(), |mark| mark.ignored = true)
        .unwrap();
    assert!(PrMarks::new(dir.path()).all()[&first_review_key()].ignored);
}

#[test]
fn write_leaves_no_temp_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let marks = PrMarks::new(dir.path());
    marks
        .update("acme/svc#1", |mark| mark.ignored = true)
        .unwrap();
    marks
        .update("acme/svc#2", |mark| mark.ignored = true)
        .unwrap();
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec![MARKS_FILE.to_string()]);
}

#[test]
fn cached_get_and_filtering_of_200_prs_stays_under_budget() {
    let template = fixture();
    let mut large = template.clone();
    large.prs = (0..LARGE_FIXTURE_SIZE)
        .map(|index| {
            let mut pr = template.prs[(index % FIXTURE_ROW_COUNT) as usize].clone();
            pr.number = 1_000 + index;
            pr
        })
        .collect();
    let clock = TickingClock(Arc::new(AtomicI64::new(START_MS)));
    let (cache, calls) = cache_over(large, Box::new(clock));
    cache.get().unwrap();
    let query = query_of(&[("lane", "first_review"), ("q", "add")]);
    let started = Instant::now();
    let data = cache.get().unwrap();
    let response = build_response(
        &data,
        &BTreeMap::new(),
        &query,
        &SquadConfig::default(),
        |_| None,
    );
    assert!(started.elapsed().as_millis() < LATENCY_BUDGET_MS);
    assert!(!response.prs.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

const FLOW_KEY: &str = "LOC-9";
const FLOW_PHASE: &str = "implement";
const PLAIN_BRANCH: &str = "main";
const OTHER_KEY: &str = "OTHER-1";
const MARK_KEYS: [&str; 3] = ["mark", "number", "repo"];

/// A feat branch whose ticket has an active flow carries the flow ref.
#[test]
fn flow_ref_maps_active_ticket_branch() {
    let phases = BTreeMap::from([(FLOW_KEY.to_string(), FLOW_PHASE.to_string())]);
    let found = flow_ref(&phases, &format!("{BRANCH_PREFIX}{FLOW_KEY}"));
    assert_eq!(
        found,
        Some(PrFlowRef {
            key: FLOW_KEY.to_string(),
            phase: FLOW_PHASE.to_string()
        })
    );
}

/// A branch without the prefix, or a ticket with no active flow, has no flow ref.
#[test]
fn flow_ref_is_none_without_prefix_or_flow() {
    let phases = BTreeMap::from([(FLOW_KEY.to_string(), FLOW_PHASE.to_string())]);
    assert_eq!(flow_ref(&phases, PLAIN_BRANCH), None);
    assert_eq!(
        flow_ref(&phases, &format!("{BRANCH_PREFIX}{OTHER_KEY}")),
        None
    );
    assert!(active_phases(None).is_empty());
}

/// The mark response keeps the repo, number and mark keys.
#[test]
fn mark_response_json_shape_is_stable() {
    let value = serde_json::to_value(PrMarkResponse {
        repo: REPO_SVC.to_string(),
        number: NUMBER_FIRST_REVIEW,
        mark: PrMark::default(),
    })
    .unwrap();
    let mut keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, MARK_KEYS);
}

/// The inbox-changed event payload stays an empty object.
#[test]
fn inbox_changed_event_is_an_empty_object() {
    let value = serde_json::to_value(InboxChangedEvent {}).unwrap();
    assert_eq!(value, serde_json::Value::Object(Default::default()));
}

const ME: &str = "alice";
const COLLEAGUE: &str = "bob";
const OTHER_COLLEAGUE: &str = "carol";
const ATTENTION_REPO: &str = "acme/svc";
const ATTENTION_NUMBER: u64 = 42;
const ATTENTION_SHA: &str = "sha42";
const BEFORE_COMMENTS: &str = "2026-10-01T00:00:00Z";
const COMMENT_ONE_AT: &str = "2026-10-02T08:00:00Z";
const COMMENT_TWO_AT: &str = "2026-10-02T09:00:00Z";
const AFTER_COMMENTS: &str = "2026-10-02T10:00:00Z";
const COMMENT_THREE_AT: &str = "2026-10-03T08:00:00Z";

fn comment(id: &str, author: &str, created_at: &str) -> SquadComment {
    SquadComment {
        id: id.to_string(),
        author: author.to_string(),
        created_at: created_at.to_string(),
        body_marker: false,
    }
}

fn own_pr(comments: Vec<SquadComment>) -> SquadPr {
    SquadPr {
        repo: ATTENTION_REPO.to_string(),
        number: ATTENTION_NUMBER,
        title: "own".to_string(),
        url: String::new(),
        is_draft: false,
        author: ME.to_string(),
        created_at: BEFORE_COMMENTS.to_string(),
        updated_at: BEFORE_COMMENTS.to_string(),
        base_ref: "master".to_string(),
        head_ref: "feat/own".to_string(),
        head_sha: ATTENTION_SHA.to_string(),
        mergeable: Mergeable::Unknown,
        review_decision: Some(ReviewDecision::ReviewRequired),
        review_requests: Vec::new(),
        my_reviews: Vec::new(),
        commits: 1,
        checks_state: None,
        unresolved_threads: 0,
        comments: comments.len() as u64,
        labels: Vec::new(),
        recent_comments: comments,
    }
}

fn squad_of(pr: SquadPr) -> SquadPrs {
    SquadPrs {
        fetched_at: AFTER_COMMENTS.to_string(),
        me: ME.to_string(),
        repos: vec![ATTENTION_REPO.to_string()],
        degraded: Vec::new(),
        prs: vec![pr],
    }
}

fn seen_at(at: &str) -> BTreeMap<String, PrMark> {
    let mark = PrMark {
        seen_sha: Some(ATTENTION_SHA.to_string()),
        seen_at: Some(at.to_string()),
        ignored: false,
        ..PrMark::default()
    };
    BTreeMap::from([(mark_key(ATTENTION_REPO, ATTENTION_NUMBER), mark)])
}

struct AttentionRun<'a> {
    store: &'a PrAttention,
    bus: &'a Bus,
    config: NotificationsPrConfig,
}

impl AttentionRun<'_> {
    fn pass(&self, data: &SquadPrs, marks: &BTreeMap<String, PrMark>) {
        let context = AttentionContext {
            config: &self.config,
            bot_authors: &[],
        };
        process_attention(data, marks, self.store, self.bus, &context);
    }
}

fn attention_events(bus: &Bus) -> usize {
    bus.recent(500, 0)
        .iter()
        .filter(|event| event[BUS_TYPE_KEY] == ATTENTION_EVENT)
        .count()
}

fn run_over<'a>(store: &'a PrAttention, bus: &'a Bus) -> AttentionRun<'a> {
    AttentionRun {
        store,
        bus,
        config: NotificationsPrConfig::default(),
    }
}

#[test]
fn two_colleague_comments_give_two_records() {
    let dir = tempfile::tempdir().unwrap();
    let (store, bus) = (PrAttention::new(dir.path()), Bus::new());
    let comments = vec![
        comment("c1", COLLEAGUE, COMMENT_ONE_AT),
        comment("c2", OTHER_COLLEAGUE, COMMENT_TWO_AT),
    ];
    run_over(&store, &bus).pass(&squad_of(own_pr(comments)), &seen_at(BEFORE_COMMENTS));
    assert_eq!(store.list(false).len(), 2);
    assert_eq!(attention_events(&bus), 2);
}

#[test]
fn seen_mark_silences_until_the_next_comment() {
    let dir = tempfile::tempdir().unwrap();
    let (store, bus) = (PrAttention::new(dir.path()), Bus::new());
    let run = run_over(&store, &bus);
    let mut comments = vec![
        comment("c1", COLLEAGUE, COMMENT_ONE_AT),
        comment("c2", OTHER_COLLEAGUE, COMMENT_TWO_AT),
    ];
    run.pass(
        &squad_of(own_pr(comments.clone())),
        &seen_at(BEFORE_COMMENTS),
    );
    run.pass(
        &squad_of(own_pr(comments.clone())),
        &seen_at(AFTER_COMMENTS),
    );
    assert_eq!(store.list(false).len(), 2);
    comments.push(comment("c3", COLLEAGUE, COMMENT_THREE_AT));
    run.pass(&squad_of(own_pr(comments)), &seen_at(AFTER_COMMENTS));
    assert_eq!(store.list(false).len(), 3);
    assert_eq!(attention_events(&bus), 3);
}

#[test]
fn mergeable_fires_once_across_two_refreshes() {
    let dir = tempfile::tempdir().unwrap();
    let (store, bus) = (PrAttention::new(dir.path()), Bus::new());
    let run = run_over(&store, &bus);
    let mut pr = own_pr(Vec::new());
    pr.review_decision = Some(ReviewDecision::Approved);
    pr.checks_state = Some(StatusContextState::Success);
    pr.mergeable = Mergeable::Mergeable;
    let data = squad_of(pr);
    run.pass(&data, &seen_at(BEFORE_COMMENTS));
    run.pass(&data, &seen_at(BEFORE_COMMENTS));
    assert_eq!(store.list(false).len(), 1);
    assert_eq!(attention_events(&bus), 1);
}

#[test]
fn disabled_kind_adds_nothing_and_keeps_old_records() {
    let dir = tempfile::tempdir().unwrap();
    let (store, bus) = (PrAttention::new(dir.path()), Bus::new());
    let mut run = run_over(&store, &bus);
    let mut comments = vec![comment("c1", COLLEAGUE, COMMENT_ONE_AT)];
    run.pass(
        &squad_of(own_pr(comments.clone())),
        &seen_at(BEFORE_COMMENTS),
    );
    run.config.new_comment = false;
    comments.push(comment("c2", COLLEAGUE, COMMENT_TWO_AT));
    run.pass(&squad_of(own_pr(comments)), &seen_at(BEFORE_COMMENTS));
    assert_eq!(store.list(false).len(), 1);
    assert_eq!(attention_events(&bus), 1);
}

#[test]
fn first_evaluation_without_marks_is_stored_as_read() {
    let dir = tempfile::tempdir().unwrap();
    let (store, bus) = (PrAttention::new(dir.path()), Bus::new());
    let comments = vec![comment("c1", COLLEAGUE, COMMENT_ONE_AT)];
    run_over(&store, &bus).pass(&squad_of(own_pr(comments)), &BTreeMap::new());
    let records = store.list(false);
    assert_eq!(records.len(), 1);
    assert!(records[0].read);
    assert_eq!(attention_events(&bus), 0);
}

fn gate_of(rows: &SquadPrs, number: u64) -> bool {
    let pr = rows.prs.iter().find(|row| row.number == number).unwrap();
    gate_ok(pr)
}

#[test]
fn gate_ok_follows_the_github_review_decision() {
    let rows: SquadPrs = serde_json::from_str(REVIEW_DECISION_ROWS_JSON).unwrap();
    assert_eq!(gate_of(&rows, NUMBER_NO_DECISION), GATE_OPEN);
    assert_eq!(gate_of(&rows, NUMBER_REVIEW_REQUIRED), GATE_CLOSED);
    assert_eq!(gate_of(&rows, NUMBER_CHANGES_REQUESTED), GATE_CLOSED);
}
