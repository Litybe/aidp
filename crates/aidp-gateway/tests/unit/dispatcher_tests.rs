use super::*;
use crate::models::enums::SessionStatus;
use crate::queue::QueuedLaunch;
use crate::sessions::SessionRecord;
use aidp_testkit::TempWorkspace;

const TEST_PAUSE_ACTOR: &str = "tester";
const PAUSED_STATE_KEY: &str = "paused";
const PAUSED_CONFIG_LINE: &str = "paused: true";
const EVENT_TYPE_KEY: &str = "type";
const EVENT_DATA_KEY: &str = "data";
const ACTOR_KEY: &str = "actor";
const AT_KEY: &str = "at";

/// Disables the global vault so a launch runs in the project checkout and creates no worktree.
/// Points `AIDP_CLAUDE_BIN` at a stub that stays alive ~30s and exits 0,
/// so a test that launches a session sees it running without depending
/// on a real `claude` on the host PATH.
/// Hold both returned values for the whole test.
fn stub_claude_bin() -> (tempfile::TempDir, aidp_testkit::EnvGuard) {
    let (dir, bin) = stub_claude_script();
    let guard = aidp_testkit::EnvGuard::with(&[
        ("AIDP_CLAUDE_BIN", Some(bin.to_str().unwrap())),
        ("AIDP_VAULT", Some("off")),
    ]);
    (dir, guard)
}

