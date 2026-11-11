use super::*;
use crate::models::constants::pixellab::USAGE_FILE;
use crate::models::constants::vault_index::SCHEMA_VERSION;
use crate::pixellab_usage::{aggregate_ticket, scan_latest_balance};
use crate::ticket_cost::load_stage_rows;
use aidp_testkit::TempWorkspace;
use serde_json::json;
use std::time::Duration;

const REPO: &str = "Repo";
const OTHER_REPO: &str = "Other";
const TICKET: &str = "GH-1";
const OTHER_TICKET: &str = "GH-2";
const MISSING_TICKET: &str = "GH-404";
const OLD_RUN: &str = "20261109-000000-aaaa";
const NEW_RUN: &str = "20261110-000000-bbbb";
const NEWEST_RUN: &str = "20261111-000000-cccc";
const ART_PIPELINE: &str = "art-produce";
const CODE_PIPELINE: &str = "dev";
const ROUTE_ART: &str = "art";
const ROUTE_CODE: &str = "code";
const STAGE: &str = "implement";
const ART_STAGE: &str = "generate";
const STATE_FILE: &str = "run-state.json";
const WAIT_LIMIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);
const BIG_TOKENS: u64 = 123_456;

fn run_dir(vault: &Path, repo: &str, ticket: &str, run: &str) -> PathBuf {
    vault.join(PIPELINES_DIR).join(repo).join(ticket).join(run)
}

fn write_state(vault: &Path, repo: &str, ticket: &str, run: &str, pipeline: &str, tokens: u64) {
    let dir = run_dir(vault, repo, ticket, run);
    std::fs::create_dir_all(&dir).unwrap();
    let state = json!({
        "run_id": run,
        "pipeline_path": "p.yaml",
        "pipeline_id": pipeline,
        "workspace_root": "w",
        "run_dir": dir,
        "started_at": "2026-11-10T10:00:00Z",
        "inputs": { "feature": ticket },
        "plan": [],
        "stages": [{
            "name": STAGE,
            "status": "success",
            "stage_dir": dir.join(STAGE),
            "usage": {
                "input": tokens, "output": 0, "cache_creation": 0, "cache_read": 0,
                "bucket": "small", "source": "jsonl", "model": "claude-sonnet-5"
            }
        }],
    });
    std::fs::write(dir.join(STATE_FILE), state.to_string()).unwrap();
}

fn write_usage(
    vault: &Path,
    repo: &str,
    ticket: &str,
    run: &str,
    generations: u32,
    balance: f64,
    at: &str,
) {
    let dir = run_dir(vault, repo, ticket, run).join(ART_STAGE);
    std::fs::create_dir_all(&dir).unwrap();
    let body = json!({
        "balanceBefore": { "credits": balance + 1.0 },
        "balanceAfter": { "credits": balance },
        "generations": generations,
        "jobs": ["job"],
        "recordedAt": at,
    });
    std::fs::write(dir.join(USAGE_FILE), body.to_string()).unwrap();
}

fn seeded_vault() -> TempWorkspace {
    let vault = TempWorkspace::new("vault-index");
    write_state(vault.path(), REPO, TICKET, OLD_RUN, CODE_PIPELINE, 10);
    write_state(vault.path(), REPO, TICKET, NEW_RUN, ART_PIPELINE, 20);
    write_state(vault.path(), OTHER_REPO, TICKET, NEW_RUN, ART_PIPELINE, 5);
    write_state(vault.path(), REPO, OTHER_TICKET, OLD_RUN, CODE_PIPELINE, 1);
    write_usage(
        vault.path(),
        REPO,
        TICKET,
        NEW_RUN,
        3,
        40.0,
        "2026-11-10T10:00:00Z",
    );
    write_usage(
        vault.path(),
        REPO,
        OTHER_TICKET,
        OLD_RUN,
        2,
        35.0,
        "2026-11-10T12:00:00Z",
    );
    vault
}

