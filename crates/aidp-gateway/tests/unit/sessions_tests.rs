use super::*;
use crate::models::enums::PermissionMode;
use aidp_testkit::TempWorkspace;

/// Pause between two probes of a held child's release flag.
const HELD_CHILD_POLL: Duration = Duration::from_millis(20);
const OVERSIZE_IMAGE_BYTES: usize = 5 * 1024 * 1024 + 1;
const SCRATCH_NAME: &str = "archive-scratch";
const GIT_PROGRAM: &str = "git";
const GIT_IDENTITY: &[&str] = &[
    "-c",
    "user.name=tester",
    "-c",
    "user.email=tester@example.com",
];

#[test]
fn validators_accept_dispatcher_defaults() {
    let cfg = aidp_tickets::config::DispatcherConfig::default();
    assert!(validate_model(&cfg.model).is_ok());
    assert!(validate_permission_mode(&cfg.permission_mode).is_ok());
    for m in ["sonnet", "opus", "haiku", "claude-opus-4-1"] {
        assert!(validate_model(m).is_ok(), "{m}");
    }
    for e in EFFORTS {
        assert!(validate_effort(e).is_ok(), "{e}");
    }
    for p in PERMISSION_MODES {
        assert!(validate_permission_mode(p).is_ok(), "{p}");
    }
}

#[test]
fn validate_model_rejects_shell_metacharacters() {
    let long = "a".repeat(65);
    for bad in [
        "sonnet\" & echo X & rem \"",
        "a|b",
        "a^b",
        "a<b",
        "a>b",
        "a%b",
        "a b",
        "a\nb",
        "-leading",
        ".leading",
        "",
        long.as_str(),
    ] {
        let e = validate_model(bad).expect_err(bad).to_string();
        assert!(e.contains("model"), "{e}");
        assert!(!e.contains('\n'), "{e}");
    }
    assert!(validate_model(&"a".repeat(64)).is_ok());
}

#[test]
fn validate_effort_error_names_all_values() {
    let e = validate_effort("ultra").unwrap_err().to_string();
    for v in EFFORTS {
        assert!(e.contains(v), "{e}");
    }
}

#[test]
fn validate_permission_mode_error_names_all_values() {
    let e = validate_permission_mode("turbo").unwrap_err().to_string();
    for v in PERMISSION_MODES {
        assert!(e.contains(v), "{e}");
    }
}

#[test]
fn validate_claude_session_id_rejects_metacharacters() {
    assert!(validate_claude_session_id("0b7c2e6a-1d2f-4c7e-9a2b-3c4d5e6f7a8b").is_ok());
    assert!(validate_claude_session_id("x\" & calc").is_err());
}

#[test]
fn claude_command_uses_aidp_claude_bin_not_cmd() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude.cmd");
    std::fs::write(&bin, "@echo off\r\n").unwrap();
    let _g = aidp_testkit::EnvGuard::set("AIDP_CLAUDE_BIN", bin.to_str().unwrap());
    let cmd = claude_command(&["-p".to_string()], dir.path().to_str().unwrap(), &[]).unwrap();
    let prog = cmd.get_program().to_string_lossy().to_string();
    assert!(prog.ends_with("claude.cmd"), "{prog}");
    assert!(prog != "cmd" && prog != "cmd.exe");
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    assert!(!args.iter().any(|a| a.eq_ignore_ascii_case("/C")));
    assert_eq!(args.first().map(String::as_str), Some("-p"));
}

#[test]
fn claude_command_without_claude_errors_naming_escape_hatch() {
    let empty = tempfile::tempdir().unwrap();
    let p = empty.path().to_str().unwrap();
    let _g = aidp_testkit::EnvGuard::with(&[("AIDP_CLAUDE_BIN", None), ("PATH", Some(p))]);
    let e = claude_command(&[], p, &[]).unwrap_err().to_string();
    assert!(e.contains("AIDP_CLAUDE_BIN"), "{e}");
}

#[test]
fn claude_command_rejects_non_file_aidp_claude_bin() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.cmd");
    let _g = aidp_testkit::EnvGuard::set("AIDP_CLAUDE_BIN", missing.to_str().unwrap());
    let e = claude_command(&[], dir.path().to_str().unwrap(), &[])
        .unwrap_err()
        .to_string();
    assert!(e.contains("AIDP_CLAUDE_BIN"), "{e}");
}

#[test]
fn claude_command_rejects_relative_aidp_claude_bin() {
    // `Cargo.toml` exists relative to the test cwd, so only the absolute check rejects it.
    let _g = aidp_testkit::EnvGuard::set("AIDP_CLAUDE_BIN", "Cargo.toml");
    let e = claude_command(&[], ".", &[]).unwrap_err().to_string();
    assert!(e.contains("AIDP_CLAUDE_BIN"), "{e}");
}

/// A fully-populated record so tests don't have to spell out every field.
fn sample_record() -> SessionRecord {
    serde_json::from_value(json!({
        "id": "sess-1",
        "projectId": "proj-uuid-1234",
        "projectName": "Cheat Profile Service",
        "cwd": "/tmp/cheat-profile-service",
        "engine": "v7",
        "templateId": "dev-aidp",
        "target": "PIXEL-9999",
        "prompt": "run pipeline v7 dev-aidp PIXEL-9999",
        "permissionMode": "default",
        "status": "done",
        "startedAt": "2026-09-04T09:00:00Z",
        "endedAt": "2026-09-04T09:05:00Z",
        "turns": 2,
        "origin": { "kind": "ui" },
        "autoClose": false,
        "eventCount": 3,
        "title": "Nightly regression run",
        "pinned": false,
        "archived": false,
        "unread": false,
    }))
    .expect("sample record must deserialize")
}