/// The stub `claude` of [`stub_claude_bin`], without touching the environment.
/// For tests that must set several variables under one `EnvGuard`.
fn stub_claude_script() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let bin = {
        let bin = dir.path().join("claude.cmd");
        std::fs::write(
            &bin,
            "@echo off\r\nping -n 30 127.0.0.1 >nul\r\nexit /b 0\r\n",
        )
        .unwrap();
        bin
    };
    #[cfg(not(windows))]
    let bin = {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.path().join("claude");
        std::fs::write(&bin, "#!/bin/sh\nsleep 30\nexit 0\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    };
    (dir, bin)
}

fn sample_session_record(id: &str, origin: Origin) -> SessionRecord {
    serde_json::from_value(json!({
        "id": id,
        "projectId": "p1",
        "projectName": "Project One",
        "cwd": "/tmp/p1",
        "engine": "v7",
        "prompt": "run pipeline v7 dev-aidp PIXEL-1",
        "permissionMode": "default",
        "status": "running",
        "startedAt": "2026-09-14T09:00:00Z",
        "origin": origin,
        "autoClose": true,
        "eventCount": 0,
    }))
    .expect("sample session record must deserialize")
}

fn queued(id: &str) -> QueuedLaunch {
    QueuedLaunch {
        id: id.to_string(),
        project_id: "p1".to_string(),
        project_name: "Project One".to_string(),
        template_id: Some("dev-aidp".to_string()),
        target: Some("PIXEL-1".to_string()),
        engine: "v7".to_string(),
        priority: 0,
        status: "queued".to_string(),
        queued_at: "2026-09-14T09:00:00.000Z".to_string(),
        origin: Origin::new("dispatcher"),
        request: json!({ "secret": "do-not-expose" }),
        reason: None,
        workspace_key: None,
    }
}

#[test]
fn queued_launch_is_cancelled_and_nothing_is_left_in_the_queue() {
    let entry = queued("q-1");
    let launch = Launch::Queued(Box::new(entry.clone()));

    let mut cancelled_ids: Vec<String> = Vec::new();
    let outcome = reconcile_launch(&launch, |id| cancelled_ids.push(id.to_string()));

    assert_eq!(
        cancelled_ids,
        vec!["q-1".to_string()],
        "the closure must be called exactly once, with the queue entry's id"
    );
    match outcome {
        LaunchOutcome::Requeued(id) => assert_eq!(id, "q-1"),
        LaunchOutcome::Started(_) => panic!("expected Requeued"),
    }

    // Started never calls the closure.
    let rec = sample_session_record("s-1", Origin::new("dispatcher"));
    let started = Launch::Started(Box::new(rec.clone()));
    let mut calls = 0usize;
    let outcome2 = reconcile_launch(&started, |_id| calls += 1);
    assert_eq!(calls, 0, "Started must never call the cancel closure");
    match outcome2 {
        LaunchOutcome::Started(id) => assert_eq!(id, "s-1"),
        LaunchOutcome::Requeued(_) => panic!("expected Started"),
    }
}

/// A minimal `Dispatcher` for tests that need the real `catalog()`
/// caching behaviour, not just its pure helper functions. Built by hand
/// (not `Dispatcher::new`) so `skill_root` can point at a directory the
/// test itself controls — this module's other tests are all pure
/// free-function tests, so there is no existing fixture to reuse.
/// Returns (Dispatcher, state_workspace, skill_workspace) so both can be kept alive.
fn test_dispatcher(skill_root: PathBuf) -> (Dispatcher, TempWorkspace, TempWorkspace) {
    let bus = Arc::new(Bus::new());
    let _state_ws = TempWorkspace::new("dispatcher-test-state");
    let state_dir = _state_ws.path();
    let _skill_ws = TempWorkspace::new("dispatcher-test-skill");

    let projects = Arc::new(Projects::load(state_dir));
    let sessions = SessionManager::load(state_dir, bus.clone(), projects.clone());
    let dispatcher = Dispatcher {
        vault_root: PathBuf::new(),
        sessions,
        launcher: None,
        flows: Mutex::new(None),
        projects,
        bus,
        state: Mutex::new(DispatcherState::default()),
        skill_root: Some(skill_root),
        last_sync: Mutex::new(None),
        catalog_cache: Mutex::new(None),
        tick_lock: Mutex::new(()),
        warned_no_vault: AtomicBool::new(false),
        first_tick: AtomicBool::new(true),
        worktree_ops: WorktreeOps::system(),
    };
    (dispatcher, _state_ws, _skill_ws)
}

#[test]
fn it_catalog_is_served_from_cache_within_the_refresh_window() {
    let _ws = TempWorkspace::new("dispatcher-catalog-cache");
    let root = _ws.path();
    let templates = root.join("templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(
        templates.join("dev-aidp.yaml"),
        "id: dev-aidp\ninputs:\n  ticket_key: {}\n",
    )
    .unwrap();

    let (d, _ws_state, _ws_skill) = test_dispatcher(root.to_path_buf());
    let first = d.catalog();
    assert!(
        first.known("dev-aidp"),
        "the freshly-built catalog must see the template written above"
    );

    // Add a second template directly on disk. Within the refresh
    // window, `catalog()` must NOT see it: the cache, not a no-op
    // wrapper, is what is actually being consulted.
    std::fs::write(
        templates.join("dev-other.yaml"),
        "id: dev-other\ninputs:\n  ticket_key: {}\n",
    )
    .unwrap();
    let second = d.catalog();
    assert!(
        !second.known("dev-other"),
        "a cached catalog must not reflect a template added after it was built"
    );
}

#[test]
fn guard_failure_never_journals_a_mismatched_event() {
    // Done -> InProgress is not in Done's allowed-target list
    // (`&[Todo, Cancelled]`), so the guard must refuse it and the
    // helper must return None: no Event is ever produced to journal a
    // transition the ticket's status never actually underwent.
    let refused = guard_and_build_event(
        "cannot test",
        "LOC-1",
        Status::Done,
        Status::InProgress,
        "test".to_string(),
        None,
    );
    assert!(
        refused.is_none(),
        "a refused guard must never produce an Event to journal"
    );

    // The same helper, called with a legal transition, must still build
    // the Event exactly as before — this is not a behaviour change on
    // the success path.
    let allowed = guard_and_build_event(
        "cannot test",
        "LOC-1",
        Status::InProgress,
        Status::Blocked,
        "gate-timeout".to_string(),
        Some("s-1".to_string()),
    )
    .expect("a legal transition must still produce an Event");
    assert_eq!(allowed.key, "LOC-1");
    assert_eq!(allowed.from, Some(Status::InProgress));
    assert_eq!(allowed.to, Status::Blocked);
    assert_eq!(allowed.reason.as_deref(), Some("gate-timeout"));
    assert_eq!(allowed.session_id.as_deref(), Some("s-1"));
}

#[test]
fn inputs_file_is_ascii_escaped() {
    let _ws = TempWorkspace::new("dispatcher-ascii-test");
    let dir = _ws.path();
    let path = dir.join("PIXEL-1.inputs.json");

    let value = json!({
        "prompt": "Résumé → terminé",
        "feature": "PIXEL-1",
        "track": "standard",
    });
    write_inputs_file(&path, &value).expect("write_inputs_file must succeed");

    let text = std::fs::read_to_string(&path).expect("file must exist");
    assert!(
        text.is_ascii(),
        "every byte of the file must be ASCII: {text:?}"
    );
    assert!(text.contains("\\u00e9"), "é must escape to \\u00e9: {text}");
    assert!(text.contains("\\u2192"), "→ must escape to \\u2192: {text}");

    let parsed: Value = serde_json::from_str(&text).expect("must still parse as JSON");
    assert_eq!(parsed["prompt"], json!("Résumé → terminé"));
    assert_eq!(parsed["feature"], json!("PIXEL-1"));
}

#[test]
fn dispatcher_state_shape_is_stable() {
    let state = DispatcherState {
        mode: "normal".to_string(),
        paused: false,
        wait_until: None,
        last_tick_at: "2026-09-14T09:08:09Z".to_string(),
        last_reason: "timer".to_string(),
        slots: SlotsState {
            used: 2,
            max: 5,
            test_used: 0,
            test_max: 1,
        },
        running: vec![RunningEntry {
            key: "PIXEL-5677".to_string(),
            session_id: "s-1".to_string(),
            project: "cheat-alerts-service".to_string(),
            started_at: "2026-09-14T09:00:00Z".to_string(),
            warnings: Vec::new(),
        }],
        last_plan: LastPlan {
            launched: vec!["PIXEL-5677".to_string()],
            skipped: vec![SkippedEntry {
                key: "LOC-2".to_string(),
                reason: "project-busy".to_string(),
                detail: None,
            }],
        },
        budget: BudgetState {
            verdict: "allow".to_string(),
            max_launches: 3,
            why: String::new(),
            utilization_5h: 0.42,
            utilization_7d: 0.10,
            limit_5h: 0.80,
            limit_7d: 0.90,
            reserve_5h: 0.10,
            reserve_7d: 0.02,
            unreflected: 0,
            resets_at_5h: Some("2026-09-14T14:00:00.000Z".to_string()),
            wait_window: None,
            observed_at: Some("2026-09-14T09:08:00.000Z".to_string()),
        },
    };

    let v = serde_json::to_value(&state).expect("serialize");
    let obj = v.as_object().expect("object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec![
        "mode",
        "paused",
        "waitUntil",
        "lastTickAt",
        "lastReason",
        "slots",
        "running",
        "lastPlan",
        "budget",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);

    assert_eq!(
        v["waitUntil"],
        Value::Null,
        "waitUntil must serialize as null, not be omitted"
    );
    assert_eq!(v["slots"]["used"], json!(2));
    assert_eq!(v["slots"]["max"], json!(5));
    assert_eq!(v["running"][0]["key"], json!("PIXEL-5677"));
    assert_eq!(v["running"][0]["sessionId"], json!("s-1"));
    assert_eq!(v["lastPlan"]["launched"][0], json!("PIXEL-5677"));
    assert_eq!(v["lastPlan"]["skipped"][0]["key"], json!("LOC-2"));
    assert_eq!(v["lastPlan"]["skipped"][0]["reason"], json!("project-busy"));
    assert_eq!(v["budget"]["verdict"], json!("allow"));
    assert_eq!(v["budget"]["maxLaunches"], json!(3));

    let round_tripped: DispatcherState = serde_json::from_value(v).expect("must deserialize back");
    assert_eq!(round_tripped, state);
}

#[test]
fn only_dispatcher_sessions_are_adopted() {
    let ui_origin = Origin {
        kind: "ui".to_string(),
        r#ref: Some("PIXEL-1".to_string()),
        label: None,
    };
    assert_eq!(
        dispatcher_ticket_key(&ui_origin),
        None,
        "origin.kind != \"dispatcher\" must never be adopted"
    );

    let dispatcher_without_ref = Origin {
        kind: "dispatcher".to_string(),
        r#ref: None,
        label: None,
    };
    assert_eq!(
        dispatcher_ticket_key(&dispatcher_without_ref),
        None,
        "a dispatcher origin without a ref must never be adopted"
    );

    let dispatcher_with_ref = Origin {
        kind: "dispatcher".to_string(),
        r#ref: Some("PIXEL-1".to_string()),
        label: Some("Fix the thing".to_string()),
    };
    assert_eq!(
        dispatcher_ticket_key(&dispatcher_with_ref),
        Some("PIXEL-1"),
        "a dispatcher origin with a ref must resolve to its ticket key"
    );
}

// -------------------------------------------------------------------
// Lot 10a: budget-guard fixtures and tests.
// -------------------------------------------------------------------

/// A `RateLimitInfo` snapshot with the given 5h utilization, resetting
/// `resets_in_secs` seconds from now. `status` is normally `"ok"`.
fn rate_limit_snapshot(
    status: &str,
    five_hour_utilization: f64,
    resets_in_secs: i64,
) -> crate::sessions::RateLimitInfo {
    crate::sessions::RateLimitInfo {
        status: status.to_string(),
        is_using_overage: false,
        five_hour: Some(crate::sessions::RateLimitWindow {
            utilization: five_hour_utilization,
            resets_at: now_ms() / 1000 + resets_in_secs,
        }),
        seven_day: None,
        observed_at: now_ms(),
    }
}

/// A `Dispatcher` wired to a REAL, empty vault (so `tick`/`tick_local`
/// run the full lot 10a budget path instead of the `"no-vault"`
/// short-circuit), with `snapshot` — if any — already visible to
/// `self.sessions.rate_limit()`.
///
/// `SessionManager`'s `rate_limit` field is private and normally only
/// set by parsing real `claude` CLI output, so this seeds it the same
/// way a daemon restart would inherit it: `snapshot` is appended to the
/// on-disk rate-limit history BEFORE `SessionManager::load` runs, and
/// `load` seeds its in-memory cache from that history via
/// `usage::latest_live_snapshot` (see `sessions.rs:645`).
///
/// Returns `(dispatcher, bus, vault_ws, state_ws)` — keep the workspaces alive
/// as long as they are needed.
fn dispatcher_with_snapshot(
    tag: &str,
    snapshot: Option<crate::sessions::RateLimitInfo>,
) -> (Dispatcher, Arc<Bus>, TempWorkspace, TempWorkspace) {
    let vault_ws = TempWorkspace::new(&format!("dispatcher-budget-vault-{tag}"));
    let state_ws = TempWorkspace::new(&format!("dispatcher-budget-state-{tag}"));
    let vault_root = vault_ws.path();
    let state_dir = state_ws.path();

    if let Some(info) = &snapshot {
        crate::usage::append_rate_limit(state_dir, info);
    }

    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(state_dir));
    let sessions = SessionManager::load(state_dir, bus.clone(), projects.clone());

    let d = Dispatcher {
        vault_root: vault_root.to_path_buf(),
        sessions,
        launcher: None,
        flows: Mutex::new(None),
        projects,
        bus: bus.clone(),
        state: Mutex::new(DispatcherState::default()),
        skill_root: None,
        last_sync: Mutex::new(None),
        catalog_cache: Mutex::new(None),
        tick_lock: Mutex::new(()),
        warned_no_vault: AtomicBool::new(false),
        first_tick: AtomicBool::new(true),
        worktree_ops: WorktreeOps::system(),
    };
    (d, bus, vault_ws, state_ws)
}

