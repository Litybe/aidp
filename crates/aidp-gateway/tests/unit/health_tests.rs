use super::*;
use aidp_testkit::TempWorkspace;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

const EMBEDDED_DOCTOR_PROGRAM: &str = "packaged/Aidp.CommandHost";
const EMBEDDED_DOCTOR_SKILL: &str = "packaged/skill";

#[test]
fn embedded_doctor_uses_the_command_adapter_and_scoped_environment() {
    let environment = std::collections::BTreeMap::from([
        (
            crate::aidp_bin::AIDP_BIN_ENV.to_string(),
            EMBEDDED_DOCTOR_PROGRAM.to_string(),
        ),
        (
            crate::models::constants::flow_execution::SKILL_ROOT_ENV.to_string(),
            EMBEDDED_DOCTOR_SKILL.to_string(),
        ),
    ]);
    let command = doctor_command(Path::new(EMBEDDED_DOCTOR_PROGRAM), &environment);
    assert_eq!(command.get_program(), EMBEDDED_DOCTOR_PROGRAM);
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        DOCTOR_ARGS
            .iter()
            .map(std::ffi::OsStr::new)
            .collect::<Vec<_>>()
    );
    for (key, value) in environment {
        assert!(command
            .get_envs()
            .any(
                |(actual_key, actual_value)| actual_key == std::ffi::OsStr::new(&key)
                    && actual_value == Some(std::ffi::OsStr::new(&value))
            ));
    }
}

#[test]
fn looks_like_auth_error_detects_login_prompt_and_auth_subtype() {
    assert!(looks_like_auth_error(
        "Invalid API key \u{b7} Please run /login",
        None
    ));
    assert!(looks_like_auth_error(
        "anything",
        Some("error_authentication")
    ));
    assert!(looks_like_auth_error("anything", Some("weird_auth_thing")));
}

#[test]
fn looks_like_auth_error_is_conservative() {
    assert!(!looks_like_auth_error("", None));
    assert!(!looks_like_auth_error(
        "anything",
        Some("error_max_budget_usd")
    ));
    assert!(!looks_like_auth_error(
        "error: expected `;`, found `}` at src/main.rs:12",
        None
    ));
}

#[test]
fn expired_login_from_simulated_turn_events_fixture() {
    // Acceptance criterion: "scénario de login expiré simulé" — built
    // from a fixture of persisted turn events, never a live login.
    let events: Vec<Value> = vec![json!({
        "kind": "turn_end",
        "isError": true,
        "result": "Invalid API key \u{b7} Please run /login",
    })];
    let detail = expired_from_events(&events);
    assert!(detail.is_some());
    assert!(detail.unwrap().contains("Please run /login"));

    let not_expired: Vec<Value> = vec![json!({
        "kind": "turn_end",
        "isError": false,
        "result": "Invalid API key \u{b7} Please run /login",
    })];
    assert!(expired_from_events(&not_expired).is_none());
}

#[test]
fn expired_from_events_takes_the_newest_matching_turn_end() {
    let events: Vec<Value> = vec![
        json!({"kind": "turn_end", "isError": true, "result": "boring compile error"}),
        json!({"kind": "other", "isError": true, "result": "not logged in"}),
        json!({"kind": "turn_end", "isError": true, "result": "not logged in, please authenticate"}),
    ];
    let detail = expired_from_events(&events);
    assert!(detail.is_some());
    assert!(detail.unwrap().contains("not logged in"));
}

#[test]
fn expired_from_events_none_when_no_signal() {
    let events: Vec<Value> = vec![
        json!({"kind": "turn_end", "isError": false, "result": "all good"}),
        json!({"kind": "turn_end", "isError": true, "result": "error_max_budget_usd", "subtype": "error_max_budget_usd"}),
    ];
    assert!(expired_from_events(&events).is_none());
    assert!(expired_from_events(&[]).is_none());
}

const SHORT_PROBE_TIMEOUT: Duration = Duration::from_millis(300);
const KILL_DEADLINE: Duration = Duration::from_secs(5);
const QUICK_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const QUICK_COMMAND_OUTPUT: &str = "probe-output";
const SLEEP_SECONDS: &str = "30";

