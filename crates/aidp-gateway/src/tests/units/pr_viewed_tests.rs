//! Viewed marks reset when the blob changes; `changed_since` lists moved files.

use super::*;

const REPO: &str = "acme/svc";
const NUMBER: u64 = 7;
const PATH_A: &str = "src/a.rs";
const PATH_B: &str = "src/b.rs";
const PATH_C: &str = "src/c.rs";
const SHA_OLD: &str = "aaa";
const SHA_NEW: &str = "bbb";

fn blob(path: &str, sha: &str) -> FileBlob {
    FileBlob {
        path: path.to_string(),
        sha: sha.to_string(),
    }
}

#[test]
fn viewed_then_blob_changed_is_unviewed() {
    let dir = tempfile::tempdir().unwrap();
    PrViewed::new(dir.path())
        .mark(REPO, NUMBER, PATH_A, SHA_OLD)
        .unwrap();
    let reloaded = PrViewed::new(dir.path());
    let same = reloaded.viewed_state(
        REPO,
        NUMBER,
        &[blob(PATH_A, SHA_OLD), blob(PATH_B, SHA_OLD)],
    );
    assert!(same[0].viewed);
    assert!(!same[1].viewed);
    let moved = reloaded.viewed_state(REPO, NUMBER, &[blob(PATH_A, SHA_NEW)]);
    assert!(!moved[0].viewed);
}

#[test]
fn unmark_clears_the_mark() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrViewed::new(dir.path());
    store.mark(REPO, NUMBER, PATH_A, SHA_OLD).unwrap();
    store.unmark(REPO, NUMBER, PATH_A).unwrap();
    assert!(!store.viewed_state(REPO, NUMBER, &[blob(PATH_A, SHA_OLD)])[0].viewed);
}

#[test]
fn changed_since_lists_modified_and_new_files() {
    let at_review = [blob(PATH_A, SHA_OLD), blob(PATH_B, SHA_OLD)];
    let now = [
        blob(PATH_A, SHA_OLD),
        blob(PATH_B, SHA_NEW),
        blob(PATH_C, SHA_OLD),
    ];
    assert_eq!(
        changed_since(&at_review, &now),
        vec![PATH_B.to_string(), PATH_C.to_string()]
    );
}