/// The exact 17 keys found in the session files already on disk in
/// `~/.aidp/gateway/sessions/`, with none of the Lot 1a additions. A
/// field without a serde default would turn this whole record into
/// `None` at load time (see `SessionManager::load`), silently dropping
/// the session from the index, the API and the viewer.
#[test]
fn old_record_json_still_deserializes() {
    let legacy = json!({
        "autoClose": false,
        "cwd": "/tmp/some-project",
        "endedAt": "2026-09-01T10:05:00Z",
        "engine": "v7",
        "eventCount": 12,
        "exitCode": 0,
        "id": "legacy-session-1",
        "model": "claude-sonnet-4",
        "origin": { "kind": "cli" },
        "permissionMode": "default",
        "projectId": "proj-legacy",
        "projectName": "Legacy Project",
        "prompt": "run pipeline v7 dev PIXEL-1",
        "resumedFrom": null,
        "startedAt": "2026-09-01T10:00:00Z",
        "status": "done",
        "turns": 3,
    });
    // Exactly 17 keys — a canary in case a future edit adds one to the fixture.
    assert_eq!(legacy.as_object().unwrap().len(), 17);

    let rec: SessionRecord =
        serde_json::from_value(legacy).expect("legacy 17-key record must still deserialize");
    assert_eq!(rec.title, None);
    assert!(!rec.pinned);
    assert!(!rec.archived);
    assert!(!rec.unread);
}

#[test]
fn lot_1a_record_json_deserializes_without_start_sha() {
    let json = json!({
        "id": "sess-2",
        "projectId": "proj-2",
        "projectName": "Some Project",
        "cwd": "/tmp/some-project",
        "engine": "v7",
        "prompt": "run pipeline v7 dev PIXEL-2",
        "permissionMode": "default",
        "status": "done",
        "startedAt": "2026-09-05T10:00:00Z",
        "origin": { "kind": "ui" },
        "autoClose": false,
        "eventCount": 1,
        "title": "A run",
        "pinned": true,
        "archived": false,
        "unread": true,
    });
    let rec: SessionRecord =
        serde_json::from_value(json).expect("lot 1a record without startSha must deserialize");
    assert_eq!(rec.start_sha, None);
}

#[test]
fn start_sha_round_trips_and_is_omitted_when_none() {
    let mut rec = sample_record();
    rec.start_sha = Some("abc123".to_string());
    let value = serde_json::to_value(&rec).expect("serialize");
    assert_eq!(value["startSha"], json!("abc123"));

    let rec2: SessionRecord =
        serde_json::from_value(value).expect("record with startSha must deserialize");
    assert_eq!(rec2.start_sha, Some("abc123".to_string()));

    let mut rec3 = sample_record();
    rec3.start_sha = None;
    let value3 = serde_json::to_value(&rec3).expect("serialize");
    assert!(
        !value3.as_object().unwrap().contains_key("startSha"),
        "start_sha=None must be omitted from JSON, got: {value3}"
    );
}

#[test]
fn matches_q_is_case_insensitive_and_hits_each_searched_field() {
    let rec = sample_record();

    // title
    assert!(matches_q(&rec, "NIGHTLY"));
    // templateId
    assert!(matches_q(&rec, "dev-aidp"));
    // target
    assert!(matches_q(&rec, "pixel-9999"));
    // projectName
    assert!(matches_q(&rec, "cheat PROFILE"));

    // projectId (an opaque uuid) must NOT be searched.
    assert!(!matches_q(&rec, "proj-uuid-1234"));
    // Unrelated needle misses.
    assert!(!matches_q(&rec, "something-else"));

    // Empty / whitespace-only needle matches everything.
    assert!(matches_q(&rec, ""));
    assert!(matches_q(&rec, "   "));
}

#[test]
fn render_markdown_covers_every_event_kind_without_panicking() {
    let rec = sample_record();
    let events = vec![
        json!({ "kind": "system", "subtype": "init", "model": "claude-sonnet-4" }),
        json!({ "kind": "text", "text": "TEXT_MARKER hello" }),
        json!({ "kind": "thinking", "text": "THINKING_MARKER pondering" }),
        json!({ "kind": "tool_use", "toolUseId": "t1", "name": "Read", "input": { "path": "a.rs" } }),
        json!({ "kind": "tool_result", "toolUseId": "t1", "content": "RESULT_MARKER ok", "isError": false }),
        json!({ "kind": "turn_end", "result": "TURNEND_MARKER done", "isError": false }),
        json!({ "kind": "question", "toolUseId": "t2", "questions": [{ "question": "Proceed?" }] }),
        json!({ "kind": "user_reply", "toolUseId": "t2", "answers": { "Proceed?": { "answer": "yes" } } }),
        json!({ "kind": "workflow_progress", "runId": "wf_b0f2b915-acb", "phase": "plan", "label": "plan", "agent": "ac282ab00979a4a7a", "status": "started" }),
        json!({ "kind": "user_message", "text": "USERMSG_MARKER go ahead" }),
        json!({ "kind": "log", "text": "LOG_MARKER a log line" }),
        json!({ "kind": "raw", "raw": { "type": "unhandled_type" } }),
        json!({ "kind": "error", "message": "ERROR_MARKER boom" }),
        // Missing fields entirely — must not panic, must not index a Value with [].
        json!({ "kind": "tool_use" }),
        json!({ "kind": "tool_result" }),
        // Unknown kind — must fall back to a fenced JSON dump, not panic.
        json!({ "kind": "mystery_kind", "anything": 1 }),
        // No "kind" at all.
        json!({}),
    ];

    let md = render_markdown(&rec, &events);

    assert!(!md.is_empty());
    assert!(md.contains("Nightly regression run"));
    for marker in [
        "TEXT_MARKER",
        "THINKING_MARKER",
        "RESULT_MARKER",
        "TURNEND_MARKER",
        "USERMSG_MARKER",
        "LOG_MARKER",
        "ERROR_MARKER",
    ] {
        assert!(
            md.contains(marker),
            "expected marker {marker:?} in rendered markdown:\n{md}"
        );
    }
    assert!(md.contains("Tool call: Read"));
    assert!(md.contains("Proceed?"));
    assert!(md.contains("unhandled_type"));
    assert!(md.contains("mystery_kind"));
    assert!(md.contains("wf_b0f2b915-acb"));
    assert!(md.contains("ac282ab00979a4a7a"));
}

