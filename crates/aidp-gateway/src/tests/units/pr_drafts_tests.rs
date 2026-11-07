//! Drafts persist across reloads and are cleared only on request.

use super::*;
use aidp_integrations::models::enums::diff_side::DiffSide;

const REPO: &str = "acme/svc";
const NUMBER: u64 = 7;
const OTHER_NUMBER: u64 = 8;
const FIRST: &str = "d1";
const SECOND: &str = "d2";
const THIRD: &str = "d3";
const DRAFT_PATH: &str = "src/a.rs";
const DRAFT_BODY: &str = "nit";
const DRAFT_LINE: u32 = 3;
const CORRUPT_CONTENT: &str = "not json";

fn draft(id: &str) -> ReviewDraft {
    ReviewDraft {
        id: id.to_string(),
        path: DRAFT_PATH.to_string(),
        line: DRAFT_LINE,
        side: DiffSide::Right,
        start_line: None,
        start_side: None,
        body: DRAFT_BODY.to_string(),
    }
}

#[test]
fn drafts_survive_reload() {
    let dir = tempfile::tempdir().unwrap();
    PrDrafts::new(dir.path())
        .put(REPO, NUMBER, vec![draft(FIRST), draft(SECOND)])
        .unwrap();
    let reloaded = PrDrafts::new(dir.path());
    assert_eq!(
        reloaded.get(REPO, NUMBER),
        vec![draft(FIRST), draft(SECOND)]
    );
    assert!(reloaded.get(REPO, OTHER_NUMBER).is_empty());
}

#[test]
fn put_replaces_all_drafts() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrDrafts::new(dir.path());
    store
        .put(REPO, NUMBER, vec![draft(FIRST), draft(SECOND)])
        .unwrap();
    store.put(REPO, NUMBER, vec![draft(THIRD)]).unwrap();
    assert_eq!(store.get(REPO, NUMBER), vec![draft(THIRD)]);
}

#[test]
fn failed_submit_keeps_drafts_until_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrDrafts::new(dir.path());
    store.put(REPO, NUMBER, vec![draft(FIRST)]).unwrap();
    let submit: Result<(), &str> = Err("upstream failure");
    if submit.is_ok() {
        store.clear(REPO, NUMBER).unwrap();
    }
    assert_eq!(
        PrDrafts::new(dir.path()).get(REPO, NUMBER),
        vec![draft(FIRST)]
    );
    store.clear(REPO, NUMBER).unwrap();
    assert!(PrDrafts::new(dir.path()).get(REPO, NUMBER).is_empty());
}

#[test]
fn corrupt_file_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(DRAFTS_FILE), CORRUPT_CONTENT).unwrap();
    assert!(PrDrafts::new(dir.path()).get(REPO, NUMBER).is_empty());
}