/// A command that sleeps far longer than any test timeout.
fn long_sleeper() -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", "Start-Sleep", SLEEP_SECONDS]);
        cmd
    } else {
        let mut cmd = Command::new("sleep");
        cmd.arg(SLEEP_SECONDS);
        cmd
    }
}

/// A command that prints `QUICK_COMMAND_OUTPUT` and exits at once.
fn quick_echo() -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "echo", QUICK_COMMAND_OUTPUT]);
        cmd
    } else {
        let mut cmd = Command::new("echo");
        cmd.arg(QUICK_COMMAND_OUTPUT);
        cmd
    }
}

#[test]
fn it_wait_or_kill_kills_and_reaps_a_child_past_its_timeout() {
    let mut child = long_sleeper()
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the sleeper command must spawn");
    let started = Instant::now();
    let status = wait_or_kill(&mut child, SHORT_PROBE_TIMEOUT);
    assert!(status.is_none());
    assert!(started.elapsed() < KILL_DEADLINE);
    assert!(matches!(child.try_wait(), Ok(Some(_))));
}

#[test]
fn it_run_bounded_with_returns_the_stdout_of_a_quick_command() {
    let (success, stdout, _stderr) = run_bounded_with(quick_echo(), QUICK_COMMAND_TIMEOUT)
        .expect("the quick command must finish before its timeout");
    assert!(success);
    assert!(stdout.contains(QUICK_COMMAND_OUTPUT));
}

#[test]
fn it_run_bounded_with_returns_none_on_timeout() {
    let started = Instant::now();
    assert!(run_bounded_with(long_sleeper(), SHORT_PROBE_TIMEOUT).is_none());
    assert!(started.elapsed() < KILL_DEADLINE);
}