/// A `SessionRecord` JSON literal in the shape written before Lot 4a
/// (no queue-related field was ever added to `SessionRecord` itself, but
/// this guards the general contract from `old_record_json_still_deserializes`
/// against a future field slipping in without a serde default).
#[test]
fn lot_4a_record_json_deserializes_without_any_new_field() {
    let json = json!({
        "id": "sess-4a",
        "projectId": "proj-4a",
        "projectName": "Some Project",
        "cwd": "/tmp/some-project",
        "engine": "v7",
        "prompt": "run pipeline v7 dev PIXEL-4",
        "permissionMode": "default",
        "status": "done",
        "startedAt": "2026-09-10T10:00:00Z",
        "origin": { "kind": "ui" },
        "autoClose": false,
        "eventCount": 1,
    });
    let rec: SessionRecord =
        serde_json::from_value(json).expect("a pre-Lot-4a record must still deserialize");
    assert_eq!(rec.id, "sess-4a");
    assert_eq!(rec.status, SessionStatus::Done);
}

#[test]
fn start_request_deserializes_without_priority() {
    let body = json!({
        "projectId": "p1",
        "templateId": "dev",
    });
    let req: StartRequest =
        serde_json::from_value(body).expect("body without priority must deserialize");
    assert_eq!(req.priority, None);
}

#[test]
fn start_request_round_trips_through_json() {
    let req = StartRequest {
        project_id: "p1".to_string(),
        template_id: Some("dev".to_string()),
        priority: Some(3),
        ..Default::default()
    };
    let value = serde_json::to_value(&req).expect("serialize");
    let req2: StartRequest = serde_json::from_value(value).expect("deserialize");
    assert_eq!(req2.project_id, "p1");
    assert_eq!(req2.template_id.as_deref(), Some("dev"));
    assert_eq!(req2.priority, Some(3));
}

#[test]
fn launch_to_json_exposes_id_and_status_for_both_variants() {
    let mut rec = sample_record();
    rec.id = "sess-launch".to_string();
    rec.status = SessionStatus::Running;
    let started = Launch::Started(Box::new(rec.clone()));
    assert_eq!(started.id(), "sess-launch");
    assert_eq!(started.status(), "running");
    assert_eq!(started.record().map(|r| r.id.clone()), Some(rec.id.clone()));
    let started_json = started.to_json();
    assert_eq!(started_json["id"], json!("sess-launch"));
    assert_eq!(started_json["status"], json!("running"));

    let queued = QueuedLaunch {
        id: "q-1".to_string(),
        project_id: "p1".to_string(),
        project_name: "Project One".to_string(),
        template_id: Some("dev".to_string()),
        target: None,
        engine: "v7".to_string(),
        priority: 0,
        status: "queued".to_string(),
        queued_at: "2026-09-10T00:00:00.000Z".to_string(),
        origin: Origin::new("api"),
        workspace_key: None,
        reason: None,
        request: json!({ "secret": "do-not-expose" }),
    };
    let launch = Launch::Queued(Box::new(queued.clone()));
    assert_eq!(launch.id(), "q-1");
    assert_eq!(launch.status(), "queued");
    assert!(launch.record().is_none());
    let queued_json = launch.to_json();
    assert_eq!(queued_json["id"], json!("q-1"));
    assert_eq!(queued_json["status"], json!("queued"));
    assert!(
        queued_json.get("request").is_none(),
        "Launch::Queued.to_json() must never expose the raw request"
    );
}

#[test]
fn purge_orphaned_removes_only_orphaned_records_and_their_files() {
    let _ws = TempWorkspace::new("gateway-purge-test");
    let base = _ws.path();
    let state_dir = base.join("state");
    let sessions_dir = state_dir.join("sessions");
    ensure_dir(&sessions_dir);

    let write_rec = |id: &str, status: SessionStatus| {
        let mut rec = sample_record();
        rec.id = id.to_string();
        rec.status = status;
        write_json(&sessions_dir.join(format!("{id}.json")), &rec).unwrap();
        append_jsonl(
            &sessions_dir.join(format!("{id}.events.jsonl")),
            &json!({ "kind": "log", "text": "x" }),
        );
    };
    // One orphaned, one done, one orphaned — deliberately out of sorted
    // order so the test also proves purge_orphaned sorts its result.
    write_rec("orphan-b", SessionStatus::Orphaned);
    write_rec("keep-me", SessionStatus::Done);
    write_rec("orphan-a", SessionStatus::Orphaned);

    let bus = Arc::new(Bus::new());
    let projects = Arc::new(crate::projects::Projects::load(&state_dir));
    let mgr = SessionManager::load(&state_dir, bus, projects);

    let purged = mgr.purge_orphaned();
    assert_eq!(purged, vec!["orphan-a".to_string(), "orphan-b".to_string()]);

    assert!(mgr.get("keep-me").is_some());
    assert!(mgr.get("orphan-a").is_none());
    assert!(mgr.get("orphan-b").is_none());

    assert!(!sessions_dir.join("orphan-a.json").exists());
    assert!(!sessions_dir.join("orphan-a.events.jsonl").exists());
    assert!(!sessions_dir.join("orphan-b.json").exists());
    assert!(!sessions_dir.join("orphan-b.events.jsonl").exists());
    assert!(sessions_dir.join("keep-me.json").exists());
}

#[test]
fn start_request_deserializes_without_context_from() {
    let body = json!({
        "projectId": "p1",
        "templateId": "dev",
    });
    let req: StartRequest =
        serde_json::from_value(body).expect("body without contextFrom must deserialize");
    assert_eq!(req.context_from, None);
}

#[test]
fn session_record_deserializes_without_chain_fields() {
    // A pre-Lot-6a record: the exact 17-key legacy shape used by
    // `old_record_json_still_deserializes`, with none of the Lot 6a
    // additions.
    let legacy = json!({
        "autoClose": false,
        "cwd": "/tmp/some-project",
        "endedAt": "2026-09-01T10:05:00Z",
        "engine": "v7",
        "eventCount": 12,
        "exitCode": 0,
        "id": "legacy-session-1",
        "model": "claude-sonnet-4",
        "origin": { "kind": "cli" },
        "permissionMode": "default",
        "projectId": "proj-legacy",
        "projectName": "Legacy Project",
        "prompt": "run pipeline v7 dev PIXEL-1",
        "resumedFrom": null,
        "startedAt": "2026-09-01T10:00:00Z",
        "status": "done",
        "turns": 3,
    });
    let rec: SessionRecord =
        serde_json::from_value(legacy).expect("a pre-Lot-6a record must still deserialize");
    assert_eq!(rec.context_from, None);
    assert_eq!(rec.chain_depth, None);
}