/// The persisted dispatcher state file of the vault at `vault_root`.
fn dispatcher_state_file(vault_root: &Path) -> PathBuf {
    vault_root
        .join("TODO")
        .join(".state")
        .join("dispatcher.json")
}

#[test]
fn tick_reports_busy_without_side_effects_while_another_tick_holds_the_lock() {
    let (d, bus, vault_ws, _state_ws) = dispatcher_with_snapshot("busy-held", None);
    let state_file = dispatcher_state_file(vault_ws.path());

    let busy_report = {
        let _held = d.tick_lock.lock().unwrap();
        d.tick_local("test")
    };

    assert_eq!(busy_report["mode"], json!(BUSY_MODE));
    assert_eq!(busy_report["lastReason"], json!("test"));
    assert_eq!(busy_report["launched"], json!([]));
    assert!(
        !state_file.exists(),
        "a busy tick must not write the dispatcher state"
    );
    let tick_events = bus
        .recent(100, 0)
        .into_iter()
        .filter(|event| event["type"] == json!("dispatcher.tick"))
        .count();
    assert_eq!(tick_events, 0, "a busy tick must not emit dispatcher.tick");
}

const TODO_DIR: &str = "TODO";
const CONFIG_FILE: &str = "config.yaml";

#[test]
fn tick_runs_normally_once_the_lock_is_released() {
    let (d, _bus, _vault_ws, _state_ws) = dispatcher_with_snapshot("busy-released", None);
    {
        let _held = d.tick_lock.lock().unwrap();
        assert_eq!(d.tick_local("held")["mode"], json!(BUSY_MODE));
    }

    let report = d.tick_local("released");

    assert_ne!(
        report["mode"],
        json!(BUSY_MODE),
        "a tick after the lock is released must run: {report}"
    );
}

#[test]
fn concurrent_ticks_both_return_either_busy_or_a_real_report() {
    let (d, _bus, _vault_ws, _state_ws) = dispatcher_with_snapshot("busy-concurrent", None);

    let reports: Vec<Value> = thread::scope(|scope| {
        let first = scope.spawn(|| d.tick_local("first"));
        let second = scope.spawn(|| d.tick_local("second"));
        vec![
            first.join().expect("the first tick must not panic"),
            second.join().expect("the second tick must not panic"),
        ]
    });

    for report in &reports {
        assert!(
            report.get("error").is_none(),
            "a concurrent tick must not fail: {report}"
        );
    }
    let busy_count = reports
        .iter()
        .filter(|report| report["mode"] == json!(BUSY_MODE))
        .count();
    assert!(
        busy_count < reports.len(),
        "at least one of two concurrent ticks must run its body: {reports:?}"
    );
}

#[test]
fn dispatcher_waiting_is_emitted_once_across_consecutive_ticks_that_stay_waiting() {
    // 95% on the 5h window, resetting in 5h — well past the default 80%
    // ceiling, and far enough out that `until_ms` (`resets_at_s * 1000 +
    // 60_000`) stays in the future for the whole test.
    let snap = rate_limit_snapshot("ok", 0.95, 5 * 3600);
    let (d, bus, _vault_ws, _state_ws) = dispatcher_with_snapshot("waiting-once", Some(snap));

    d.tick_local("seed");
    assert_eq!(
        d.state_json()["mode"],
        json!("waiting"),
        "a 5h window at 95% must enter waiting on the first tick"
    );

    // Two more ticks that must both stay waiting (deadline is hours
    // away), taking the `waiting_noop` fast path.
    d.tick_local("again-1");
    d.tick_local("again-2");
    assert_eq!(d.state_json()["mode"], json!("waiting"));

    let waiting_events = bus
        .recent(100, 0)
        .into_iter()
        .filter(|e| e["type"] == json!("dispatcher.waiting"))
        .count();
    assert_eq!(
        waiting_events, 1,
        "dispatcher.waiting must fire exactly once per entry into waiting, not once per tick"
    );
}

#[test]
fn session_ended_replay_is_a_no_op_while_waiting() {
    let snap = rate_limit_snapshot("ok", 0.95, 5 * 3600);
    let (d, bus, _vault_ws, _state_ws) = dispatcher_with_snapshot("waiting-replay", Some(snap));

    d.tick_local("seed");
    assert_eq!(d.state_json()["mode"], json!("waiting"));

    // The exact call `on_session_ended` makes to replay immediately
    // after a slot frees — CRITICAL: nothing may launch on this path
    // while waiting.
    let report = d.tick("session-ended");
    assert_eq!(
        report["mode"],
        json!("waiting"),
        "a tick while waiting must take the `waiting_noop` fast path — its report carries \
         `mode`, unlike a real tick's report"
    );
    assert_eq!(report["launched"], json!([]));

    let events = bus.recent(100, 0);
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == json!("dispatcher.waiting"))
            .count(),
        1,
        "the replay tick must not re-enter waiting a second time"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == json!("ticket.started"))
            .count(),
        0,
        "no launch must ever happen while waiting, including on this replay path"
    );
}

#[test]
fn leaving_wait_emits_resumed_and_reopens_the_budget() {
    // No rate-limit snapshot at all -> `budget::verdict`'s rule 1
    // "unknown" branch -> the default `unknown_policy = "conservative"`,
    // which is neither `wait` nor `paused`. That alone is enough to
    // prove the leaving-waiting transition, without needing a real
    // post-reset snapshot — a real `Wait`'s own `until_ms` is always at
    // least 60s past its window's reset, which a unit test cannot wait
    // out.
    let (d, bus, _vault_ws, _state_ws) = dispatcher_with_snapshot("resume", None);

    // Seed the dispatcher as already `"waiting"`, with the deadline
    // already elapsed — `waiting_noop` falls through to a real
    // recompute exactly as it would once a genuine `wait_until` passes.
    {
        let mut s = lock(&d.state);
        s.mode = "waiting".to_string();
        s.wait_until = Some("2020-01-01T00:00:00.000Z".to_string());
    }

    d.tick_local("resume-check");

    let resumed: Vec<Value> = bus
        .recent(100, 0)
        .into_iter()
        .filter(|e| e["type"] == json!("dispatcher.resumed"))
        .collect();
    assert_eq!(
        resumed.len(),
        1,
        "dispatcher.resumed must fire exactly once on leaving waiting: {resumed:?}"
    );
    assert_eq!(resumed[0]["data"]["reason"], json!("budget"));

    let state = d.state_json();
    assert_eq!(state["mode"], json!("normal"));
    assert_eq!(state["waitUntil"], Value::Null);
    assert_eq!(state["budget"]["verdict"], json!("conservative"));
    assert!(
        state["budget"]["maxLaunches"].as_u64().unwrap_or(0) >= 1,
        "the budget must have room again once the dispatcher stops waiting: {state}"
    );
}

/// The `dispatcher.paused_changed` events published so far on `bus`.
fn paused_changed_events(bus: &Bus) -> Vec<Value> {
    bus.recent(100, 0)
        .into_iter()
        .filter(|event| event[EVENT_TYPE_KEY] == json!(EVENT_DISPATCHER_PAUSED_CHANGED))
        .collect()
}