fn pick(index: &VaultSnapshot, run: &str) -> Arc<RunSummary> {
    index
        .runs_of(TICKET)
        .iter()
        .find(|summary| summary.run_id == run && summary.repo == REPO)
        .cloned()
        .unwrap()
}

fn wait_for_runs(vault: &Path, count: usize) {
    let deadline = std::time::Instant::now() + WAIT_LIMIT;
    while snapshot(vault).run_count() < count && std::time::Instant::now() < deadline {
        std::thread::sleep(POLL);
    }
}

#[test]
fn a_ticket_lists_its_runs_newest_first_across_repos() {
    let vault = seeded_vault();
    let index = scan(vault.path(), None);
    let runs: Vec<(&str, &str)> = index
        .runs_of(TICKET)
        .iter()
        .map(|run| (run.run_id.as_str(), run.repo.as_str()))
        .collect();
    assert_eq!(
        runs,
        vec![(NEW_RUN, OTHER_REPO), (NEW_RUN, REPO), (OLD_RUN, REPO)]
    );
    assert_eq!(index.run_count(), 4);
    assert!(index.runs_of(MISSING_TICKET).is_empty());
}

#[test]
fn routes_come_from_the_run_state() {
    let vault = seeded_vault();
    let index = scan(vault.path(), None);
    assert_eq!(index.route_of(TICKET).as_deref(), Some(ROUTE_ART));
    assert_eq!(index.route_of(OTHER_TICKET).as_deref(), Some(ROUTE_CODE));
    assert_eq!(
        index.route_of_run(TICKET, OLD_RUN).as_deref(),
        Some(ROUTE_CODE)
    );
    assert_eq!(index.route_of_run(TICKET, "missing"), None);
}

#[test]
fn the_index_answers_what_the_vault_walks_answer() {
    let vault = seeded_vault();
    let index = scan(vault.path(), None);
    for key in [TICKET, OTHER_TICKET] {
        let mut indexed = index.stage_rows(key);
        let mut walked = load_stage_rows(vault.path(), key);
        indexed.sort_by(|a, b| (&a.run_id, a.tokens.input).cmp(&(&b.run_id, b.tokens.input)));
        walked.sort_by(|a, b| (&a.run_id, a.tokens.input).cmp(&(&b.run_id, b.tokens.input)));
        assert_eq!(indexed, walked);
        assert_eq!(index.pixellab_of(key), aggregate_ticket(vault.path(), key));
    }
    assert_eq!(
        index.latest_balance().cloned(),
        scan_latest_balance(vault.path())
    );
    assert_eq!(index.latest_balance().unwrap().run_id, OLD_RUN);
}

#[test]
fn a_refresh_reads_again_only_the_runs_that_changed() {
    let vault = seeded_vault();
    let first = scan(vault.path(), None);
    write_state(
        vault.path(),
        REPO,
        TICKET,
        NEW_RUN,
        ART_PIPELINE,
        BIG_TOKENS,
    );
    let second = scan(vault.path(), Some(&first));
    assert!(Arc::ptr_eq(&pick(&first, OLD_RUN), &pick(&second, OLD_RUN)));
    assert!(!Arc::ptr_eq(
        &pick(&first, NEW_RUN),
        &pick(&second, NEW_RUN)
    ));
    assert_eq!(pick(&second, NEW_RUN).stages[0].tokens.input, BIG_TOKENS);
}

#[test]
fn a_refresh_sees_new_and_removed_runs() {
    let vault = seeded_vault();
    let first = scan(vault.path(), None);
    write_state(vault.path(), REPO, TICKET, NEWEST_RUN, CODE_PIPELINE, 1);
    std::fs::remove_dir_all(run_dir(vault.path(), REPO, OTHER_TICKET, OLD_RUN)).unwrap();
    let second = scan(vault.path(), Some(&first));
    assert_eq!(second.runs_of(TICKET).len(), 4);
    assert!(second.runs_of(OTHER_TICKET).is_empty());
}