#[test]
fn build_prompt_unchanged_without_context() {
    let req = StartRequest {
        project_id: "p1".to_string(),
        template_id: Some("dev-aidp".to_string()),
        target: Some("PIXEL-1".to_string()),
        context_from: None,
        ..Default::default()
    };
    let prompt = build_prompt(&req, "v7", &skill_generation());
    let expected = format!(
        "/run-pipeline-{} dev-aidp PIXEL-1\nThe template and target above are final: do not ask which template or target to use.",
        skill_generation()
    );
    assert_eq!(prompt, expected);
}

/// A `StartRequest` posted without `autoApproveGates` / `inputsFile`
/// (Lot 9a) must still deserialize — existing viewers and scripts that
/// POST `/api/sessions` without them keep working unchanged.
#[test]
fn start_request_without_the_new_fields_still_deserializes() {
    let req: StartRequest = serde_json::from_str(r#"{"projectId":"p1","templateId":"dev-csharp"}"#)
        .expect("a request without the new fields must still deserialize");
    assert_eq!(req.auto_approve_gates, None);
    assert_eq!(req.inputs_file, None);
}

#[test]
fn build_prompt_mentions_the_inputs_file() {
    let path = "C:/v/TODO/.state/launch/PIXEL-1.inputs.json";
    let req = StartRequest {
        project_id: "p1".to_string(),
        template_id: Some("dev-aidp".to_string()),
        target: Some("PIXEL-1".to_string()),
        inputs_file: Some(path.to_string()),
        ..Default::default()
    };
    let prompt = build_prompt(&req, "v7", &skill_generation());
    assert!(prompt.contains(path));
    assert!(prompt.contains("--inputs-file"));

    let req_without = StartRequest {
        project_id: "p1".to_string(),
        template_id: Some("dev-aidp".to_string()),
        target: Some("PIXEL-1".to_string()),
        ..Default::default()
    };
    let prompt_without = build_prompt(&req_without, "v7", &skill_generation());
    assert!(!prompt_without.contains(path));
    assert!(!prompt_without.contains("--inputs-file"));
}

#[test]
fn decide_chain_admits_increments_and_refuses_cycles() {
    assert_eq!(
        decide_chain("child", &[], 0),
        ChainDecision::Admit { depth: 1 }
    );

    // Simulates the real call site (SessionManager::start): the child id
    // is minted before the admission check and passed in as `child_id`,
    // so a self-cycle (this id already present in the parent's ancestry
    // — an id reused/looped scenario) is actually detected here rather
    // than only when it recurs deeper in `ancestry`.
    let ancestry = vec!["a".to_string(), "child".to_string(), "b".to_string()];
    assert_eq!(
        decide_chain("child", &ancestry, 0),
        ChainDecision::Cycle("child".to_string())
    );

    assert_eq!(
        decide_chain("child", &[], MAX_CHAIN_DEPTH),
        ChainDecision::TooDeep {
            depth: MAX_CHAIN_DEPTH + 1,
            max: MAX_CHAIN_DEPTH
        }
    );
}

#[test]
fn resolve_cap_uses_override_when_set_and_parsable() {
    assert_eq!(resolve_cap(Some("7".to_string())), 7);
}

#[test]
fn resolve_cap_falls_back_to_default_for_unparsable_or_absent() {
    assert_eq!(
        resolve_cap(Some("not-a-number".to_string())),
        MAX_CHAIN_DEPTH
    );
    assert_eq!(resolve_cap(None), MAX_CHAIN_DEPTH);
}

#[test]
fn inherit_budget_prefers_request_then_parent() {
    assert_eq!(inherit_budget(Some(5.0), Some(2.0)), Some(2.0));
    assert_eq!(inherit_budget(Some(5.0), None), Some(5.0));
    assert_eq!(inherit_budget(None, None), None);
}

/// Delay the waiter may take before the test thread must get the lock.
const LOCK_ACQUIRE_DEADLINE: Duration = Duration::from_millis(500);
/// Delay the waiter may take to return once its child is killed.
const WAITER_RETURN_DEADLINE: Duration = Duration::from_secs(2);
/// Pause between two lock or completion probes in these tests.
const PROBE_INTERVAL: Duration = Duration::from_millis(10);

/// A child that sleeps for a minute, far longer than any test deadline.
fn long_running_child() -> Child {
    let mut command = if cfg!(windows) {
        let mut powershell = Command::new("powershell");
        powershell.args(["-NoProfile", "-Command", "Start-Sleep 60"]);
        powershell
    } else {
        let mut sleep = Command::new("sleep");
        sleep.arg("60");
        sleep
    };
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn long-running child")
}

/// A child that exits with code 0 right away.
fn instantly_successful_child() -> Child {
    let mut command = if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "exit 0"]);
        cmd
    } else {
        Command::new("true")
    };
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn instantly successful child")
}

#[test]
fn it_wait_for_exit_releases_the_child_lock_between_polls() {
    let child = Arc::new(Mutex::new(long_running_child()));
    let waited_child = child.clone();
    let (done_sender, done_receiver) = std::sync::mpsc::channel();
    let waiter = thread::spawn(move || {
        let code = wait_for_exit(&waited_child);
        let _ = done_sender.send(());
        code
    });

    let lock_deadline = std::time::Instant::now() + LOCK_ACQUIRE_DEADLINE;
    let mut acquired = None;
    while acquired.is_none() && std::time::Instant::now() < lock_deadline {
        acquired = child.try_lock().ok();
        if acquired.is_none() {
            thread::sleep(PROBE_INTERVAL);
        }
    }
    let mut guard = acquired.expect("the waiter must not hold the child lock across the wait");
    guard.kill().expect("kill the long-running child");
    drop(guard);

    done_receiver
        .recv_timeout(WAITER_RETURN_DEADLINE)
        .expect("the waiter must return once its child is killed");
    waiter.join().expect("waiter thread must not panic");
}

#[test]
fn it_wait_for_exit_returns_the_exit_code_of_a_successful_child() {
    let child = Mutex::new(instantly_successful_child());
    assert_eq!(wait_for_exit(&child), Some(0));
}