#[test]
fn set_paused_change_writes_config_refreshes_cache_and_emits_once() {
    let (d, bus, vault_ws, _state_ws) = dispatcher_with_snapshot("set-paused-on", None);

    let outcome = d.set_paused(true, TEST_PAUSE_ACTOR).unwrap();

    assert!(outcome);
    let config_text =
        std::fs::read_to_string(vault_ws.path().join(TODO_DIR).join(CONFIG_FILE)).unwrap();
    assert!(
        config_text.contains(PAUSED_CONFIG_LINE),
        "config.yaml must hold the paused flag: {config_text}"
    );
    assert_eq!(d.state_json()[PAUSED_STATE_KEY], json!(true));
    let events = paused_changed_events(&bus);
    assert_eq!(events.len(), 1, "exactly one event expected: {events:?}");
    assert_eq!(events[0][EVENT_DATA_KEY][PAUSED_STATE_KEY], json!(true));
    assert_eq!(
        events[0][EVENT_DATA_KEY][ACTOR_KEY],
        json!(TEST_PAUSE_ACTOR)
    );
    let at = events[0][EVENT_DATA_KEY][AT_KEY].as_str().unwrap();
    assert!(
        chrono::DateTime::parse_from_rfc3339(at).is_ok(),
        "at must be RFC3339: {at}"
    );
}

#[test]
fn set_paused_keeps_the_comments_of_config_yaml() {
    let (d, _bus, vault_ws, _state_ws) = dispatcher_with_snapshot("set-paused-comments", None);
    let path = vault_ws.path().join(TODO_DIR).join(CONFIG_FILE);
    let original = "# header comment kept
dispatcher:
  # why paused
  paused: false
  poll_seconds: 30 # trailing note
";
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, original).unwrap();

    d.set_paused(true, TEST_PAUSE_ACTOR).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    for kept in [
        "# header comment kept",
        "# why paused",
        "# trailing note",
        "poll_seconds: 30",
    ] {
        assert!(text.contains(kept), "`{kept}` lost: {text}");
    }
    assert!(text.contains("paused: true"), "{text}");
}

#[test]
fn set_paused_same_value_is_a_noop_without_event() {
    let (d, bus, _vault_ws, _state_ws) = dispatcher_with_snapshot("set-paused-noop", None);
    d.set_paused(true, TEST_PAUSE_ACTOR).unwrap();

    let outcome = d.set_paused(true, TEST_PAUSE_ACTOR).unwrap();

    assert!(outcome);
    assert_eq!(d.state_json()[PAUSED_STATE_KEY], json!(true));
    assert_eq!(paused_changed_events(&bus).len(), 1);
}

#[test]
fn set_paused_resume_emits_a_second_event_with_paused_false() {
    let (d, bus, _vault_ws, _state_ws) = dispatcher_with_snapshot("set-paused-off", None);
    d.set_paused(true, TEST_PAUSE_ACTOR).unwrap();

    let outcome = d.set_paused(false, TEST_PAUSE_ACTOR).unwrap();

    assert!(!outcome);
    assert_eq!(d.state_json()[PAUSED_STATE_KEY], json!(false));
    let events = paused_changed_events(&bus);
    assert_eq!(events.len(), 2, "pause then resume: {events:?}");
    assert_eq!(events[1][EVENT_DATA_KEY][PAUSED_STATE_KEY], json!(false));
    assert_eq!(
        events[1][EVENT_DATA_KEY][ACTOR_KEY],
        json!(TEST_PAUSE_ACTOR)
    );
}

#[test]
fn set_paused_without_vault_fails_and_emits_nothing() {
    let (d, _state_ws, _skill_ws) = test_dispatcher(PathBuf::new());

    let outcome = d.set_paused(true, TEST_PAUSE_ACTOR);

    assert!(outcome.is_err());
    assert!(paused_changed_events(&d.bus).is_empty());
    assert_eq!(d.state_json()[PAUSED_STATE_KEY], json!(false));
}

#[test]
fn each_rate_limit_pattern_is_matched_case_insensitively_in_last_text() {
    let _ws = TempWorkspace::new("dispatcher-rate-limit");
    let (d, _ws_state, _ws_skill) = test_dispatcher(_ws.path().to_path_buf());
    let cfg = aidp_tickets::config::BudgetConfig::default();

    for pattern in &cfg.rate_limit_patterns {
        let mut rec = sample_session_record("s-rl", Origin::new("dispatcher"));
        rec.last_text = Some(format!("Oops: {}", pattern.to_uppercase()));
        rec.exit_code = None;
        assert!(
            d.session_looks_rate_limited(&rec, &cfg),
            "pattern {pattern:?}, matched case-insensitively (uppercased in the text), must \
             yield rate_limited: true"
        );
    }
}

#[test]
fn non_zero_exit_with_low_utilization_and_neutral_text_is_not_rate_limited() {
    // 10% on the 5h window — well under the default 80% ceiling.
    let snap = rate_limit_snapshot("ok", 0.10, 5 * 3600);
    let (d, _bus, _vault_ws, _state_ws) = dispatcher_with_snapshot("false-positive", Some(snap));

    let mut rec = sample_session_record("s-fail", Origin::new("dispatcher"));
    rec.exit_code = Some(1);
    rec.last_text = Some("the tests failed on line 42, check the assertion".to_string());

    let cfg = aidp_tickets::config::BudgetConfig::default();
    assert!(
        !d.session_looks_rate_limited(&rec, &cfg),
        "a low-utilization non-zero exit with neutral text must not be classified as \
         rate-limited — this is the false positive `budget_cfg.max_utilization_5h` exists \
         to avoid"
    );
}

// -------------------------------------------------------------------
// Lot 10a follow-up: `aidp ticket start --force` during a wait
// (`try_forced_during_wait` / `tick_local_forced`).
// -------------------------------------------------------------------

/// A `Dispatcher` wired to a real, empty vault AND a real `skill_root`
/// declaring one template (`dev-force-test`, `inputs: prompt`) —
/// everything `try_forced_during_wait` needs to build and execute a
/// real `Launch` end to end: `TicketStore::open` on the vault,
/// `plan::build_launch` against the template catalog, and
/// `SessionManager::start` against a real (but idle) project
/// directory. Seeded directly into `"waiting"` mode with a deadline
/// minutes away — the same shortcut
/// `leaving_wait_emits_resumed_and_reopens_the_budget` uses to seed
/// `self.state` by hand rather than computing a real budget verdict:
/// simpler, and this lot's forced path does not care which verdict
/// produced the wait, only that one is in force.
///
/// Returns `(dispatcher, bus, vault_ws, state_ws, skill_ws)` — keep all
/// workspaces alive as long as they are needed.
fn dispatcher_waiting_with_template(
    tag: &str,
) -> (
    Dispatcher,
    Arc<Bus>,
    TempWorkspace,
    TempWorkspace,
    TempWorkspace,
) {
    let vault_ws = TempWorkspace::new(&format!("dispatcher-force-vault-{tag}"));
    let state_ws = TempWorkspace::new(&format!("dispatcher-force-state-{tag}"));
    let skill_ws = TempWorkspace::new(&format!("dispatcher-force-skill-{tag}"));
    let vault_root = vault_ws.path();
    let state_dir = state_ws.path();
    let skill_root = skill_ws.path();

    let templates = skill_root.join("templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(
        templates.join("dev-force-test.yaml"),
        "id: dev-force-test\ninputs:\n  prompt: {}\n",
    )
    .unwrap();

    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(state_dir));
    let sessions = SessionManager::load(state_dir, bus.clone(), projects.clone());

    let state = DispatcherState {
        mode: "waiting".to_string(),
        wait_until: millis_to_rfc3339(now_ms() + 5 * 60_000),
        slots: SlotsState {
            used: 0,
            max: 5,
            test_used: 0,
            test_max: 1,
        },
        ..DispatcherState::default()
    };

    let d = Dispatcher {
        vault_root: vault_root.to_path_buf(),
        sessions,
        launcher: None,
        flows: Mutex::new(None),
        projects,
        bus: bus.clone(),
        state: Mutex::new(state),
        skill_root: Some(skill_root.to_path_buf()),
        last_sync: Mutex::new(None),
        catalog_cache: Mutex::new(None),
        tick_lock: Mutex::new(()),
        warned_no_vault: AtomicBool::new(false),
        first_tick: AtomicBool::new(true),
        worktree_ops: WorktreeOps::system(),
    };
    (d, bus, vault_ws, state_ws, skill_ws)
}