#[test]
fn cached_computes_once_within_the_ttl() {
    let slot: CacheSlot<usize> = CacheSlot::empty();
    let compute_calls = AtomicUsize::new(0);
    let compute = || compute_calls.fetch_add(1, Ordering::SeqCst) + 1;
    assert_eq!(cached(&slot, PROBE_CACHE_TTL, compute), 1);
    assert_eq!(cached(&slot, PROBE_CACHE_TTL, compute), 1);
    assert_eq!(compute_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cached_recomputes_after_a_zero_ttl() {
    let slot: CacheSlot<usize> = CacheSlot::empty();
    let compute_calls = AtomicUsize::new(0);
    let compute = || compute_calls.fetch_add(1, Ordering::SeqCst) + 1;
    assert_eq!(cached(&slot, Duration::ZERO, compute), 1);
    assert_eq!(cached(&slot, Duration::ZERO, compute), 2);
    assert_eq!(compute_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn cached_is_single_flight_across_concurrent_cold_misses() {
    const THREAD_COUNT: usize = 16;
    let slot: Arc<CacheSlot<usize>> = Arc::new(CacheSlot::empty());
    let compute_calls = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(THREAD_COUNT));

    let handles: Vec<_> = (0..THREAD_COUNT)
        .map(|_| {
            let slot = Arc::clone(&slot);
            let compute_calls = Arc::clone(&compute_calls);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                cached(&slot, PROBE_CACHE_TTL, || {
                    compute_calls.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(100));
                    42
                })
            })
        })
        .collect();

    let results: Vec<usize> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(results.iter().all(|&value| value == 42));
    assert_eq!(compute_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cached_returns_the_stale_value_without_waiting_for_a_concurrent_refresh() {
    let slot: Arc<CacheSlot<usize>> = Arc::new(CacheSlot::empty());
    *lock(&slot.value) = Some((Instant::now() - PROBE_CACHE_TTL, 1));
    let started = Arc::new(std::sync::Barrier::new(2));

    let refreshing = {
        let slot = Arc::clone(&slot);
        let started = Arc::clone(&started);
        thread::spawn(move || {
            cached(&slot, PROBE_CACHE_TTL, || {
                started.wait();
                thread::sleep(Duration::from_millis(200));
                2
            })
        })
    };
    started.wait();
    thread::sleep(Duration::from_millis(20));

    let began = Instant::now();
    let stale = cached(&slot, PROBE_CACHE_TTL, || 3);
    assert_eq!(stale, 1);
    assert!(began.elapsed() < Duration::from_millis(100));

    assert_eq!(refreshing.join().unwrap(), 2);
}

/// Fill every probe cache so `health_json` answers without spawning.
fn seed_probe_caches() {
    *lock(&doctor_slot().value) = Some((
        Instant::now(),
        DoctorSummary::unavailable(SEEDED_DOCTOR_ERROR).to_value(),
    ));
    *lock(&login_probe_slot().value) = Some((Instant::now(), None));
    *lock(&aidp_binary_slot().value) = Some((Instant::now(), aidp_binary_unavailable()));
    *lock(&providers_slot().value) = Some((Instant::now(), Value::Array(Vec::new())));
}

/// Error text used to seed the doctor cache in tests that only need
/// `health_json` to answer without spawning a real probe.
const SEEDED_DOCTOR_ERROR: &str = "seeded";

#[test]
fn doctor_summary_serializes_expected_keys_and_types() {
    let ok_summary = DoctorSummary {
        ok: true,
        checks: 3,
        failing: Vec::new(),
        error: None,
    };
    let ok_value = ok_summary.to_value();
    assert_eq!(ok_value["ok"], json!(true));
    assert_eq!(ok_value["checks"], json!(3));
    assert_eq!(ok_value["failing"], json!([]));
    assert_eq!(ok_value["error"], Value::Null);

    let failing_summary = DoctorSummary {
        ok: false,
        checks: 2,
        failing: vec!["hooks_installed".to_string()],
        error: None,
    };
    let failing_value = failing_summary.to_value();
    assert_eq!(failing_value["ok"], json!(false));
    assert_eq!(failing_value["failing"], json!(["hooks_installed"]));

    let unavailable_value = DoctorSummary::unavailable(SEEDED_DOCTOR_ERROR).to_value();
    assert_eq!(unavailable_value["ok"], json!(false));
    assert_eq!(unavailable_value["checks"], json!(0));
    assert_eq!(unavailable_value["failing"], json!([]));
    assert_eq!(unavailable_value["error"], json!(SEEDED_DOCTOR_ERROR));
}

#[test]
fn health_json_body_carries_a_threads_object() {
    let workspace = TempWorkspace::new("health-threads");
    let state_dir = workspace.path();
    std::fs::create_dir_all(state_dir).unwrap();
    let bus = Arc::new(crate::bus::Bus::new());
    let projects = Arc::new(crate::projects::Projects::load(state_dir));
    let sessions = SessionManager::load(state_dir, bus, projects);
    seed_probe_caches();
    let started = Instant::now();
    let body = health_json(state_dir, &sessions, false);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(body.get(THREADS_KEY).is_some_and(Value::is_object));
    assert!(body.get("doctor").is_some());
    assert!(body.get("claudeLogin").is_some());
}

#[test]
fn aidp_binary_block_keeps_a_joined_report() {
    let report = json!({ "ok": true });
    assert_eq!(aidp_binary_block(Ok(report.clone())), report);
}

#[test]
fn aidp_binary_block_degrades_when_the_probe_thread_panicked() {
    let panicked = std::thread::spawn(|| -> Value { panic!("probe failed") }).join();
    let block = aidp_binary_block(panicked);
    assert_eq!(block["ok"], Value::Null);
    assert_eq!(block[AIDP_BINARY_ERROR_KEY], AIDP_BINARY_PROBE_PANICKED);
}

#[test]
fn aidp_binary_report_json_has_exactly_the_documented_keys() {
    let report = aidp_core::aidp_binary::evaluate(Default::default());
    let block = aidp_binary_block(Ok(report.to_json()));
    let mut keys: Vec<&str> = block
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "issues",
            "modifiedAt",
            "ok",
            "path",
            "runningModifiedAt",
            "runningPath"
        ]
    );
    assert_eq!(block["ok"], false);
    assert_eq!(block["issues"], json!(["absent"]));
}