#[test]
fn dequeue_next_starts_nothing_once_shutdown_has_begun() {
    let workspace = TempWorkspace::new("gateway-shutdown-dequeue-test");
    let state_dir = workspace.path().join("state");
    let project_dir = workspace.path().join("project");
    ensure_dir(&state_dir);
    ensure_dir(&project_dir);
    let missing_claude = workspace.path().join("missing-claude.cmd");
    let _env = aidp_testkit::EnvGuard::set("AIDP_CLAUDE_BIN", missing_claude.to_str().unwrap());

    let bus = Arc::new(Bus::new());
    let projects = Arc::new(crate::projects::Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let manager = SessionManager::load(&state_dir, bus, projects);
    manager.queue.enqueue(QueuedLaunch {
        id: "queued-during-shutdown".to_string(),
        project_id: project.id.clone(),
        project_name: project.name.clone(),
        template_id: None,
        target: None,
        engine: "prompt".to_string(),
        priority: 0,
        status: "queued".to_string(),
        queued_at: now_iso(),
        origin: Origin::new("api"),
        workspace_key: None,
        reason: None,
        request: json!({ "projectId": project.id, "prompt": "hello" }),
    });

    manager.shutdown();
    manager.dequeue_next(&project_workspace(&project.path).key(), &project.id);

    assert_eq!(
        manager.queue_depth(),
        1,
        "the queued launch must stay queued"
    );
    assert!(lock(&manager.index).is_empty(), "no session may start");
}

#[test]
fn start_refuses_a_new_launch_once_shutdown_has_begun() {
    let workspace = TempWorkspace::new("gateway-shutdown-start-test");
    let state_dir = workspace.path().join("state");
    let project_dir = workspace.path().join("project");
    ensure_dir(&state_dir);
    ensure_dir(&project_dir);
    let missing_claude = workspace.path().join("missing-claude.cmd");
    let _env = aidp_testkit::EnvGuard::set("AIDP_CLAUDE_BIN", missing_claude.to_str().unwrap());

    let bus = Arc::new(Bus::new());
    let projects = Arc::new(crate::projects::Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let manager = SessionManager::load(&state_dir, bus, projects);

    manager.shutdown();
    let result = manager.start(StartRequest {
        project_id: project.id.clone(),
        prompt: Some("hello".to_string()),
        ..Default::default()
    });
    let Err(err) = result else {
        panic!("start must refuse a launch once shutdown has begun");
    };

    assert_eq!(err.to_string(), SHUTTING_DOWN_ERROR);
    assert_eq!(manager.queue_depth(), 0, "no launch may be queued either");
    assert!(lock(&manager.index).is_empty(), "no session may start");
}

const WORKSPACE_TAG: &str = "gateway-capture-run-id";
const CAPTURED_SESSION_ID: &str = "sess-capture";
const FIRST_RUN_ID: &str = "20261001-081734-9124";
const LATER_RUN_ID: &str = "20261001-090000-abcd";
const UNKNOWN_SESSION_ID: &str = "sess-unknown";

/// A manager holding one stored session that has no run id yet.
fn manager_with_session(ws: &TempWorkspace) -> Arc<SessionManager> {
    let state_dir = ws.path().join("state");
    let sessions_dir = state_dir.join("sessions");
    ensure_dir(&sessions_dir);
    let mut rec = sample_record();
    rec.id = CAPTURED_SESSION_ID.to_string();
    rec.workflow_run_id = None;
    write_json(
        &sessions_dir.join(format!("{CAPTURED_SESSION_ID}.json")),
        &rec,
    )
    .unwrap();
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(crate::projects::Projects::load(&state_dir));
    SessionManager::load(&state_dir, bus, projects)
}

/// The first captured run id is stored on the session.
#[test]
fn capture_run_id_stores_the_first_run_id() {
    let ws = TempWorkspace::new(WORKSPACE_TAG);
    let mgr = manager_with_session(&ws);
    mgr.capture_run_id(CAPTURED_SESSION_ID, FIRST_RUN_ID.to_string());
    let rec = mgr.get(CAPTURED_SESSION_ID).unwrap();
    assert_eq!(rec.workflow_run_id.as_deref(), Some(FIRST_RUN_ID));
}

/// A later run id does not replace the first one.
#[test]
fn capture_run_id_ignores_a_later_run_id() {
    let ws = TempWorkspace::new(WORKSPACE_TAG);
    let mgr = manager_with_session(&ws);
    mgr.capture_run_id(CAPTURED_SESSION_ID, FIRST_RUN_ID.to_string());
    mgr.capture_run_id(CAPTURED_SESSION_ID, LATER_RUN_ID.to_string());
    let rec = mgr.get(CAPTURED_SESSION_ID).unwrap();
    assert_eq!(rec.workflow_run_id.as_deref(), Some(FIRST_RUN_ID));
}

/// Capturing for an unknown session changes nothing and does not panic.
#[test]
fn capture_run_id_on_unknown_session_is_a_no_op() {
    let ws = TempWorkspace::new(WORKSPACE_TAG);
    let mgr = manager_with_session(&ws);
    mgr.capture_run_id(UNKNOWN_SESSION_ID, FIRST_RUN_ID.to_string());
    assert!(mgr.get(UNKNOWN_SESSION_ID).is_none());
    let rec = mgr.get(CAPTURED_SESSION_ID).unwrap();
    assert!(rec.workflow_run_id.is_none());
}

#[test]
fn record_json_without_workspace_deserializes_with_no_workspace() {
    let rec = sample_record();
    assert_eq!(rec.workspace, None);
    assert!(rec.warnings.is_empty());
    let value = serde_json::to_value(&rec).expect("serialize");
    assert!(value.get("workspace").is_none());
    assert!(value.get("warnings").is_none());
}

#[test]
fn legacy_record_without_workspace_is_keyed_by_its_cwd() {
    let rec = sample_record();
    assert_eq!(
        workspace_key_of(&rec),
        project_workspace(&rec.cwd).key(),
        "a record saved before workspaces keeps the checkout key"
    );
}

#[test]
fn record_workspace_round_trips_through_json() {
    let mut rec = sample_record();
    rec.workspace = Some(Workspace {
        kind: WorkspaceKind::Ticket,
        path: PathBuf::from("/vault/Pipelines/repo/LOC-1/.worktree"),
        branch: Some("feat/LOC-1".to_string()),
        name: None,
    });
    let value = serde_json::to_value(&rec).expect("serialize");
    let back: SessionRecord = serde_json::from_value(value).expect("deserialize");
    assert_eq!(back.workspace, rec.workspace);
    assert_eq!(
        workspace_key_of(&back),
        rec.workspace.as_ref().unwrap().key()
    );
}

#[test]
fn start_request_without_worktree_keeps_the_project_behaviour() {
    let req: StartRequest = serde_json::from_value(json!({
        "projectId": "p1",
        "prompt": "hello",
    }))
    .expect("body without worktree must deserialize");
    assert_eq!(req.worktree, None);
}

#[test]
fn it_start_request_carries_the_worktree_key() {
    let req: StartRequest = serde_json::from_value(json!({
        "projectId": "p1",
        "worktree": "LOC-62",
    }))
    .expect("deserialize");
    assert_eq!(req.worktree.as_deref(), Some("LOC-62"));
}

#[test]
fn worktree_refused_keeps_its_slug_through_anyhow() {
    let error = anyhow::Error::new(WorktreeRefused {
        slug: "branch-checked-out-elsewhere",
        detail: "branch already checked out".to_string(),
    });
    let refused = error
        .downcast_ref::<WorktreeRefused>()
        .expect("typed error survives anyhow");
    assert_eq!(refused.slug, "branch-checked-out-elsewhere");
}

struct HeldStdout(Arc<AtomicBool>);

impl std::io::Read for HeldStdout {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        while !self.0.load(Ordering::SeqCst) {
            thread::sleep(HELD_CHILD_POLL);
        }
        Ok(0)
    }
}

struct RecordedStdin(Arc<Mutex<Vec<u8>>>);

impl Write for RecordedStdin {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct HeldChild {
    released: Arc<AtomicBool>,
    written: Arc<Mutex<Vec<u8>>>,
}

impl SpawnedChild for HeldChild {
    fn take_stdout(&mut self) -> Option<Box<dyn std::io::Read + Send>> {
        Some(Box::new(HeldStdout(self.released.clone())))
    }
    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        Some(Box::new(RecordedStdin(self.written.clone())))
    }
    fn id(&self) -> Option<u32> {
        None
    }
    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(0)
    }
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(self.released.load(Ordering::SeqCst).then_some(0))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        self.released.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Spawns children that stay alive until killed and record what is written to their stdin.
#[derive(Default)]
struct HeldRunner {
    stdin_logs: Mutex<Vec<Arc<Mutex<Vec<u8>>>>>,
}

impl ProcessRunner for HeldRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        aidp_core::process_runner::OsProcessRunner.output(spec)
    }

    fn spawn(&self, _spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        let written = Arc::new(Mutex::new(Vec::new()));
        lock(&self.stdin_logs).push(written.clone());
        Ok(Box::new(HeldChild {
            released: Arc::new(AtomicBool::new(false)),
            written,
        }))
    }
}