/// Plant `<vault>/TODO/<service>/<key>.md` in `todo`, routed to
/// `service` and `dev-force-test` — the one template
/// `dispatcher_waiting_with_template` declares.
fn write_force_test_ticket(vault_root: &Path, key: &str, service: &str) {
    write_test_ticket(vault_root, key, service, Status::Todo, false);
}

/// Plant a `dev-force-test` ticket in `status`, optionally flagged as a manual start.
fn write_test_ticket(vault_root: &Path, key: &str, service: &str, status: Status, manual: bool) {
    let store = TicketStore::open(vault_root).expect("open store for test ticket");
    let now = Utc::now();
    let mut ticket = aidp_tickets::model::Ticket {
        key: key.to_string(),
        source: aidp_tickets::model::Source::Local,
        r#type: aidp_tickets::model::IssueType::Task,
        summary: key.to_string(),
        status,
        priority: aidp_tickets::model::Priority::Medium,
        priority_rank: aidp_tickets::model::Priority::Medium.rank(),
        service: service.to_string(),
        template: Some("dev-force-test".to_string()),
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created: now,
        updated: now,
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot: aidp_tickets::model::Autopilot {
            manual_priority: manual.then_some(true),
            ..aidp_tickets::model::Autopilot::default()
        },
        tags: Vec::new(),
    };
    store
        .save(&mut ticket, "", &serde_yaml::Mapping::new())
        .expect("save test ticket");
}

#[test]
fn it_forced_start_during_wait_launches_and_is_journalled_forced_over_budget() {
    let _claude = stub_claude_bin();
    let (d, _bus, _vault_ws, _state_ws, _skill_ws) = dispatcher_waiting_with_template("launch");

    let _project_ws = TempWorkspace::new("dispatcher-force-project-launch");
    let project_dir = _project_ws.path();
    let project = d.projects.add(&project_dir.to_string_lossy());

    write_force_test_ticket(d.vault_root.as_path(), "FORCE-1", &project.name);
    // A second, equally eligible candidate on its OWN project — proof
    // that the forced path never reaches `build_plan`/`plan::plan` at
    // all: it must not appear anywhere in the report, not even skipped.
    let _other_project_ws = TempWorkspace::new("dispatcher-force-other-project");
    let other_project_dir = _other_project_ws.path();
    let other_project = d.projects.add(&other_project_dir.to_string_lossy());
    write_force_test_ticket(d.vault_root.as_path(), "OTHER-1", &other_project.name);

    let report = d.tick_local_forced("manual-start", "FORCE-1");

    assert_eq!(
        report["mode"], "waiting",
        "the dispatcher must still report `mode: waiting` afterwards — only the forced \
         ticket jumped the queue, the underlying wait is unchanged: {report}"
    );
    assert_eq!(
        report["launched"],
        json!(["FORCE-1"]),
        "the forced ticket must actually launch: {report}"
    );
    assert_eq!(
        report["skipped"],
        json!([]),
        "no other ticket may even be MENTIONED this tick — `build_plan`/`plan::plan` must \
         never run for a forced-during-wait tick: {report}"
    );

    let events = ledger::read(d.vault_root.as_path(), Some("FORCE-1"), 10);
    let queued_event = events
        .iter()
        .find(|e| e.to == Status::Queued)
        .expect("a Queued-transition event must be journalled for the forced ticket");
    assert_eq!(
        queued_event.reason.as_deref(),
        Some("forced-over-budget"),
        "the queued-transition event's reason must be `forced-over-budget`, not the normal \
         `dispatch` — this is the exact bug this task fixes: {events:?}"
    );
}

#[test]
fn it_try_forced_during_wait_is_inert_outside_waiting_mode() {
    // Per this lot's contract: outside a wait (or once the wait has
    // elapsed), a force key must be irrelevant — `tick_inner` falls
    // through to the pre-existing lot 9a `manual_priority` queue-jump
    // unchanged. `plan_dispatch.rs::manual_priority_goes_first` already
    // covers that queue-jump itself (unaffected by this lot's diff);
    // what THIS test proves is the seam between the two: this method
    // must return `None` (never intercept the tick) whenever the
    // dispatcher is not currently waiting, so nothing here needs to
    // duplicate that other test.
    let (d, _bus, _vault_ws, _state_ws, _skill_ws) = dispatcher_waiting_with_template("inert");
    {
        let mut s = lock(&d.state);
        s.mode = "normal".to_string();
        s.wait_until = None;
    }
    let store = TicketStore::open(d.vault_root.as_path()).expect("open store");

    assert!(
        d.try_forced_during_wait(&store, "manual-start", "WHATEVER")
            .is_none(),
        "outside waiting mode, the forced path must never intercept the tick"
    );

    // Same for an elapsed wait: the deadline has passed, so this must
    // also fall through (`waiting_noop`'s own contract, shared here).
    {
        let mut s = lock(&d.state);
        s.mode = "waiting".to_string();
        s.wait_until = Some("2020-01-01T00:00:00.000Z".to_string());
    }
    assert!(
        d.try_forced_during_wait(&store, "manual-start", "WHATEVER")
            .is_none(),
        "an already-elapsed wait must also fall through, exactly like `waiting_noop`"
    );
}

#[test]
fn it_forced_start_during_wait_still_skips_a_busy_project() {
    let _claude = stub_claude_bin();
    let (d, _bus, _vault_ws, _state_ws, _skill_ws) = dispatcher_waiting_with_template("busy");

    let _project_ws = TempWorkspace::new("dispatcher-force-project-busy");
    let project_dir = _project_ws.path();
    let project = d.projects.add(&project_dir.to_string_lossy());

    // Occupy the project with a real (if short-lived) session BEFORE
    // the forced attempt — the one-session-per-project rule is owned by
    // `queue::admit` inside `SessionManager::start`, which this test
    // must go through for real: there is no test seam to fake a
    // "running" record directly (`SessionManager`'s session table is
    // private to `sessions.rs`), and a record loaded from disk is
    // deliberately reclassified `orphaned` by `SessionManager::load`
    // (crash-recovery semantics), so it cannot be pre-seeded on disk
    // either.
    let occupying = d
        .sessions
        .start(StartRequest {
            project_id: project.id.clone(),
            engine: Some("v7".to_string()),
            template_id: Some("dev-force-test".to_string()),
            origin: Some(Origin {
                kind: "dispatcher".to_string(),
                r#ref: Some("OTHER-BUSY".to_string()),
                label: None,
            }),
            ..Default::default()
        })
        .expect("occupying start() must succeed");
    assert!(
        matches!(occupying, Launch::Started(_)),
        "the occupying session must actually start, not queue, for this test to mean anything"
    );

    write_force_test_ticket(d.vault_root.as_path(), "FORCE-BUSY", &project.name);

    let report = d.tick_local_forced("manual-start", "FORCE-BUSY");

    assert_eq!(
        report["launched"],
        json!([]),
        "a forced ticket whose project is already busy must NOT launch — the \
         one-session-per-project rule is never bypassed, even when forced: {report}"
    );
    let skipped = report["skipped"]
        .as_array()
        .expect("skipped must be an array");
    assert_eq!(
        skipped.len(),
        1,
        "exactly the forced ticket must be reported: {report}"
    );
    assert_eq!(skipped[0]["key"], "FORCE-BUSY");
    assert_eq!(
        skipped[0]["reason"], "queue-race",
        "the forced attempt loses the admission race to the already-running session and \
         must be reported honestly, not silently dropped: {report}"
    );

    let records = d.sessions.list(None, None, 0);
    let for_project: Vec<_> = records
        .iter()
        .filter(|r| r.project_id == project.id)
        .collect();
    assert_eq!(
        for_project.len(),
        1,
        "no second session may ever have been created for the busy project: {for_project:?}"
    );
}

