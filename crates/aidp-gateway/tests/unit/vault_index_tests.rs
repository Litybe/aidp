use super::*;
use crate::models::constants::pixellab::USAGE_FILE;
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