fn held_manager(label: &str) -> (TempWorkspace, Arc<SessionManager>, String, Arc<HeldRunner>) {
    let workspace = TempWorkspace::new(label);
    let state_dir = workspace.path().join("state");
    let project_dir = workspace.path().join("project");
    ensure_dir(&state_dir);
    ensure_dir(&project_dir);
    let projects = Arc::new(crate::projects::Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let runner = Arc::new(HeldRunner::default());
    let manager = SessionManager::load_with_runner(
        &state_dir,
        Arc::new(Bus::new()),
        projects,
        runner.clone(),
    );
    (workspace, manager, project.id, runner)
}

fn prompt_request(project_id: &str, mode: Option<&str>) -> StartRequest {
    StartRequest {
        project_id: project_id.to_string(),
        engine: Some(ENGINE_PROMPT.to_string()),
        prompt: Some("hello".to_string()),
        permission_mode: mode.map(str::to_string),
        origin: Some(Origin::new("api")),
        ..Default::default()
    }
}

#[test]
fn auto_title_is_the_first_non_blank_line_cut_to_sixty_characters() {
    assert_eq!(
        auto_title(Some("\n  \n  Fix the build  \nsecond")),
        Some("Fix the build".to_string())
    );
    let long = "x".repeat(100);
    assert_eq!(
        auto_title(Some(&long)).map(|title| title.chars().count()),
        Some(60)
    );
    assert_eq!(auto_title(Some("   \n ")), None);
    assert_eq!(auto_title(None), None);
}

#[test]
fn a_given_title_wins_over_the_prompt() {
    let mut req = StartRequest {
        prompt: Some("derive me".to_string()),
        ..Default::default()
    };
    assert_eq!(resolve_title(&req), Some("derive me".to_string()));
    req.title = Some("  Chosen  ".to_string());
    assert_eq!(resolve_title(&req), Some("Chosen".to_string()));
    req.title = Some("   ".to_string());
    assert_eq!(resolve_title(&req), Some("derive me".to_string()));
}

#[test]
fn kind_is_free_for_a_viewer_prompt_session_only() {
    let mut rec = sample_record();
    assert_eq!(rec.kind(), None);
    rec.engine = ENGINE_PROMPT.to_string();
    assert_eq!(rec.kind(), Some(SessionKind::Free));
    rec.origin = Origin::new("api");
    assert_eq!(rec.kind(), None);
}

#[test]
fn a_record_saved_before_workspaces_loads_without_one() {
    let rec = sample_record();
    assert!(rec.workspace.is_none());
    let value = serde_json::to_value(&rec).expect("serialize");
    assert!(value.get("workspace").is_none());
}

#[test]
fn start_request_reads_the_workspace_title_and_external_fields() {
    let req: StartRequest = serde_json::from_value(json!({
        "projectId": "p",
        "workspace": { "kind": "scratch", "name": "try-it" },
        "title": "T",
        "external": true,
    }))
    .expect("deserialize");
    let workspace = req.workspace.expect("workspace");
    assert_eq!(workspace.kind, WorkspaceKind::Scratch);
    assert_eq!(workspace.name.as_deref(), Some("try-it"));
    assert_eq!(req.title.as_deref(), Some("T"));
    assert_eq!(req.external, Some(true));
}

#[test]
fn reply_content_is_bare_text_without_images_and_blocks_with_them() {
    assert_eq!(
        serde_json::to_value(reply_content(Some("hi"), &[])).unwrap(),
        json!("hi")
    );
    let blocks = crate::attachments::validate_and_build(&[crate::models::Attachment {
        media_type: crate::models::enums::ImageMediaType::Png,
        data_base64: "AAAA".to_string(),
    }])
    .expect("valid attachment");
    let content = serde_json::to_value(reply_content(Some("look"), &blocks)).unwrap();
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content.as_array().map(Vec::len), Some(2));
}