#[test]
fn it_unrouted_ticket_is_skipped_with_a_stable_reason_and_the_remedy_in_detail() {
    let (d, _bus, _vault_ws, _state_ws, _skill_ws) = dispatcher_waiting_with_template("unrouted");

    write_force_test_ticket(d.vault_root.as_path(), "UNROUTED-1", "no-such-project");

    let report = d.tick_local_forced("manual-start", "UNROUTED-1");

    let skipped = report["skipped"]
        .as_array()
        .expect("skipped must be an array");
    assert_eq!(
        skipped.len(),
        1,
        "the unrouted ticket must be reported skipped, not silently dropped: {report}"
    );
    assert_eq!(
        skipped[0]["reason"], "needs-routing",
        "reason must stay the stable slug so a caller can match on it without parsing prose: \
         {report}"
    );
    let detail = skipped[0]["detail"]
        .as_str()
        .expect("detail must carry the remedy text");
    assert!(
        detail.contains("no-such-project") && detail.contains("aidp ticket route"),
        "detail must name the missing project and the remedy command: {detail:?}"
    );
}

// -------------------------------------------------------------------
// Lot 9a/9b bug fix: `POST /api/tickets/:key/start` with `force: true`
// must also bypass a config-level `dispatcher.paused`, not just a
// budget wait — and a refused start (paused, no force; no free slot;
// ...) must never be a silent 200.
// -------------------------------------------------------------------

/// Same shape as [`dispatcher_waiting_with_template`] but seeded with
/// `self.state.mode` left at its (non-`"waiting"`) default, and
/// `TODO/config.yaml` written with `dispatcher.paused: true` — so
/// `try_forced_during_wait` is inert (wrong mode) and the tick reaches
/// the `cfg.paused` gate instead.
fn dispatcher_paused_with_template(
    tag: &str,
) -> (
    Dispatcher,
    Arc<Bus>,
    TempWorkspace,
    TempWorkspace,
    TempWorkspace,
) {
    let (d, bus, vault_ws, state_ws, skill_ws) = dispatcher_waiting_with_template(tag);
    {
        let mut s = lock(&d.state);
        s.mode = "normal".to_string();
        s.wait_until = None;
    }
    // `TicketStore::open` creates `TODO/` (and a default `config.yaml`)
    // on first use — do that once so the directory exists before this
    // overwrites the file.
    TicketStore::open(d.vault_root.as_path()).expect("open store to create TODO/");
    std::fs::write(
        d.vault_root.join("TODO").join("config.yaml"),
        "dispatcher:\n  paused: true\n  slots: 5\n",
    )
    .unwrap();
    (d, bus, vault_ws, state_ws, skill_ws)
}

#[test]
fn it_forced_start_during_pause_launches_and_is_journalled_forced_over_pause() {
    let _claude = stub_claude_bin();
    let (d, _bus, _vault_ws, _state_ws, _skill_ws) =
        dispatcher_paused_with_template("pause-launch");

    let _project_ws = TempWorkspace::new("dispatcher-force-project-pause-launch");
    let project_dir = _project_ws.path();
    let project = d.projects.add(&project_dir.to_string_lossy());
    write_force_test_ticket(d.vault_root.as_path(), "FORCE-PAUSE-1", &project.name);

    let report = d.tick_local_forced("manual-start", "FORCE-PAUSE-1");

    assert_eq!(
        report["mode"], "paused",
        "the dispatcher must still report `mode: paused` afterwards — only the forced \
         ticket jumped the pause, the pause itself is unchanged: {report}"
    );
    assert_eq!(
        report["launched"],
        json!(["FORCE-PAUSE-1"]),
        "force: true must let this one ticket launch even while the dispatcher is paused: \
         {report}"
    );

    let events = ledger::read(d.vault_root.as_path(), Some("FORCE-PAUSE-1"), 10);
    let queued_event = events
        .iter()
        .find(|e| e.to == Status::Queued)
        .expect("a Queued-transition event must be journalled for the forced ticket");
    assert_eq!(
        queued_event.reason.as_deref(),
        Some("forced-over-pause"),
        "the queued-transition event's reason must be `forced-over-pause`, not the normal \
         `dispatch`: {events:?}"
    );

    let records = d.sessions.list(None, None, 0);
    assert!(
        records
            .iter()
            .any(|r| r.project_id == project.id && r.status == SessionStatus::Running),
        "a real session must actually be running for the forced ticket's project: \
         {records:?}"
    );
}

#[test]
fn it_forced_start_during_pause_still_respects_the_slot_ceiling() {
    let _claude = stub_claude_bin();
    let (d, _bus, _vault_ws, _state_ws, _skill_ws) =
        dispatcher_paused_with_template("pause-no-slot");
    std::fs::write(
        d.vault_root.join("TODO").join("config.yaml"),
        "dispatcher:\n  paused: true\n  slots: 0\n",
    )
    .unwrap();

    let _project_ws = TempWorkspace::new("dispatcher-force-project-pause-no-slot");
    let project_dir = _project_ws.path();
    let project = d.projects.add(&project_dir.to_string_lossy());
    write_force_test_ticket(d.vault_root.as_path(), "FORCE-PAUSE-2", &project.name);

    let report = d.tick_local_forced("manual-start", "FORCE-PAUSE-2");

    assert_eq!(
        report["launched"],
        json!([]),
        "with `slots: 0`, force must waive the PAUSE but never the slot ceiling itself: \
         {report}"
    );
    let skipped = report["skipped"]
        .as_array()
        .expect("skipped must be an array");
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0]["key"], "FORCE-PAUSE-2");
    assert_eq!(skipped[0]["reason"], "no-slot");
}

#[test]
fn it_non_forced_start_during_pause_is_a_reportable_noop_not_launched() {
    // This is the seam `manual_start_ticket` in `http.rs` relies on to
    // turn a bare pause into an explicit 423 instead of a silent 200:
    // without a force key, `tick_local` must report `mode: "paused"`
    // with an EMPTY `launched`/`skipped` — the ticket was never even
    // looked at — so the HTTP layer can tell "never considered" (bare
    // pause) apart from "considered and declined" (a `skipped` entry).
    let (d, _bus, _vault_ws, _state_ws, _skill_ws) =
        dispatcher_paused_with_template("pause-no-force");

    let _project_ws = TempWorkspace::new("dispatcher-force-project-pause-no-force");
    let project_dir = _project_ws.path();
    let project = d.projects.add(&project_dir.to_string_lossy());
    write_force_test_ticket(d.vault_root.as_path(), "NOFORCE-1", &project.name);

    let report = d.tick_local("manual-start");

    assert_eq!(report["mode"], "paused");
    assert_eq!(report["launched"], json!([]));
    assert_eq!(report["skipped"], json!([]));
}