#[test]
fn refresh_ticket_reads_a_ticket_again_even_when_its_run_state_did_not_change() {
    let vault = TempWorkspace::new("vault-index-ticket");
    write_state(vault.path(), REPO, TICKET, NEW_RUN, ART_PIPELINE, 1);
    assert_eq!(snapshot(vault.path()).pixellab_of(TICKET).generations, 0);
    write_usage(
        vault.path(),
        REPO,
        TICKET,
        NEW_RUN,
        7,
        10.0,
        "2026-11-10T10:00:00Z",
    );
    let fresh = refresh_ticket(vault.path(), TICKET);
    assert_eq!(fresh.pixellab_of(TICKET).generations, 7);
    assert_eq!(snapshot(vault.path()).pixellab_of(TICKET).generations, 7);
}

#[test]
fn forget_makes_the_next_read_rebuild() {
    let vault = TempWorkspace::new("vault-index-forget");
    write_state(vault.path(), REPO, TICKET, NEW_RUN, ART_PIPELINE, 1);
    assert_eq!(snapshot(vault.path()).run_count(), 1);
    write_state(vault.path(), REPO, TICKET, OLD_RUN, ART_PIPELINE, 1);
    forget(vault.path());
    assert_eq!(snapshot(vault.path()).run_count(), 2);
}

#[test]
fn invalidate_refreshes_in_the_background() {
    let vault = TempWorkspace::new("vault-index-invalidate");
    write_state(vault.path(), REPO, TICKET, NEW_RUN, ART_PIPELINE, 1);
    assert_eq!(snapshot(vault.path()).run_count(), 1);
    write_state(vault.path(), REPO, TICKET, OLD_RUN, ART_PIPELINE, 1);
    invalidate(vault.path());
    wait_for_runs(vault.path(), 2);
    assert_eq!(snapshot(vault.path()).run_count(), 2);
}

#[test]
fn a_vault_without_runs_or_without_a_path_is_empty() {
    let vault = TempWorkspace::new("vault-index-empty");
    assert_eq!(snapshot(vault.path()).run_count(), 0);
    assert!(snapshot(Path::new("")).latest_balance().is_none());
    assert_eq!(refresh_ticket(Path::new(""), TICKET).run_count(), 0);
}

#[test]
fn parallel_map_keeps_the_order_of_its_items() {
    let squares = parallel_map((0..50u32).collect(), |value| value * value);
    assert_eq!(
        squares,
        (0..50u32).map(|value| value * value).collect::<Vec<_>>()
    );
    assert!(parallel_map(Vec::<u32>::new(), |value| value).is_empty());
}

const DB_NAME: &str = "vault-index.sqlite";
const UNKNOWN_SCHEMA_VERSION: u32 = 9999;
const GARBAGE: &[u8] = b"this is not a sqlite database, only text that fills the file";
const USER_VERSION_PRAGMA: &str = "user_version";

fn database_in(dir: &TempWorkspace) -> PathBuf {
    dir.path().join(DB_NAME)
}

fn forget_memory(vault: &Path) {
    *lock(&slot_of(vault).current) = None;
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + WAIT_LIMIT;
    while !condition() && std::time::Instant::now() < deadline {
        std::thread::sleep(POLL);
    }
}

fn stored_runs_of(database: &Path, vault: &Path) -> Vec<StoredRun> {
    IndexStore::open(database).unwrap().load(vault).unwrap()
}

fn stored_big_tokens(database: &Path, vault: &Path) -> bool {
    stored_runs_of(database, vault).iter().any(|entry| {
        entry.run.run_id == NEW_RUN
            && entry.run.repo == REPO
            && entry
                .run
                .stages
                .first()
                .is_some_and(|row| row.tokens.input == BIG_TOKENS)
    })
}