#[test]
fn aidp_binary_key_does_not_collide_with_the_status_payload_key() {
    assert_eq!(AIDP_BINARY_KEY, "aidpBinary");
}

const HEALTH_TEST_KEY: &str = "HEALTH-RF-1";
const HEALTH_TEST_OTHER_KEY: &str = "HEALTH-RF-2";
const HEALTH_TEST_PATH: &str = "C:/vault/Pipelines/repo/HEALTH-RF-1/.worktree";
const HEALTH_TEST_ERROR: &str = "locked by another process";

/// A failure of `key` that went through three delete attempts.
fn remove_failure_of(key: &str, attempts: u32) -> WorktreeRemoveFailure {
    WorktreeRemoveFailure {
        key: key.to_string(),
        path: HEALTH_TEST_PATH.to_string(),
        error: HEALTH_TEST_ERROR.to_string(),
        attempts,
    }
}

#[test]
fn a_recorded_remove_failure_is_visible_in_the_health_body() {
    let workspace = TempWorkspace::new("health-remove-failed");
    let state_dir = workspace.path();
    std::fs::create_dir_all(state_dir).unwrap();
    let bus = Arc::new(crate::bus::Bus::new());
    let projects = Arc::new(crate::projects::Projects::load(state_dir));
    let sessions = SessionManager::load(state_dir, bus, projects);
    seed_probe_caches();
    record_worktree_remove_failure(remove_failure_of(HEALTH_TEST_KEY, 3));

    let body = health_json(state_dir, &sessions, false);

    let entries = body[WORKTREES_REMOVE_FAILED_KEY]
        .as_array()
        .expect("the health body carries the failed removals");
    let entry = entries
        .iter()
        .find(|entry| entry["key"] == HEALTH_TEST_KEY)
        .expect("the recorded failure is listed");
    assert_eq!(entry["path"], HEALTH_TEST_PATH);
    assert_eq!(entry["error"], HEALTH_TEST_ERROR);
    assert_eq!(entry["attempts"], 3);
    clear_worktree_remove_failure(HEALTH_TEST_KEY);
}

#[test]
fn recording_a_failure_again_replaces_the_earlier_entry_and_clearing_removes_it() {
    record_worktree_remove_failure(remove_failure_of(HEALTH_TEST_OTHER_KEY, 1));
    record_worktree_remove_failure(remove_failure_of(HEALTH_TEST_OTHER_KEY, 3));

    let matching: Vec<WorktreeRemoveFailure> = worktree_remove_failures()
        .into_iter()
        .filter(|failure| failure.key == HEALTH_TEST_OTHER_KEY)
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].attempts, 3);

    clear_worktree_remove_failure(HEALTH_TEST_OTHER_KEY);
    assert!(worktree_remove_failures()
        .iter()
        .all(|failure| failure.key != HEALTH_TEST_OTHER_KEY));
}

#[test]
fn cleanup_body_carries_purged_sessions_and_the_worktree_sweep() {
    let sweep = WorktreeSweepReport {
        removed: vec![
            SweptWorktree {
                key: "SWEEP-1".to_string(),
                salvaged_ref: Some("refs/aidp-wip/SWEEP-1-0123456789ab".to_string()),
            },
            SweptWorktree {
                key: "SWEEP-2".to_string(),
                salvaged_ref: None,
            },
        ],
        failed: vec![remove_failure_of("SWEEP-3", 3)],
    };

    let body = cleanup_json(&["s-1".to_string()], &sweep);

    assert_eq!(body["ok"], true);
    assert_eq!(body["count"], 1);
    assert_eq!(body["purged"], json!(["s-1"]));
    assert_eq!(body["worktrees"]["removed"][0]["key"], "SWEEP-1");
    assert_eq!(
        body["worktrees"]["removed"][0]["salvagedRef"],
        "refs/aidp-wip/SWEEP-1-0123456789ab"
    );
    assert!(body["worktrees"]["removed"][1].get("salvagedRef").is_none());
    assert_eq!(body["worktrees"]["failed"][0]["key"], "SWEEP-3");
    assert_eq!(body["worktrees"]["failed"][0]["attempts"], 3);
}