#[test]
fn a_plan_session_starts_beside_a_running_writer() {
    let (_workspace, manager, project_id, _runner) = held_manager("gw-plan-lane");
    let writer = manager
        .start(prompt_request(&project_id, None))
        .expect("writer");
    assert!(matches!(writer, Launch::Started(_)));
    let second_writer = manager
        .start(prompt_request(&project_id, None))
        .expect("second writer");
    assert!(matches!(second_writer, Launch::Queued(_)));
    let plan = manager
        .start(prompt_request(&project_id, Some("plan")))
        .expect("plan session");
    assert!(matches!(plan, Launch::Started(_)));
}

#[test]
fn an_oversize_attachment_is_refused_before_anything_reaches_stdin() {
    use base64::Engine;
    let (_workspace, manager, project_id, runner) = held_manager("gw-attachment-limit");
    let Launch::Started(rec) = manager
        .start(prompt_request(&project_id, None))
        .expect("start")
    else {
        panic!("expected a started session");
    };
    let written_before = lock(&lock(&runner.stdin_logs)[0]).len();
    let data = base64::engine::general_purpose::STANDARD.encode(vec![0u8; OVERSIZE_IMAGE_BYTES]);
    let error = manager
        .reply(
            &rec.id,
            &json!({ "text": "see", "attachments": [{ "mediaType": "image/png", "dataBase64": data }] }),
        )
        .expect_err("oversize image must be refused");
    let refusal = error.downcast_ref::<ApiRefusal>().expect("typed refusal");
    assert_eq!(refusal.status, crate::models::enums::ApiStatus::BadRequest);
    assert!(
        refusal.message.contains("attachments[0]"),
        "{}",
        refusal.message
    );
    assert_eq!(lock(&lock(&runner.stdin_logs)[0]).len(), written_before);
    manager.cancel(&rec.id).ok();
}

#[test]
fn a_queued_writer_is_dequeued_while_a_plan_session_runs_on_its_workspace() {
    let (_workspace, manager, project_id, _runner) = held_manager("gw-dequeue-lane");
    let plan = manager
        .start(prompt_request(&project_id, Some(PERMISSION_MODE_PLAN)))
        .expect("plan session");
    assert!(matches!(plan, Launch::Started(_)));
    let project = manager.projects.get(&project_id).expect("project");
    let workspace_key = project_workspace(&project.path).key();
    manager.queue.enqueue(QueuedLaunch {
        id: "queued-writer".to_string(),
        project_id: project_id.clone(),
        project_name: project.name.clone(),
        template_id: None,
        target: None,
        engine: ENGINE_PROMPT.to_string(),
        priority: 0,
        status: SessionStatus::Queued.as_str().to_string(),
        queued_at: now_iso(),
        origin: Origin::new("api"),
        workspace_key: Some(workspace_key.clone()),
        reason: None,
        request: serde_json::to_value(prompt_request(&project_id, None)).expect("request"),
    });

    manager.dequeue_next(&workspace_key, &project_id);

    assert_eq!(manager.queue_depth(), 0, "the writer must leave the queue");
    assert_eq!(
        manager.running_count(),
        2,
        "the writer must start beside the plan session"
    );
}