#[test]
fn a_stored_index_loads_without_walking_the_vault() {
    let vault = seeded_vault();
    let dir = TempWorkspace::new("vault-index-db");
    let scanned = scan(vault.path(), None);
    let store = IndexStore::open(&database_in(&dir)).unwrap();
    assert!(!store.is_scanned(vault.path()).unwrap());
    let runs: Vec<StoredRun> = scanned.stored_runs(None).into_values().collect();
    store.replace(vault.path(), &runs).unwrap();
    assert!(store.is_scanned(vault.path()).unwrap());
    std::fs::remove_dir_all(vault.path().join(PIPELINES_DIR)).unwrap();
    let loaded = VaultSnapshot::from_stored(store.load(vault.path()).unwrap());
    assert_eq!(loaded.run_count(), 4);
    for key in [TICKET, OTHER_TICKET] {
        assert_eq!(loaded.stage_rows(key), scanned.stage_rows(key));
        assert_eq!(loaded.pixellab_of(key), scanned.pixellab_of(key));
        assert_eq!(loaded.route_of(key), scanned.route_of(key));
    }
    assert_eq!(loaded.latest_balance(), scanned.latest_balance());
    assert!(loaded.latest_balance().is_some());
    for run in loaded.runs_of(TICKET) {
        let original = scanned
            .runs_of(TICKET)
            .iter()
            .find(|candidate| candidate.run_id == run.run_id && candidate.repo == run.repo)
            .unwrap();
        assert_eq!(run.signature, original.signature);
    }
}

#[test]
fn a_restart_serves_the_database_at_once_then_refreshes_in_the_background() {
    let vault = seeded_vault();
    let dir = TempWorkspace::new("vault-index-restart");
    let database = database_in(&dir);
    use_database(vault.path(), &database);
    assert_eq!(snapshot(vault.path()).run_count(), 4);
    assert_eq!(stored_runs_of(&database, vault.path()).len(), 4);

    forget_memory(vault.path());
    std::fs::remove_dir_all(run_dir(vault.path(), REPO, OTHER_TICKET, OLD_RUN)).unwrap();
    write_state(
        vault.path(),
        REPO,
        TICKET,
        NEW_RUN,
        ART_PIPELINE,
        BIG_TOKENS,
    );
    let at_once = snapshot(vault.path());
    assert_eq!(at_once.run_count(), 4);
    assert_ne!(pick(&at_once, NEW_RUN).stages[0].tokens.input, BIG_TOKENS);

    wait_until(|| {
        stored_runs_of(&database, vault.path()).len() == 3
            && stored_big_tokens(&database, vault.path())
    });
    let refreshed = snapshot(vault.path());
    assert_eq!(refreshed.run_count(), 3);
    assert_eq!(pick(&refreshed, NEW_RUN).stages[0].tokens.input, BIG_TOKENS);
    assert_eq!(stored_runs_of(&database, vault.path()).len(), 3);
    assert!(stored_big_tokens(&database, vault.path()));
}

#[test]
fn a_changed_run_state_is_stored_again_and_an_unchanged_run_is_kept() {
    let vault = seeded_vault();
    let dir = TempWorkspace::new("vault-index-changed");
    let database = database_in(&dir);
    use_database(vault.path(), &database);
    let first = snapshot(vault.path());
    write_state(
        vault.path(),
        REPO,
        TICKET,
        NEW_RUN,
        ART_PIPELINE,
        BIG_TOKENS,
    );
    invalidate(vault.path());
    wait_until(|| stored_big_tokens(&database, vault.path()));
    assert!(stored_big_tokens(&database, vault.path()));
    let second = snapshot(vault.path());
    assert!(Arc::ptr_eq(&pick(&first, OLD_RUN), &pick(&second, OLD_RUN)));
}

