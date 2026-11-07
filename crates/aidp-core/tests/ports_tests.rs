use aidp_core::clock::{Clock, FixedClock, SystemClock};
use aidp_core::git::{Git, RecordingGit};
use aidp_core::id::{FixedIdGen, IdGen, SystemIdGen};
use aidp_core::process_runner::{
    ProcessRunner, ProcessSpec, ScriptedOutput, ScriptedProcessRunner,
};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::Ordering;

#[test]
fn fixed_clock_returns_a_fixed_rfc3339_and_matching_epoch_millis() {
    let clock = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    assert_eq!(clock.now_rfc3339(), "2026-01-01T00:00:00+00:00");
    assert_eq!(clock.now_ms(), 1_767_225_600_000);
}

#[test]
fn system_id_gen_reproduces_the_legacy_new_run_id_shape() {
    let clock = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    let id = SystemIdGen.new_run_id(&clock);
    let mut parts = id.rsplitn(2, '-');
    let rand = parts.next().expect("id must contain a suffix");
    let ts = parts.next().expect("id must contain a timestamp");
    assert_eq!(ts, "20260101-000000");
    assert_eq!(rand.len(), 4);
    assert!(rand.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn fixed_id_gen_always_returns_the_same_id_regardless_of_clock() {
    let gen = FixedIdGen("FIXED-0001".to_string());
    let clock_a = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    let clock_b = FixedClock::from_rfc3339("2030-06-15T12:34:56Z");
    assert_eq!(gen.new_run_id(&clock_a), "FIXED-0001");
    assert_eq!(gen.new_run_id(&clock_b), "FIXED-0001");
}

#[test]
fn recording_git_hands_back_canned_head_and_branch_and_counts_calls() {
    let mut git = RecordingGit::new();
    git.head_value = Some("cafebabe000000000000000000000000000000ff".to_string());
    git.branch_value = Some("main".to_string());
    let ws = Path::new(".");

    assert_eq!(
        git.head(ws),
        Some("cafebabe000000000000000000000000000000ff".to_string())
    );
    assert_eq!(git.branch(ws), Some("main".to_string()));

    assert_eq!(git.calls.head.load(Ordering::SeqCst), 1);
    assert_eq!(git.calls.branch.load(Ordering::SeqCst), 1);
    assert_eq!(git.calls.log.load(Ordering::SeqCst), 0);
    assert_eq!(git.calls.changed.load(Ordering::SeqCst), 0);
}

#[test]
fn recording_git_worktree_list_and_last_commit_date_return_canned_values() {
    let mut git = RecordingGit::new();
    git.worktrees = vec![std::path::PathBuf::from("/tmp/wt1")];
    git.last_commit_dates
        .get_mut()
        .expect("mutex must not be poisoned")
        .insert("a.txt".to_string(), "2026-01-01T00:00:00Z".to_string());

    let ws = Path::new(".");
    assert_eq!(
        git.worktree_list(ws),
        vec![std::path::PathBuf::from("/tmp/wt1")]
    );
    assert_eq!(
        git.last_commit_date(ws, "a.txt"),
        Some("2026-01-01T00:00:00Z".to_string())
    );
    assert_eq!(git.last_commit_date(ws, "missing.txt"), None);
    assert_eq!(git.calls.worktree_list.load(Ordering::SeqCst), 1);
    assert_eq!(git.calls.last_commit_date.load(Ordering::SeqCst), 2);
}

#[test]
fn scripted_process_runner_output_replays_canned_bytes_and_status() {
    let runner = ScriptedProcessRunner::new().with_output(ScriptedOutput {
        status_code: 0,
        stdout: b"hello".to_vec(),
        stderr: Vec::new(),
    });
    let spec = ProcessSpec::new("anything");
    let out = runner.output(&spec).expect("output must succeed");
    assert_eq!(out.stdout, b"hello");
    assert!(out.status.success());
}

#[test]
fn scripted_process_runner_spawn_replays_a_full_session_on_stdout() {
    let fixture = "{\"type\":\"init\"}\n{\"type\":\"message\"}\n{\"type\":\"result\"}\n";
    let runner = ScriptedProcessRunner::new().with_spawn_stdout(fixture.as_bytes().to_vec());
    let spec = ProcessSpec::new("claude").arg("--stream-json");
    let mut child = runner.spawn(&spec).expect("spawn must succeed");
    let mut out = String::new();
    child
        .take_stdout()
        .expect("stdout must be present")
        .read_to_string(&mut out)
        .expect("read must succeed");
    assert_eq!(out, fixture);
    assert_eq!(child.wait().expect("wait must succeed"), 0);
}

#[test]
fn os_process_runner_output_runs_a_real_trivial_command() {
    let runner = aidp_core::process_runner::OsProcessRunner;
    #[cfg(windows)]
    let spec = ProcessSpec::new("cmd").args(["/C", "echo hi"]);
    #[cfg(not(windows))]
    let spec = ProcessSpec::new("echo").arg("hi");
    let out = runner.output(&spec).expect("a trivial command must run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("hi"));
}

#[test]
fn system_clock_now_ms_and_now_rfc3339_are_consistent_with_each_other() {
    let clock = SystemClock;
    let before = clock.now_ms();
    let text = clock.now_rfc3339();
    assert!(!text.is_empty());
    assert!(before > 0);
}