fn run_git(directory: &Path, args: &[&str]) {
    let status = Command::new(GIT_PROGRAM)
        .args(GIT_IDENTITY)
        .args(args)
        .current_dir(directory)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

/// A done session record bound to a scratch worktree of a fresh git project.
fn scratch_session(manager: &SessionManager, project_id: &str, id: &str) -> Workspace {
    let project = manager.projects.get(project_id).expect("project");
    let project_dir = PathBuf::from(&project.path);
    let scratch_path = project_dir.join("scratch-tree");
    if !project_dir.join(".git").exists() {
        run_git(&project_dir, &["init", "--initial-branch=master"]);
        run_git(&project_dir, &["commit", "--allow-empty", "-m", "init"]);
        let branch = worktrees::scratch_branch_for(SCRATCH_NAME);
        let path = scratch_path.to_string_lossy().to_string();
        run_git(&project_dir, &["worktree", "add", "-b", &branch, &path]);
    }
    let workspace = Workspace {
        kind: WorkspaceKind::Scratch,
        path: scratch_path,
        branch: None,
        name: Some(SCRATCH_NAME.to_string()),
    };
    let mut record = sample_record();
    record.id = id.to_string();
    record.project_id = project_id.to_string();
    record.cwd = workspace.path.to_string_lossy().to_string();
    record.workspace = Some(workspace.clone());
    lock(&manager.index).insert(id.to_string(), record);
    workspace
}

fn archive(manager: &SessionManager, id: &str) {
    manager
        .patch(id, &json!({ "archived": true }))
        .expect("known session");
}

#[test]
fn it_archiving_a_session_keeps_a_scratch_workspace_another_session_still_uses() {
    let (_workspace, manager, project_id, _runner) = held_manager("gw-archive-shared");
    let workspace = scratch_session(&manager, &project_id, "sess-a");
    scratch_session(&manager, &project_id, "sess-b");

    archive(&manager, "sess-a");

    assert!(workspace.path.exists(), "the shared scratch must be kept");
}

#[test]
fn it_archiving_the_only_session_of_a_scratch_workspace_releases_it() {
    let (_workspace, manager, project_id, _runner) = held_manager("gw-archive-sole");
    let workspace = scratch_session(&manager, &project_id, "sess-a");

    archive(&manager, "sess-a");

    assert!(
        !workspace.path.exists(),
        "the sole owner's scratch must be released"
    );
}

const HELD_WORKSPACE_KEY: &str = "held-workspace";
const HELD_STAGE_NAME: &str = "implement";
const HELD_STAGE_PREFIX: &str = "workflow stage";

/// A workspace held for a named stage reports that stage in its holder message.
#[test]
fn workspace_holder_names_the_holding_stage() {
    let workspace = TempWorkspace::new("holder_stage");
    let state_dir = workspace.child("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    let projects = Arc::new(crate::projects::Projects::load(&state_dir));
    let manager = SessionManager::load(&state_dir, Arc::new(Bus::new()), projects);

    assert!(manager.try_hold_workspace_for_stage(HELD_WORKSPACE_KEY, HELD_STAGE_NAME));
    let holder = manager.workspace_holder(HELD_WORKSPACE_KEY).unwrap();
    assert_eq!(holder, format!("{HELD_STAGE_PREFIX} {HELD_STAGE_NAME}"));
    assert!(holder.contains(HELD_STAGE_NAME));
}

const RUN_DIR: &str = "run-dir";
const OTHER_DIR: &str = "other-dir";
const SOME_TOOL_FLAG: &str = "--model";

/// Every permission mode a native stage session can ask for passes the launch validation.
#[test]
fn validate_permission_mode_accepts_every_stage_permission_mode() {
    for mode in PermissionMode::ALL {
        assert!(
            validate_permission_mode(mode.as_str()).is_ok(),
            "{}",
            mode.as_str()
        );
    }
}

/// One `--add-dir` flag carries every directory, after what the tail already holds.
#[test]
fn with_add_dirs_appends_one_flag_with_every_directory() {
    let tail = vec![SOME_TOOL_FLAG.to_string()];
    let directories = [PathBuf::from(RUN_DIR), PathBuf::from(OTHER_DIR)];

    assert_eq!(
        with_add_dirs(tail, &directories),
        vec![SOME_TOOL_FLAG, ADD_DIR_FLAG, RUN_DIR, OTHER_DIR]
    );
}

/// No directory leaves the tail as it was.
#[test]
fn with_add_dirs_leaves_the_tail_alone_without_directories() {
    let tail = vec![SOME_TOOL_FLAG.to_string()];

    assert_eq!(with_add_dirs(tail.clone(), &[]), tail);
}

/// A headless launch asking no mode or `default` runs `dontAsk`; another mode is kept.
#[test]
fn headless_launches_never_run_the_default_mode() {
    let dont_ask = PermissionMode::DontAsk.as_str();
    let default_mode = PermissionMode::Default.as_str();
    assert_eq!(launch_permission_mode(None, true), dont_ask);
    assert_eq!(launch_permission_mode(Some(""), true), dont_ask);
    assert_eq!(launch_permission_mode(Some(default_mode), true), dont_ask);
    for kept in PermissionMode::ALL
        .into_iter()
        .filter(|mode| *mode != PermissionMode::Default)
    {
        assert_eq!(
            launch_permission_mode(Some(kept.as_str()), true),
            kept.as_str()
        );
    }
    assert_eq!(launch_permission_mode(None, false), default_mode);
    assert_eq!(
        launch_permission_mode(Some(default_mode), false),
        default_mode
    );
}

/// No mode of a headless launch relays prompts; another launch keeps asking.
#[test]
fn headless_launches_never_get_a_prompting_policy() {
    for mode in PermissionMode::ALL {
        let policy = permission_policy(mode.as_str(), true);
        assert!(
            !matches!(policy, PermissionPolicy::Default | PermissionPolicy::Stdio),
            "{mode:?} gave {policy:?}"
        );
    }
    for asking in [PermissionMode::Default, PermissionMode::AcceptEdits] {
        assert_eq!(
            permission_policy(asking.as_str(), true),
            PermissionPolicy::Headless
        );
        assert_eq!(
            permission_policy(asking.as_str(), false),
            PermissionPolicy::Default
        );
    }
    assert_eq!(
        permission_policy(PermissionMode::Plan.as_str(), true),
        PermissionPolicy::Plan
    );
    assert_eq!(
        permission_policy(PermissionMode::DontAsk.as_str(), true),
        PermissionPolicy::Headless
    );
    assert_eq!(
        permission_policy(PermissionMode::DontAsk.as_str(), false),
        PermissionPolicy::Bypass
    );
}

/// A manager with one project, a registered test checkout and an unrelated directory.
fn checkout_roots_fixture(
    label: &str,
) -> (
    TempWorkspace,
    Arc<SessionManager>,
    crate::projects::Project,
    PathBuf,
    PathBuf,
) {
    let workspace = TempWorkspace::new(label);
    let state_dir = workspace.path().join("state");
    let project_dir = workspace.path().join("project");
    let checkout_dir = workspace.path().join("checkout");
    let unrelated_dir = workspace.path().join("unrelated");
    for dir in [&state_dir, &project_dir, &checkout_dir, &unrelated_dir] {
        ensure_dir(dir);
    }
    let projects = Arc::new(crate::projects::Projects::load(&state_dir));
    let added = projects.add(project_dir.to_str().unwrap());
    let project = projects
        .set_test_checkout(&added.id, checkout_dir.to_str())
        .expect("absolute checkout")
        .expect("known project");
    let manager = SessionManager::load(&state_dir, Arc::new(Bus::new()), projects);
    (workspace, manager, project, checkout_dir, unrelated_dir)
}

fn path_workspace(path: &Path) -> WorkspaceRequest {
    WorkspaceRequest {
        kind: WorkspaceKind::Path,
        ticket: None,
        name: None,
        path: Some(path.to_string_lossy().into_owned()),
    }
}

#[test]
fn an_explicit_workspace_on_a_project_test_checkout_is_accepted() {
    let (_workspace, manager, project, checkout, _) = checkout_roots_fixture("gw-checkout-ok");
    let resolved = manager
        .resolve_requested_workspace(&path_workspace(&checkout), &project)
        .expect("test checkout is an allowed root");
    assert_eq!(
        resolved.workspace.path,
        checkout.canonicalize().expect("canonical checkout")
    );
}

#[test]
fn an_explicit_workspace_on_an_unrelated_path_is_still_refused() {
    let (_workspace, manager, project, _, unrelated) = checkout_roots_fixture("gw-checkout-no");
    let error = manager
        .resolve_requested_workspace(&path_workspace(&unrelated), &project)
        .err()
        .expect("unrelated path must be refused");
    let refusal = error.downcast_ref::<ApiRefusal>().expect("typed refusal");
    assert_eq!(refusal.status, crate::models::enums::ApiStatus::Forbidden);
}
