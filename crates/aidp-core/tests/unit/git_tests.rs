use super::*;

#[test]
fn recording_git_counts_each_method_independently() {
    let git = RecordingGit::new();
    let ws = Path::new(".");
    let _ = git.head(ws);
    let _ = git.head(ws);
    let _ = git.branch(ws);
    assert_eq!(git.calls.head.load(Ordering::SeqCst), 2);
    assert_eq!(git.calls.branch.load(Ordering::SeqCst), 1);
    assert_eq!(git.calls.log.load(Ordering::SeqCst), 0);
}

#[test]
fn recording_git_returns_canned_values() {
    let mut git = RecordingGit::new();
    git.head_value = Some("cafebabe".to_string());
    git.branch_value = Some("main".to_string());
    assert_eq!(git.head(Path::new(".")), Some("cafebabe".to_string()));
    assert_eq!(git.branch(Path::new(".")), Some("main".to_string()));
}