/// A `Dispatcher` wired exactly like [`dispatcher_waiting_with_template`],
/// but in `"normal"` mode with `TODO/config.yaml`'s
/// `dispatcher.permission_mode` set to a value `SessionManager::start`
/// rejects — so a real tick's launch attempt fails deterministically inside
/// `start()`, before any process spawns and without depending on a real
/// `claude` binary on the host.
fn dispatcher_normal_with_bad_permission_mode(
    tag: &str,
) -> (
    Dispatcher,
    Arc<Bus>,
    TempWorkspace,
    TempWorkspace,
    TempWorkspace,
) {
    let (d, bus, vault_ws, state_ws, skill_ws) = dispatcher_waiting_with_template(tag);
    {
        let mut s = lock(&d.state);
        s.mode = "normal".to_string();
        s.wait_until = None;
    }
    // `TicketStore::open` creates `TODO/` (and a default `config.yaml`) on
    // first use — do that once so the directory exists before this
    // overwrites the file.
    TicketStore::open(d.vault_root.as_path()).expect("open store to create TODO/");
    std::fs::write(
        d.vault_root.join("TODO").join("config.yaml"),
        "dispatcher:\n  permission_mode: not-a-real-mode\n  slots: 5\n",
    )
    .unwrap();
    (d, bus, vault_ws, state_ws, skill_ws)
}

#[test]
fn it_start_failed_is_recorded_with_reason_and_non_empty_detail() {
    let (d, _bus, _vault_ws, _state_ws, _skill_ws) =
        dispatcher_normal_with_bad_permission_mode("start-failed");

    let _project_ws = TempWorkspace::new("dispatcher-start-failed-project");
    let project_dir = _project_ws.path();
    let project = d.projects.add(&project_dir.to_string_lossy());
    write_force_test_ticket(d.vault_root.as_path(), "START-FAIL-1", &project.name);

    let report = d.tick_local("test");

    let skipped = report["skipped"]
        .as_array()
        .expect("skipped must be an array");
    assert_eq!(
        skipped.len(),
        1,
        "the ticket must be attempted and fail: {report}"
    );
    assert_eq!(skipped[0]["key"], "START-FAIL-1");
    assert_eq!(skipped[0]["reason"], "start-failed");
    let detail = skipped[0]["detail"].as_str().unwrap_or_default();
    assert!(
        !detail.is_empty(),
        "detail must explain the underlying start() failure: {report}"
    );

    // The persisted `.state/dispatcher.json` carries the same shape.
    let state_file = dispatcher_state_file(d.vault_root.as_path());
    let persisted: Value =
        serde_json::from_str(&std::fs::read_to_string(&state_file).unwrap()).unwrap();
    let persisted_skipped = persisted["lastPlan"]["skipped"]
        .as_array()
        .expect("lastPlan.skipped must be an array");
    assert_eq!(persisted_skipped.len(), 1);
    assert_eq!(persisted_skipped[0]["reason"], "start-failed");
    assert!(persisted_skipped[0]["detail"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
}

mod run_correlation {
    use crate::bus::Bus;
    use crate::dispatcher::Dispatcher;
    use crate::projects::Projects;
    use crate::runs::resolve_run;
    use crate::sessions::SessionManager;
    use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner};
    use aidp_testkit::{EnvGuard, TempWorkspace};
    use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
    use aidp_tickets::store::TicketStore;
    use serde_json::{json, Value};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const VAULT_ENV: &str = "AIDP_VAULT";
    const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";
    const REPO: &str = "corr-repo";
    const FEATURE: &str = "corr-feature";
    const RUN_A: &str = "20260930-100000-aaaa";
    const RUN_B: &str = "20260930-100010-bbbb";
    const RUN_UNKNOWN: &str = "20260930-235959-zzzz";
    const RUN_A_STARTED: &str = "2026-09-30T10:00:00Z";
    const RUN_B_STARTED: &str = "2026-09-30T10:00:10Z";
    const WORKSPACE: &str = "C:/code/corr-repo";
    const TICKET_KEY: &str = "PIXEL-4242";
    const TEMPLATE_ID: &str = "dev-corr-test";
    const TEMPLATE_BODY: &str = "id: dev-corr-test\ninputs:\n  prompt: {}\n";
    const SESSIONS_MODULE_PATH: &str = "aidp_gateway::sessions";
    const MANUAL_START: &str = "manual-start";
    const FIELD_TARGET: &str = "target";
    const FIELD_FIELDS: &str = "fields";
    const FIELD_RUN_ID: &str = "run_id";
    const FIELD_TICKET_KEY: &str = "ticket_key";
    const FIELD_SESSION_ID: &str = "session_id";
    const FIELD_LAUNCHED: &str = "launched";
    const OBSERVE_TIMEOUT: Duration = Duration::from_secs(20);
    const POLL_INTERVAL: Duration = Duration::from_millis(50);

    /// Writes the `run-state.json` of one run of [`REPO`] in `vault`.
    fn write_run(vault: &Path, run_id: &str, started_at: &str) {
        let dir = vault
            .join("Pipelines")
            .join(REPO)
            .join(FEATURE)
            .join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        let state = json!({
            "run_id": run_id,
            "pipeline_path": "templates/dev-aidp.yaml",
            "pipeline_id": "dev-aidp",
            "workspace_root": WORKSPACE,
            "run_dir": dir.to_string_lossy(),
            "started_at": started_at,
            "repo": REPO,
            "feature": FEATURE,
            "vault_root": vault.to_string_lossy(),
            "stages": [],
        });
        std::fs::write(
            dir.join("run-state.json"),
            serde_json::to_string(&state).unwrap(),
        )
        .unwrap();
    }

    /// A captured run id wins over the run that started nearest in time.
    #[test]
    fn a_captured_run_id_beats_the_closest_start_time() {
        let vault = TempWorkspace::new("corr-vault");
        let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
        write_run(vault.path(), RUN_A, RUN_A_STARTED);
        write_run(vault.path(), RUN_B, RUN_B_STARTED);

        let found = resolve_run(WORKSPACE, Some(RUN_B)).unwrap();

        assert_eq!(found.run_id, RUN_B);
    }

    /// A run id absent from the vault resolves to nothing.
    #[test]
    fn an_unknown_run_id_resolves_to_nothing() {
        let vault = TempWorkspace::new("corr-vault-unknown");
        let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
        write_run(vault.path(), RUN_A, RUN_A_STARTED);

        let found = resolve_run(WORKSPACE, Some(RUN_UNKNOWN));

        assert!(found.is_none());
    }

    /// Appends every event of the process to one file so any thread is captured.
    fn shared_log_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "aidp-gateway-test-log-{}.jsonl",
            std::process::id()
        ))
    }

    /// Installs the process-wide JSON subscriber once; later calls are no-ops.
    fn install_shared_subscriber() {
        static INSTALLED: std::sync::Once = std::sync::Once::new();
        INSTALLED.call_once(|| {
            let path = shared_log_path();
            let subscriber = tracing_subscriber::fmt()
                .json()
                .with_writer(move || {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .expect("shared log file")
                })
                .finish();
            let _ = tracing::subscriber::set_global_default(subscriber);
        });
    }

    /// The JSON log lines written so far by the process.
    fn shared_log_lines() -> Vec<serde_json::Value> {
        std::fs::read_to_string(shared_log_path())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// A user message whose tool result is the `aidp run start` envelope of `run_id`.
    fn run_start_stream(run_id: &str) -> Vec<u8> {
        let envelope = json!({ "run_id": run_id, "run_dir": "x" }).to_string();
        let message = json!({
            "type": "user",
            "message": { "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": envelope }
            ]}
        });
        format!("{message}\n").into_bytes()
    }

    /// Saves a routed, ready ticket for `service` using [`TEMPLATE_ID`].
    fn seed_ticket(vault: &Path, service: &str) {
        let now = chrono::Utc::now();
        let mut ticket = Ticket {
            key: TICKET_KEY.to_string(),
            source: Source::Local,
            r#type: IssueType::Task,
            summary: TICKET_KEY.to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            priority_rank: Priority::Medium.rank(),
            service: service.to_string(),
            template: Some(TEMPLATE_ID.to_string()),
            track: None,
            labels: Vec::new(),
            components: Vec::new(),
            epic: None,
            assignee: None,
            created: now,
            updated: now,
            github: None,
            route: None,
            flow: None,
            milestone: None,
            autopilot: Autopilot::default(),
            tags: Vec::new(),
        };
        TicketStore::open(vault)
            .unwrap()
            .save(&mut ticket, "", &serde_yaml::Mapping::new())
            .unwrap();
    }

    /// The first sessions event carrying a non-empty `run_id`.
    fn session_event_with_run_id() -> Option<Value> {
        shared_log_lines().into_iter().find(|line| {
            line[FIELD_TARGET] == SESSIONS_MODULE_PATH
                && line[FIELD_FIELDS][FIELD_RUN_ID] == json!(RUN_B)
                && line[FIELD_FIELDS][FIELD_TICKET_KEY] == json!(TICKET_KEY)
        })
    }

    /// A launched ticket logs its run id, ticket key and session id.
    #[test]
    fn it_a_launched_ticket_logs_run_id_ticket_key_and_session_id() {
        install_shared_subscriber();

        let vault = TempWorkspace::new("corr-dispatch-vault");
        let state = TempWorkspace::new("corr-dispatch-state");
        let skill = TempWorkspace::new("corr-dispatch-skill");
        let project_dir = TempWorkspace::new("corr-dispatch-project");
        let templates = skill.path().join("templates");
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(templates.join("dev-corr-test.yaml"), TEMPLATE_BODY).unwrap();
        let _env = EnvGuard::with(&[
            (VAULT_ENV, vault.path().to_str()),
            (SKILL_ROOT_ENV, skill.path().to_str()),
        ]);

        let bus = Arc::new(Bus::new());
        let projects = Arc::new(Projects::load(state.path()));
        let project = projects.add(project_dir.path().to_str().unwrap());
        let runner: Arc<dyn ProcessRunner> =
            Arc::new(ScriptedProcessRunner::new().with_spawn_stdout(run_start_stream(RUN_B)));
        let sessions =
            SessionManager::load_with_runner(state.path(), bus.clone(), projects.clone(), runner);
        let dispatcher = Dispatcher::new(vault.path().to_path_buf(), sessions, projects, bus);
        seed_ticket(vault.path(), &project.name);

        let report = dispatcher.tick_local(MANUAL_START);

        assert_eq!(report[FIELD_LAUNCHED], json!([TICKET_KEY]), "{report}");
        let deadline = Instant::now() + OBSERVE_TIMEOUT;
        let event = loop {
            if let Some(event) = session_event_with_run_id() {
                break event;
            }
            assert!(Instant::now() < deadline, "no sessions event with a run id");
            std::thread::sleep(POLL_INTERVAL);
        };
        assert_eq!(event[FIELD_FIELDS][FIELD_RUN_ID], json!(RUN_B));
        assert_eq!(event[FIELD_FIELDS][FIELD_TICKET_KEY], json!(TICKET_KEY));
        assert!(event[FIELD_FIELDS][FIELD_SESSION_ID]
            .as_str()
            .is_some_and(|session_id| !session_id.is_empty()));
    }
}