#[test]
fn forget_makes_the_database_stale_so_the_next_read_walks_the_vault() {
    let vault = seeded_vault();
    let dir = TempWorkspace::new("vault-index-forget-db");
    let database = database_in(&dir);
    use_database(vault.path(), &database);
    assert_eq!(snapshot(vault.path()).run_count(), 4);
    forget(vault.path());
    assert!(!IndexStore::open(&database)
        .unwrap()
        .is_scanned(vault.path())
        .unwrap());
    std::fs::remove_dir_all(run_dir(vault.path(), REPO, OTHER_TICKET, OLD_RUN)).unwrap();
    assert_eq!(snapshot(vault.path()).run_count(), 3);
    assert_eq!(stored_runs_of(&database, vault.path()).len(), 3);
}

#[test]
fn refresh_ticket_stores_the_runs_it_read_again() {
    let vault = TempWorkspace::new("vault-index-ticket-db");
    let dir = TempWorkspace::new("vault-index-ticket-db-file");
    let database = database_in(&dir);
    write_state(vault.path(), REPO, TICKET, NEW_RUN, ART_PIPELINE, 1);
    use_database(vault.path(), &database);
    assert_eq!(snapshot(vault.path()).pixellab_of(TICKET).generations, 0);
    write_usage(
        vault.path(),
        REPO,
        TICKET,
        NEW_RUN,
        7,
        10.0,
        "2026-11-10T10:00:00Z",
    );
    refresh_ticket(vault.path(), TICKET);
    let stored = stored_runs_of(&database, vault.path());
    assert_eq!(stored[0].run.pixellab.generations, 7);
    assert!(stored[0].run.balance.is_some());
}

#[test]
fn another_schema_version_rebuilds_the_database() {
    let vault = seeded_vault();
    let dir = TempWorkspace::new("vault-index-schema");
    let database = database_in(&dir);
    let runs: Vec<StoredRun> = scan(vault.path(), None)
        .stored_runs(None)
        .into_values()
        .collect();
    IndexStore::open(&database)
        .unwrap()
        .replace(vault.path(), &runs)
        .unwrap();
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .pragma_update(None, USER_VERSION_PRAGMA, UNKNOWN_SCHEMA_VERSION)
            .unwrap();
    }
    let store = IndexStore::open(&database).unwrap();
    assert!(!store.is_scanned(vault.path()).unwrap());
    assert!(store.load(vault.path()).unwrap().is_empty());
    let connection = rusqlite::Connection::open(&database).unwrap();
    let version: u32 = connection
        .pragma_query_value(None, USER_VERSION_PRAGMA, |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn a_corrupt_database_is_recreated_and_the_index_still_answers() {
    let vault = seeded_vault();
    let dir = TempWorkspace::new("vault-index-corrupt");
    let database = database_in(&dir);
    std::fs::write(&database, GARBAGE).unwrap();
    assert!(IndexStore::open(&database).is_err());
    use_database(vault.path(), &database);
    assert_eq!(snapshot(vault.path()).run_count(), 4);
    assert_eq!(stored_runs_of(&database, vault.path()).len(), 4);
}

#[test]
fn an_unopenable_database_falls_back_to_memory() {
    let vault = seeded_vault();
    let dir = TempWorkspace::new("vault-index-unopenable");
    let blocked = dir.path().join(DB_NAME);
    std::fs::create_dir_all(&blocked).unwrap();
    use_database(vault.path(), &blocked);
    assert_eq!(snapshot(vault.path()).run_count(), 4);
    assert_eq!(refresh_ticket(vault.path(), TICKET).run_count(), 4);
    forget(vault.path());
    assert_eq!(snapshot(vault.path()).run_count(), 4);
}

#[test]
fn the_database_sits_next_to_the_memory_database() {
    let state = Path::new("state");
    assert_eq!(
        database_path(state).parent(),
        aidp_memory::store::db_path(state).parent()
    );
    assert_eq!(database_path(state).file_name().unwrap(), INDEX_DB_FILE);
}
