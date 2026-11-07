use super::*;
use crate::models::enums::attention_signal_kind::AttentionSignalKind;

const REPO: &str = "org/repo";
const SHA: &str = "abc";
const FIRST: &str = "2026-01-01T00:00:00Z";
const SECOND: &str = "2026-01-02T00:00:00Z";

fn signal(fingerprint: &str, notify: bool) -> AttentionSignal {
    AttentionSignal {
        kind: AttentionSignalKind::Conflict,
        fingerprint: fingerprint.to_string(),
        detail: None,
        notify,
    }
}

#[test]
fn same_signal_dedups_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrAttention::new(dir.path());
    let signals = [signal("fp1", true)];
    assert_eq!(store.upsert_new(&signals, REPO, 1, SHA, FIRST).len(), 1);
    assert!(store.upsert_new(&signals, REPO, 1, SHA, SECOND).is_empty());
    let restarted = PrAttention::new(dir.path());
    assert!(restarted
        .upsert_new(&signals, REPO, 1, SHA, SECOND)
        .is_empty());
    let listed = restarted.list(false);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].first_seen, FIRST);
}

#[test]
fn new_sha_fingerprint_makes_new_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrAttention::new(dir.path());
    store.upsert_new(&[signal("fp-a", true)], REPO, 1, SHA, FIRST);
    let created = store.upsert_new(&[signal("fp-b", true)], REPO, 1, "def", SECOND);
    assert_eq!(created.len(), 1);
    let listed = store.list(false);
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].fingerprint, "fp-b");
}

#[test]
fn mark_read_by_fingerprints_and_all() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrAttention::new(dir.path());
    store.upsert_new(
        &[signal("a", true), signal("b", true), signal("c", true)],
        REPO,
        1,
        SHA,
        FIRST,
    );
    let some = AttentionReadRequest {
        fingerprints: Some(vec!["a".to_string()]),
        all: None,
    };
    assert_eq!(store.mark_read(&some), 1);
    assert_eq!(store.mark_read(&some), 0);
    assert_eq!(store.list(true).len(), 2);
    let everything = AttentionReadRequest {
        fingerprints: None,
        all: Some(true),
    };
    assert_eq!(store.mark_read(&everything), 2);
    assert!(store.list(true).is_empty());
    assert_eq!(store.list(false).len(), 3);
}

#[test]
fn silent_records_are_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrAttention::new(dir.path());
    let created = store.upsert_new(&[signal("quiet", false)], REPO, 1, SHA, FIRST);
    assert!(!created[0].notified);
    assert!(!PrAttention::new(dir.path()).list(false)[0].notified);
}