const LAUNCH_TEMPLATE: &str = "dev";
const LAUNCH_TRACK: &str = "dev";
const LAUNCH_SERVICE: &str = "svc";
const LAUNCH_KEY: &str = "LOC-7";
const LAUNCH_PROJECT: &str = "project-1";
const LAUNCH_SUMMARY: &str = "summary";
const LAUNCH_INPUTS: &str = "inputs.json";

const LAUNCH_RUN_ID: &str = "20261007-145303-fd8c";
const LAUNCH_BRANCH: &str = "feat/LOC-7";

fn sample_launch() -> plan::Launch {
    plan::Launch {
        key: LAUNCH_KEY.to_string(),
        service: LAUNCH_SERVICE.to_string(),
        template: LAUNCH_TEMPLATE.to_string(),
        track: LAUNCH_TRACK.to_string(),
        demand: plan::Demand::Issue { number: 7 },
        manual: false,
        detail: None,
        flow: None,
        test: false,
    }
}

fn sample_request(engine: Option<&str>) -> StartRequest {
    dispatcher_start_request(
        LAUNCH_PROJECT,
        &sample_launch(),
        LAUNCH_SUMMARY,
        Path::new(LAUNCH_INPUTS),
        &aidp_tickets::config::DispatcherConfig::default(),
        None,
        engine,
    )
}

fn sample_todo_ticket() -> aidp_tickets::model::Ticket {
    let now = Utc::now();
    aidp_tickets::model::Ticket {
        key: LAUNCH_KEY.to_string(),
        source: aidp_tickets::model::Source::Local,
        r#type: aidp_tickets::model::IssueType::Task,
        summary: LAUNCH_SUMMARY.to_string(),
        status: Status::Queued,
        priority: aidp_tickets::model::Priority::Medium,
        priority_rank: aidp_tickets::model::Priority::Medium.rank(),
        service: LAUNCH_SERVICE.to_string(),
        template: Some(LAUNCH_TEMPLATE.to_string()),
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created: now,
        updated: now,
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot: aidp_tickets::model::Autopilot::default(),
        tags: Vec::new(),
    }
}

/// A dispatcher launch is autonomous: its session environment carries the switch.
#[test]
fn dispatcher_start_request_is_autonomous() {
    assert_eq!(sample_request(None).env, autonomous_env());
}

#[test]
fn dispatcher_start_request_has_no_engine_by_default() {
    assert_eq!(sample_request(None).engine, None);
}

#[test]
fn dispatcher_start_request_keeps_the_workflow_engine_when_asked() {
    let request = sample_request(Some(crate::models::constants::engine::ENGINE_WORKFLOW));
    assert_eq!(
        request.engine.as_deref(),
        Some(crate::models::constants::engine::ENGINE_WORKFLOW)
    );
}

#[test]
fn native_launch_records_the_run_id_in_the_ticket_autopilot() {
    let mut ticket = sample_todo_ticket();
    lifecycle::on_started(
        &mut ticket,
        LAUNCH_RUN_ID,
        Some(LAUNCH_RUN_ID.to_string()),
        LAUNCH_BRANCH,
    );
    assert_eq!(ticket.status, Status::InProgress);
    assert_eq!(ticket.autopilot.run_id.as_deref(), Some(LAUNCH_RUN_ID));
    assert_eq!(ticket.autopilot.session_id.as_deref(), Some(LAUNCH_RUN_ID));
}

#[test]
fn only_final_native_statuses_end_a_ticket() {
    assert!(is_live_native(NativeRunStatus::Running));
    assert!(is_live_native(NativeRunStatus::WaitingGate));
    assert!(!is_live_native(NativeRunStatus::Done));
    assert!(matches!(
        native_outcome(NativeRunStatus::Done),
        Some(lifecycle::SessionOutcome::Done)
    ));
}
