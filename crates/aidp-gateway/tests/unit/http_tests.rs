use super::*;
use aidp_testkit::TempWorkspace;

const FIELD_MODE_KEY: &str = "mode";
const FIELD_TEMPLATE_ID_KEY: &str = "templateId";
const DEV_TEMPLATE_ID: &str = "dev";
const EXTERNAL_FLOW_KEY: &str = "TEST-92-run-a";
const EXTERNAL_TICKET_KEY: &str = "TEST-92";
const EXTERNAL_TICKET_SUMMARY: &str = "External work item";
const INSTANCE_STATUS_ROUTE: &str = "/api/gateway/status";
const INSTANCE_WORKTREES_ROUTE: &str = "/api/worktrees";
const INSTANCE_FIRST_DIR: &str = "first-instance";
const INSTANCE_SECOND_DIR: &str = "second-instance";
const INSTANCE_STATE_DIR: &str = "state";
const INSTANCE_VAULT_DIR: &str = "vault";
const INSTANCE_HOME_DIR: &str = "home";
const INSTANCE_HOST: &str = "127.0.0.1";
const INSTANCE_STATUS_VAULT: &str = "vault";
const INSTANCE_HOME_ENV: &str = "HOME";
const INSTANCE_USERPROFILE_ENV: &str = "USERPROFILE";
const INSTANCE_SKILL_ENV: &str = "AIDP_SKILL_ROOT";
const INSTANCE_SEED_MARKER: &str = ".defaults-seeded";
const INSTANCE_STATUS_OK: u16 = 200;
const INSTANCE_RESTRICTED_POLICY: &str = "restrictedLaunchPolicy";
const INSTANCE_HEADLESS_POLICY: &str = "headlessLaunchPolicy";

#[test]
fn it_http_instances_use_their_configured_vaults_when_ambient_vault_is_disabled() {
    let workspace = TempWorkspace::new(INSTANCE_FIRST_DIR);
    let home = workspace.child(INSTANCE_HOME_DIR);
    std::fs::create_dir_all(&home).unwrap();
    let skill = aidp_testkit::fixtures::skill_root();
    let _environment = aidp_testkit::EnvGuard::with(&[
        (INSTANCE_HOME_ENV, Some(home.to_str().unwrap())),
        (INSTANCE_USERPROFILE_ENV, Some(home.to_str().unwrap())),
        (INSTANCE_SKILL_ENV, Some(skill.to_str().unwrap())),
        (
            crate::models::constants::flow_execution::VAULT_ENV,
            Some(crate::models::constants::flow_execution::VAULT_DISABLED),
        ),
    ]);
    let mut handles = Vec::new();
    for name in [INSTANCE_FIRST_DIR, INSTANCE_SECOND_DIR] {
        let root = workspace.child(name);
        let state_dir = root.join(INSTANCE_STATE_DIR);
        let vault_root = root.join(INSTANCE_VAULT_DIR);
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::write(state_dir.join(INSTANCE_SEED_MARKER), "").unwrap();
        let listener = TcpListener::bind((INSTANCE_HOST, 0)).unwrap();
        let handle = serve_on(
            listener,
            HttpConfig {
                state_dir,
                vault_root: vault_root.clone(),
                hook_token: String::new(),
                ui_token: String::new(),
                read_only: false,
                allowed_origins: Vec::new(),
            },
        )
        .unwrap();
        let base = format!("http://{INSTANCE_HOST}:{}", handle.port());
        let status: Value = ureq::get(&format!("{base}{INSTANCE_STATUS_ROUTE}"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        assert_eq!(
            status[INSTANCE_STATUS_VAULT],
            vault_root.to_string_lossy().as_ref()
        );
        assert_eq!(
            status[crate::models::constants::runtime_api::CAPABILITIES][INSTANCE_RESTRICTED_POLICY],
            true
        );
        assert_eq!(
            status[crate::models::constants::runtime_api::CAPABILITIES][INSTANCE_HEADLESS_POLICY],
            true
        );
        assert_eq!(
            ureq::get(&format!("{base}{INSTANCE_WORKTREES_ROUTE}"))
                .call()
                .unwrap()
                .status(),
            INSTANCE_STATUS_OK
        );
        handles.push(handle);
    }
    for handle in handles {
        handle.stop();
    }
}

#[test]
fn it_external_flow_locator_resolves_execution_workspace_without_local_ticket() {
    let workspace = TempWorkspace::new(EXTERNAL_FLOW_KEY);
    let vault = workspace.child("vault");
    let state_dir = workspace.child("state");
    let project_path = workspace.child("project");
    std::fs::create_dir_all(&project_path).unwrap();
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_path.to_str().unwrap());
    let repo = aidp_core::vault::repo_segment(&project_path);
    let flows = crate::flows::store::FlowStore::new(vault.clone(), state_dir.clone());
    let mut state = aidp_flows::FlowState::new(EXTERNAL_FLOW_KEY, aidp_flows::FlowDef::default());
    state.ticket = json!({
        crate::models::constants::flow_execution::TICKET_KEY: EXTERNAL_TICKET_KEY,
        crate::models::constants::flow_execution::TICKET_PROJECT: project.name,
        crate::models::constants::flow_inputs::TICKET_SUMMARY: EXTERNAL_TICKET_SUMMARY,
    });
    flows
        .save_state(&repo, &state, &crate::time::now_iso())
        .unwrap();
    let locator = ticket_locator(vault.clone(), state_dir, projects);
    let (ticket, path) = locator(EXTERNAL_FLOW_KEY).unwrap();
    assert_eq!(ticket.key, EXTERNAL_TICKET_KEY);
    assert_eq!(ticket.summary, EXTERNAL_TICKET_SUMMARY);
    assert_eq!(path, project_path);
    assert!(TicketStore::open(&vault)
        .unwrap()
        .get(EXTERNAL_TICKET_KEY)
        .is_err());
}

#[test]
fn method_has_body_covers_the_body_bearing_verbs() {
    assert!(method_has_body(&Method::Patch));
    assert!(method_has_body(&Method::Post));
    assert!(method_has_body(&Method::Put));
    assert!(!method_has_body(&Method::Get));
    assert!(!method_has_body(&Method::Delete));
    assert!(!method_has_body(&Method::Options));
}

#[test]
fn cors_methods_advertises_patch() {
    assert!(CORS_METHODS.contains("PATCH"));
}

#[test]
fn parse_ticket_status_accepts_every_documented_alias() {
    for (input, expected) in [
        ("todo", Status::Todo),
        ("  Todo ", Status::Todo),
        ("queued", Status::Queued),
        ("QUEUED", Status::Queued),
        ("in-progress", Status::InProgress),
        ("in_progress", Status::InProgress),
        ("inprogress", Status::InProgress),
        ("  In-Progress ", Status::InProgress),
        ("in-review", Status::InReview),
        ("in_review", Status::InReview),
        ("inreview", Status::InReview),
        ("done", Status::Done),
        ("blocked", Status::Blocked),
        ("  Blocked ", Status::Blocked),
        ("cancelled", Status::Cancelled),
        ("canceled", Status::Cancelled),
    ] {
        assert_eq!(
            parse_ticket_status(input).unwrap_or_else(|e| {
                panic!("`{input}` must parse as {expected:?}, got error: {e}")
            }),
            expected,
            "input `{input}`"
        );
    }
}

#[test]
fn parse_ticket_status_rejects_an_unknown_value() {
    let err = parse_ticket_status("bogus").expect_err("an unknown status must error");
    let msg = err.to_string();
    assert!(msg.contains("unknown"), "error must say `unknown`: {msg}");
    assert!(
        msg.contains("todo") && msg.contains("cancelled"),
        "error must list the expected values: {msg}"
    );
}

#[test]
fn parse_ticket_source_accepts_github_and_local() {
    for (input, expected) in [
        ("github", Source::Github),
        ("  GitHub ", Source::Github),
        ("GITHUB", Source::Github),
        ("local", Source::Local),
        ("  Local ", Source::Local),
    ] {
        assert_eq!(
            parse_ticket_source(input).unwrap_or_else(|e| {
                panic!("`{input}` must parse as {expected:?}, got error: {e}")
            }),
            expected,
            "input `{input}`"
        );
    }
}

#[test]
fn parse_ticket_source_rejects_an_unknown_value() {
    let err = parse_ticket_source("bogus").expect_err("an unknown source must error");
    let msg = err.to_string();
    assert!(msg.contains("unknown"), "error must say `unknown`: {msg}");
}

fn temp_vault(label: &str) -> TempWorkspace {
    TempWorkspace::new(&format!("gateway-http-test-{label}"))
}

#[test]
fn create_local_ticket_requires_service_and_summary() {
    let _ws = temp_vault("create-local-missing-fields");
    let vault = _ws.path();
    let store = TicketStore::open(vault).expect("TicketStore::open must succeed");

    let missing_service = json!({ "summary": "Something" });
    let err = create_local_ticket(&store, &missing_service)
        .expect_err("a body with no service must error");
    assert!(
        err.to_string().contains("service"),
        "error must mention `service`: {err}"
    );

    let missing_summary = json!({ "service": "svc", "allowUnknownService": true });
    let err = create_local_ticket(&store, &missing_summary)
        .expect_err("a body with no summary must error");
    assert!(
        err.to_string().contains("summary"),
        "error must mention `summary`: {err}"
    );
}

#[test]
fn create_local_ticket_succeeds_with_service_and_summary() {
    let _ws = temp_vault("create-local-success");
    let vault = _ws.path();
    let store = TicketStore::open(vault).expect("TicketStore::open must succeed");

    let body = json!({
        "service": "svc",
        "summary": "A fresh local ticket",
        "allowUnknownService": true,
    });
    let v = create_local_ticket(&store, &body).expect("a valid body must succeed");
    let key = v["key"]
        .as_str()
        .expect("response must carry a key")
        .to_string();
    assert!(!key.is_empty(), "key must be non-empty");

    let (ticket, _body, _extra) = store.get(&key).expect("the created ticket must re-read");
    assert_eq!(ticket.status, Status::Todo);
    assert_eq!(ticket.summary, "A fresh local ticket");
}

#[test]
fn auto_approve_gates_from_http_is_allow_listed() {
    let gates = Some(vec![
        "pr".to_string(),
        "merge".to_string(),
        "deploy".to_string(),
    ]);
    assert_eq!(
        sanitize_auto_approve_gates(gates),
        Some(vec!["pr".to_string()]),
        "only the allow-listed gate must survive"
    );

    let none_allowed = Some(vec!["merge".to_string()]);
    assert_eq!(
        sanitize_auto_approve_gates(none_allowed),
        None,
        "an empty filtered result must map to None, not Some(vec![])"
    );

    assert_eq!(
        sanitize_auto_approve_gates(None),
        None,
        "an absent field stays absent"
    );
}

fn segs_of(path: &str) -> Vec<&str> {
    let p = path.trim_start_matches("/api").trim_matches('/');
    if p.is_empty() {
        vec![]
    } else {
        p.split('/').collect()
    }
}

#[test]
fn session_run_route_shape_is_stable() {
    let segs = segs_of("/api/sessions/abc123/run");
    assert_eq!(segs.as_slice(), ["sessions", "abc123", "run"]);
    assert!(matches!(segs.as_slice(), ["sessions", _id, "run"]));
}

#[test]
fn automation_test_route_shape_is_stable_and_distinct_from_run() {
    let segs = segs_of("/api/automations/abc123/test");
    assert_eq!(segs.as_slice(), ["automations", "abc123", "test"]);
    assert!(matches!(segs.as_slice(), ["automations", _id, "test"]));

    let run_segs = segs_of("/api/automations/abc123/run");
    assert_ne!(segs, run_segs);
    assert!(matches!(run_segs.as_slice(), ["automations", _id, "run"]));
}

#[test]
fn usage_route_shape_is_stable() {
    let segs = segs_of("/api/usage");
    assert_eq!(segs.as_slice(), ["usage"]);
}

#[test]
fn usage_rate_limit_route_is_distinct_from_usage() {
    let segs = segs_of("/api/usage/rate-limit");
    assert_eq!(segs.as_slice(), ["usage", "rate-limit"]);
    assert_ne!(segs, segs_of("/api/usage"));
}

#[test]
fn queue_route_shape_is_stable() {
    let segs = segs_of("/api/queue");
    assert_eq!(segs.as_slice(), ["queue"]);
}

#[test]
fn queue_promote_and_cancel_route_shapes_are_stable_and_distinct() {
    let promote = segs_of("/api/queue/q1/promote");
    assert!(matches!(promote.as_slice(), ["queue", _id, "promote"]));

    let cancel = segs_of("/api/queue/q1/cancel");
    assert!(matches!(cancel.as_slice(), ["queue", _id, "cancel"]));

    assert_ne!(promote, cancel);
}

#[test]
fn health_route_shapes_are_stable_and_distinct() {
    let health = segs_of("/api/health");
    assert_eq!(health.as_slice(), ["health"]);

    let cleanup = segs_of("/api/health/cleanup");
    assert_eq!(cleanup.as_slice(), ["health", "cleanup"]);
    assert_ne!(health, cleanup);
}

const TRANSITION_REASON: &str = "moved by hand";
const EVENT_TYPE_FIELD: &str = "type";
const EVENT_DATA_FIELD: &str = "data";

#[test]
fn a_manual_transition_publishes_ticket_status_for_the_viewers_and_the_dispatcher() {
    let _ws = temp_vault("transition-event");
    let store = TicketStore::open(_ws.path()).expect("TicketStore::open must succeed");
    let body = json!({ "service": "svc", "summary": "Move me", "allowUnknownService": true });
    let key = create_local_ticket(&store, &body).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let bus = Bus::new();
    let events = bus.subscribe();

    transition_ticket_http(
        &store,
        &bus,
        &key,
        Status::Queued,
        Some(TRANSITION_REASON.to_string()),
        false,
    )
    .expect("todo -> queued is a legal human transition");

    let event = events.try_recv().expect("the transition is published");
    assert_eq!(event[EVENT_TYPE_FIELD], EVENT_TICKET_STATUS);
    let data = &event[EVENT_DATA_FIELD];
    assert_eq!(data["key"], key.as_str());
    assert_eq!(data["status"], Status::Queued.as_str());
    assert_eq!(data["from"], Status::Todo.as_str());
    assert_eq!(data["to"], Status::Queued.as_str());
    assert_eq!(data["reason"], TRANSITION_REASON);
    assert_eq!(data["actor"], Actor::Human.as_str());
}

#[test]
fn a_refused_transition_publishes_nothing() {
    let _ws = temp_vault("transition-refused");
    let store = TicketStore::open(_ws.path()).expect("TicketStore::open must succeed");
    let body = json!({ "service": "svc", "summary": "Stay", "allowUnknownService": true });
    let key = create_local_ticket(&store, &body).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let bus = Bus::new();
    let events = bus.subscribe();
    assert!(transition_ticket_http(&store, &bus, &key, Status::Done, None, false).is_err());
    assert!(events.try_recv().is_err());
}

#[test]
fn manual_start_re_arms_an_already_queued_ticket_instead_of_guarding_it() {
    // Why the branch has to exist: the guard rejects the self-edge, and
    // `force` does not waive it — so a route that guarded an already-queued
    // ticket could never start it again, forced or not.
    assert!(
        model::guard(Status::Queued, Status::Queued, Actor::Human, false).is_err(),
        "`queued` -> `queued` must stay an illegal transition"
    );
    assert!(
        model::guard(Status::Queued, Status::Queued, Actor::Human, true).is_err(),
        "`force` must NOT waive the self-edge — it only covers leaving `in-progress`"
    );

    assert!(
        !manual_start_needs_transition(Status::Queued),
        "a ticket parked in `queued` by an earlier refused start must be re-armable, \
         otherwise the first refusal is a trap door: every retry 409s for good"
    );

    // A start that really does move the ticket still goes through the guard.
    for from in [Status::Todo, Status::Blocked] {
        assert!(
            manual_start_needs_transition(from),
            "`{from}` -> `queued` is a real transition and must stay guarded"
        );
        assert!(
            model::guard(from, Status::Queued, Actor::Human, false).is_ok(),
            "`{from}` -> `queued` is a legal human transition"
        );
    }
}

#[test]
fn tickets_sync_and_start_route_shapes_are_stable_and_distinct() {
    let sync = segs_of("/api/tickets/sync");
    assert_eq!(sync.as_slice(), ["tickets", "sync"]);
    assert!(matches!(sync.as_slice(), ["tickets", "sync"]));

    let start = segs_of("/api/tickets/LOC-1/start");
    assert_eq!(start.as_slice(), ["tickets", "LOC-1", "start"]);
    assert!(matches!(start.as_slice(), ["tickets", _key, "start"]));

    assert_ne!(sync, start);

    // Every other `[key, action]` ticket route shares the same shape as
    // `start` and must stay distinct from the `sync` literal too.
    for action in ["cancel", "route", "status"] {
        let path = format!("/api/tickets/LOC-1/{action}");
        let segs = segs_of(&path);
        assert!(matches!(segs.as_slice(), ["tickets", "LOC-1", a] if *a == action));
        assert_ne!(segs, sync);
    }
}

#[test]
fn tickets_list_and_single_route_shapes_are_distinct() {
    let list = segs_of("/api/tickets");
    assert_eq!(list.as_slice(), ["tickets"]);

    let one = segs_of("/api/tickets/LOC-1");
    assert_eq!(one.as_slice(), ["tickets", "LOC-1"]);
    assert_ne!(list, one);
}

#[test]
fn dispatcher_route_shapes_are_stable_and_distinct() {
    let status = segs_of("/api/dispatcher");
    assert_eq!(status.as_slice(), ["dispatcher"]);

    let plan = segs_of("/api/dispatcher/plan");
    assert_eq!(plan.as_slice(), ["dispatcher", "plan"]);
    assert_ne!(status, plan);

    let budget = segs_of("/api/dispatcher/budget");
    assert_eq!(budget.as_slice(), ["dispatcher", "budget"]);
    assert_ne!(budget, status);
    assert_ne!(budget, plan);

    let pause = segs_of("/api/dispatcher/pause");
    let resume = segs_of("/api/dispatcher/resume");
    let tick = segs_of("/api/dispatcher/tick");
    assert_eq!(pause.as_slice(), ["dispatcher", "pause"]);
    assert_eq!(resume.as_slice(), ["dispatcher", "resume"]);
    assert_eq!(tick.as_slice(), ["dispatcher", "tick"]);
    assert_ne!(pause, resume);
    assert_ne!(pause, tick);
    assert_ne!(resume, tick);
    assert_ne!(budget, pause);
    assert_ne!(budget, resume);
    assert_ne!(budget, tick);
}

const NO_ORIGIN: OriginCheck<'static> = OriginCheck {
    origin: "",
    allowed: &[],
};

#[test]
fn origin_gate_decision_table() {
    let allowed = vec!["http://localhost:5173".to_string()];
    let evil = OriginCheck {
        origin: "https://evil.example",
        allowed: &allowed,
    };
    let good = OriginCheck {
        origin: "http://localhost:5173",
        allowed: &allowed,
    };
    let empty = OriginCheck {
        origin: "http://localhost:5173",
        allowed: &[],
    };
    let s = &["sessions"][..];
    let post = &Method::Post;
    assert_eq!(
        gate(evil, true, "", "", false, post, s),
        Gate::ForbiddenOrigin
    );
    assert_eq!(gate(NO_ORIGIN, true, "", "", false, post, s), Gate::Allow);
    assert_eq!(gate(good, true, "", "", false, post, s), Gate::Allow);
    assert_eq!(
        gate(evil, true, "", "", false, &Method::Get, s),
        Gate::Allow
    );
    assert_eq!(
        gate(empty, true, "", "", false, post, s),
        Gate::ForbiddenOrigin
    );
    assert_eq!(
        gate(evil, true, "", "", false, &Method::Options, s),
        Gate::Allow
    );
}

#[test]
fn parse_allowed_origins_trims_and_drops_empties() {
    assert_eq!(
        parse_allowed_origins(" http://a , ,http://b,"),
        vec!["http://a".to_string(), "http://b".to_string()]
    );
    assert!(parse_allowed_origins("").is_empty());
}

#[test]
fn off_loopback_with_no_token_is_unauthorized() {
    assert_eq!(
        gate(NO_ORIGIN, false, "", "", false, &Method::Get, &["sessions"]),
        Gate::Unauthorized
    );
}

#[test]
fn off_loopback_with_wrong_token_is_unauthorized_right_token_is_allow() {
    assert_eq!(
        gate(
            NO_ORIGIN,
            false,
            "secret",
            "wrong",
            false,
            &Method::Get,
            &["sessions"]
        ),
        Gate::Unauthorized
    );
    assert_eq!(
        gate(
            NO_ORIGIN,
            false,
            "secret",
            "secret",
            false,
            &Method::Get,
            &["sessions"]
        ),
        Gate::Allow
    );
}

#[test]
fn off_loopback_hooks_route_is_allowed_without_ui_token() {
    assert_eq!(
        gate(
            NO_ORIGIN,
            false,
            "",
            "",
            false,
            &Method::Post,
            &["hooks", "gh"]
        ),
        Gate::Allow
    );
}

#[test]
fn loopback_with_no_token_is_allow() {
    assert_eq!(
        gate(NO_ORIGIN, true, "", "", false, &Method::Get, &["sessions"]),
        Gate::Allow
    );
}

#[test]
fn read_only_blocks_mutating_methods_but_not_get_or_hooks() {
    assert_eq!(
        gate(NO_ORIGIN, true, "", "", true, &Method::Post, &["sessions"]),
        Gate::ReadOnly
    );
    assert_eq!(
        gate(
            NO_ORIGIN,
            true,
            "",
            "",
            true,
            &Method::Patch,
            &["sessions", "x"]
        ),
        Gate::ReadOnly
    );
    assert_eq!(
        gate(NO_ORIGIN, true, "", "", true, &Method::Get, &["sessions"]),
        Gate::Allow
    );
    assert_eq!(
        gate(
            NO_ORIGIN,
            true,
            "",
            "",
            true,
            &Method::Post,
            &["hooks", "gh"]
        ),
        Gate::Allow
    );
}

#[test]
fn options_off_loopback_with_no_token_is_allow() {
    assert_eq!(
        gate(
            NO_ORIGIN,
            false,
            "secret",
            "",
            false,
            &Method::Options,
            &["sessions"]
        ),
        Gate::Allow
    );
}

#[test]
fn hook_token_does_not_satisfy_the_ui_gate() {
    // A token that equals the *hook* token but not the UI token must not open a UI route:
    // two tokens stay two tokens.
    assert_eq!(
        gate(
            NO_ORIGIN,
            false,
            "ui-secret",
            "hook-secret",
            false,
            &Method::Get,
            &["sessions"]
        ),
        Gate::Unauthorized
    );
}

#[test]
fn urldecode_table_is_frozen() {
    // Measured on the pre-LOC-25 code: the byte rewrite changes none of them.
    assert_eq!(urldecode("%41B"), "AB");
    assert_eq!(urldecode("%zz"), "%zz");
    assert_eq!(urldecode("abc%"), "abc%");
    assert_eq!(urldecode("%é"), "%é");
    assert_eq!(urldecode("a+b"), "a b");
    // Panicked with "end byte index 3 is not a char boundary" before the fix.
    let _ = urldecode("%aé");
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(512))]
    #[test]
    fn urldecode_never_panics(s in "[%+0-9a-fA-F\u{e9}\u{c0}\u{6f22} \u{a0}]{0,32}") {
        let _ = urldecode(&s);
    }
}

#[test]
fn bound_body_accepts_max_and_rejects_max_plus_one() {
    let max = MAX_BODY_BYTES;
    assert_eq!(bound_body(None, max), BodyBound::Accept);
    assert_eq!(bound_body(Some(max), max), BodyBound::Accept);
    assert_eq!(bound_body(Some(max + 1), max), BodyBound::TooLarge);
    assert_eq!(bound_body(Some(0), 0), BodyBound::Accept);
}

#[test]
fn parse_max_body_bytes_falls_back_to_default() {
    for raw in [
        None,
        Some(""),
        Some("abc"),
        Some("0"),
        Some("-1"),
        Some("99999999999999999999999"),
    ] {
        assert_eq!(parse_max_body_bytes(raw), MAX_BODY_BYTES, "{raw:?}");
    }
    assert_eq!(parse_max_body_bytes(Some("1024")), 1024);
    assert_eq!(parse_max_body_bytes(Some(" 1024 ")), 1024);
}

#[test]
fn parse_max_inflight_requests_falls_back_to_default() {
    for raw in [None, Some(""), Some("many"), Some("0"), Some("-4")] {
        assert_eq!(
            parse_max_inflight_requests(raw),
            MAX_INFLIGHT_REQUESTS,
            "{raw:?}"
        );
    }
    assert_eq!(parse_max_inflight_requests(Some("32")), 32);
    assert_eq!(parse_max_inflight_requests(Some(" 32 ")), 32);
}

#[test]
fn send_frame_reports_a_stalled_client_once_the_grace_elapses() {
    let (tx, _unread_receiver) = sync_channel::<Vec<u8>>(1);
    let grace = Duration::from_millis(120);
    assert_eq!(
        send_frame_with(&tx, b"first".to_vec(), grace),
        SendOutcome::Sent
    );
    let started = Instant::now();
    assert_eq!(
        send_frame_with(&tx, b"second".to_vec(), grace),
        SendOutcome::Stalled
    );
    assert!(
        started.elapsed() >= grace,
        "the grace must elapse before giving up"
    );
}

#[test]
fn send_frame_reports_a_closed_stream_when_the_receiver_is_gone() {
    let (tx, receiver) = sync_channel::<Vec<u8>>(1);
    drop(receiver);
    assert_eq!(
        send_frame_with(&tx, b"frame".to_vec(), Duration::from_millis(120)),
        SendOutcome::Closed
    );
}

#[test]
fn chan_reader_ends_cleanly_when_the_sender_is_dropped() {
    let (tx, rx) = sync_channel::<Vec<u8>>(SSE_CHANNEL_CAPACITY);
    drop(tx);
    let mut reader = ChanReader {
        rx,
        buf: Vec::new(),
        pos: 0,
    };
    let mut out = [0u8; 16];
    assert_eq!(reader.read(&mut out).expect("clean end of stream"), 0);
}

/// The `(status, code)` an `api_err()` reply carries, or a panic naming what
/// was returned instead — `Reply` derives no `Debug`, so a match arm cannot
/// print the unexpected variant itself.
fn reply_status_and_code(reply: Reply) -> (u16, String) {
    match reply {
        Reply::Json(status, v) => (status, v["code"].as_str().unwrap_or_default().to_string()),
        _ => panic!("expected a Reply::Json"),
    }
}

#[test]
fn api_err_maps_every_session_error_variant_to_its_documented_status() {
    let cases: Vec<(anyhow::Error, u16, &str)> = vec![
        (
            SessionError::NotFound("x".to_string()).into(),
            404,
            "not_found",
        ),
        (
            SessionError::Conflict("x".to_string()).into(),
            409,
            "conflict",
        ),
        (SessionError::ShuttingDown.into(), 503, "shutting_down"),
        (
            SessionError::Unauthorized("x".to_string()).into(),
            401,
            "unauthorized",
        ),
        (SessionError::Config("x".to_string()).into(), 503, "config"),
        (
            SessionError::Upstream("x".to_string()).into(),
            502,
            "upstream",
        ),
        (
            SessionError::Io(std::io::Error::other("x")).into(),
            500,
            "io",
        ),
    ];
    for (err, expected_status, expected_code) in cases {
        let (status, code) = reply_status_and_code(api_err(err));
        assert_eq!(status, expected_status, "code {expected_code}");
        assert_eq!(code, expected_code);
    }
}

#[test]
fn api_err_maps_every_ticket_error_variant_to_its_documented_status() {
    let cases: Vec<(anyhow::Error, u16, &str)> = vec![
        (
            TicketError::NotFound("x".to_string()).into(),
            404,
            "not_found",
        ),
        (
            TicketError::Conflict("x".to_string()).into(),
            409,
            "conflict",
        ),
        (
            TicketError::Unauthorized("x".to_string()).into(),
            401,
            "unauthorized",
        ),
        (TicketError::Config("x".to_string()).into(), 503, "config"),
        (
            TicketError::Upstream("x".to_string()).into(),
            502,
            "upstream",
        ),
        (
            TicketError::Io(std::io::Error::other("x")).into(),
            500,
            "io",
        ),
    ];
    for (err, expected_status, expected_code) in cases {
        let (status, code) = reply_status_and_code(api_err(err));
        assert_eq!(status, expected_status, "code {expected_code}");
        assert_eq!(code, expected_code);
    }
}

#[test]
fn api_err_maps_every_integration_error_variant_to_its_documented_status() {
    let cases: Vec<(anyhow::Error, u16, &str)> = vec![
        (
            IntegrationError::NotFound("x".to_string()).into(),
            404,
            "not_found",
        ),
        (
            IntegrationError::Conflict("x".to_string()).into(),
            409,
            "conflict",
        ),
        (
            IntegrationError::Unauthorized("x".to_string()).into(),
            401,
            "unauthorized",
        ),
        (
            IntegrationError::Config {
                var: "AIDP_JIRA_BASE",
                remedy: "set it",
            }
            .into(),
            503,
            "config",
        ),
        (
            IntegrationError::Upstream("x".to_string()).into(),
            502,
            "upstream",
        ),
        (
            IntegrationError::Io(std::io::Error::other("x")).into(),
            500,
            "io",
        ),
    ];
    for (err, expected_status, expected_code) in cases {
        let (status, code) = reply_status_and_code(api_err(err));
        assert_eq!(status, expected_status, "code {expected_code}");
        assert_eq!(code, expected_code);
    }
}

#[test]
fn api_err_falls_back_to_bad_request_for_an_untyped_anyhow_error() {
    let (status, code) = reply_status_and_code(api_err(anyhow::anyhow!("plain failure")));
    assert_eq!(status, 400);
    assert_eq!(code, "bad_request");
}

#[test]
fn api_err_finds_a_typed_error_wrapped_in_context() {
    let wrapped: anyhow::Error =
        anyhow::Error::new(SessionError::NotFound("session x".to_string()))
            .context("while resuming");
    let (status, code) = reply_status_and_code(api_err(wrapped));
    assert_eq!(status, 404);
    assert_eq!(code, "not_found");
}

#[test]
fn api_err_maps_a_bare_io_error_in_the_chain_to_500_io() {
    let wrapped: anyhow::Error =
        anyhow::Error::new(std::io::Error::other("disk full")).context("reading");
    let (status, code) = reply_status_and_code(api_err(wrapped));
    assert_eq!(status, 500);
    assert_eq!(code, "io");
}

const OTHER_SERVICE: &str = "other";
const DEV_AIDP_TEMPLATE: &str = "dev-aidp";
const DEEP_TRACK: &str = "deep";
const BOGUS_TRACK: &str = "bogus";

/// A routed ticket on `quick` with the `dev` template.
fn routed_ticket() -> Ticket {
    let now = chrono::Utc::now();
    Ticket {
        key: "LOC-1".to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "sample".to_string(),
        status: Status::Todo,
        priority: Priority::Medium,
        priority_rank: 0,
        service: "svc".to_string(),
        template: Some("dev".to_string()),
        track: Some(Track::Quick),
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
    }
}

/// The body fields become typed overrides.
#[test]
fn body_fields_are_read_as_overrides() {
    let overrides = StartOverrides::from_body(&json!({
        "force": true,
        "service": OTHER_SERVICE,
        "template": DEV_AIDP_TEMPLATE,
        "track": DEEP_TRACK
    }))
    .unwrap();
    assert_eq!(overrides.service.as_deref(), Some(OTHER_SERVICE));
    assert_eq!(overrides.template.as_deref(), Some(DEV_AIDP_TEMPLATE));
    assert_eq!(overrides.track, Some(Track::Deep));
}

/// Absent or blank fields carry no override.
#[test]
fn absent_or_blank_fields_are_no_override() {
    let overrides = StartOverrides::from_body(&json!({ "template": "  " })).unwrap();
    assert_eq!(overrides, StartOverrides::default());
}

/// A track outside the closed set is rejected while reading the body.
#[test]
fn unparsable_track_is_an_error() {
    let result = StartOverrides::from_body(&json!({ "track": BOGUS_TRACK }));
    assert!(result.is_err());
}

/// Only the given fields replace the ticket's own values.
#[test]
fn overrides_replace_template_and_track_only_when_given() {
    let mut routed = routed_ticket();
    StartOverrides {
        template: Some(DEV_AIDP_TEMPLATE.to_string()),
        ..StartOverrides::default()
    }
    .apply(&mut routed);
    assert_eq!(routed.template.as_deref(), Some(DEV_AIDP_TEMPLATE));
    assert_eq!(routed.track, Some(Track::Quick));
}

/// The listen address joins the IP and the port for both families.
#[test]
fn socket_addr_joins_ip_and_port() {
    let v4 = socket_addr(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 3001);
    assert_eq!(v4.to_string(), "127.0.0.1:3001");
    let v6 = socket_addr(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), 3001);
    assert_eq!(v6.to_string(), "[::1]:3001");
}

mod leak_guards {
    use crate::http::{serve_on, HttpConfig};
    use aidp_testkit::TempWorkspace;
    use serde_json::{json, Value};
    use std::fs;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::time::Duration;

    const SENTINEL_TOKEN: &str = "s3cr3t-sentinelle";
    const SENTINEL_SECRET: &str = "SENTINELLE-XYZ";
    const HOOK_TOKEN: &str = "hook-secret";
    const UI_TOKEN: &str = "ui-secret";
    const LOOPBACK: &str = "127.0.0.1";
    const DEFAULTS_SEEDED_MARKER: &str = ".defaults-seeded";
    const HEALTH_PATH: &str = "/api/health";
    const METHOD_GET: &str = "GET";
    const STATUS_FIELD: &str = "status";
    const DURATION_FIELD: &str = "duration_ms";
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
    const WEBHOOK_HOST: &str = "example.invalid";
    const RUNS_FILE: &str = "automation-runs.jsonl";
    const STEP_ACTION: &str = "action";
    const API_AUTOMATIONS: &str = "/api/automations";
    const API_EVENTS: &str = "/api/events/recent";
    const KEY_ID: &str = "id";
    const KEY_TRACE: &str = "trace";
    const KEY_URL: &str = "url";
    const KEY_STEP: &str = "step";
    const KEY_DATA: &str = "data";

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

    fn config(state_dir: PathBuf, ui_token: &str) -> HttpConfig {
        HttpConfig {
            state_dir,
            vault_root: PathBuf::new(),
            hook_token: HOOK_TOKEN.to_string(),
            ui_token: ui_token.to_string(),
            read_only: false,
            allowed_origins: vec![],
        }
    }

    fn api_url(port: u16, path: &str) -> String {
        format!("http://{LOOPBACK}:{port}{path}")
    }

    fn webhook_url() -> String {
        format!("https://{WEBHOOK_HOST}/hooks/{SENTINEL_SECRET}")
    }

    fn webhook_automation() -> Value {
        json!({
            "name": "leaky webhook",
            "enabled": false,
            "trigger": { "kind": "webhook" },
            "action": { "kind": "webhook", "url": webhook_url() },
            "delivery": { "kind": "webhook", "url": webhook_url() },
        })
    }

    /// The access log records method, path, status and duration, never the query-string token.
    #[test]
    fn it_access_log_omits_query_string_token() {
        install_shared_subscriber();
        let workspace = TempWorkspace::new("access-log-token");
        let state_dir = workspace.path().to_path_buf();
        fs::write(state_dir.join(DEFAULTS_SEEDED_MARKER), "").expect("defaults marker");
        let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handle = serve_on(listener, config(state_dir, SENTINEL_TOKEN)).expect("serve_on");

        let url = format!("{}?token={SENTINEL_TOKEN}", api_url(port, HEALTH_PATH));
        let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
        let _ = agent.get(&url).call();
        handle.stop();

        let captured = shared_log_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        let access_line = captured.lines().find(|line| {
            line.contains(METHOD_GET)
                && line.contains(HEALTH_PATH)
                && line.contains(STATUS_FIELD)
                && line.contains(DURATION_FIELD)
        });
        assert!(access_line.is_some(), "no access line: {captured}");
        assert!(!captured.contains(SENTINEL_TOKEN), "token leaked in logs");
    }

    /// A failing webhook never leaves its URL path, the credential, on disk or on the bus.
    #[test]
    fn it_failed_webhook_never_persists_or_publishes_its_url_path() {
        let workspace = TempWorkspace::new("gw-webhook-redaction");
        let state_dir = workspace.path().to_path_buf();
        fs::write(state_dir.join(DEFAULTS_SEEDED_MARKER), "").expect("seed marker");
        let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind");
        let handle = serve_on(listener, config(state_dir.clone(), UI_TOKEN)).expect("serve_on");
        let port = handle.port();

        let created: Value = ureq::post(&api_url(port, API_AUTOMATIONS))
            .send_json(webhook_automation())
            .expect("create automation")
            .into_json()
            .expect("automation json");
        let id = created[KEY_ID].as_str().expect("automation id").to_string();

        let run: Value = ureq::post(&api_url(port, &format!("{API_AUTOMATIONS}/{id}/run")))
            .send_json(json!({}))
            .expect("run automation")
            .into_json()
            .expect("run json");
        let action = run[KEY_TRACE]
            .as_array()
            .expect("trace")
            .iter()
            .find(|step| step[KEY_STEP] == STEP_ACTION)
            .expect("action step");
        assert!(action[KEY_DATA][KEY_URL]
            .as_str()
            .expect("url")
            .contains(WEBHOOK_HOST));

        let runs_file = fs::read_to_string(state_dir.join(RUNS_FILE)).expect("runs file");
        assert!(runs_file.contains(WEBHOOK_HOST));
        assert!(
            !runs_file.contains(SENTINEL_SECRET),
            "runs file leaks the secret"
        );

        let events = ureq::get(&api_url(port, API_EVENTS))
            .call()
            .expect("events")
            .into_string()
            .expect("events body");
        assert!(
            !events.contains(SENTINEL_SECRET),
            "event bus leaks the secret"
        );

        handle.stop();
    }
}

const RUN_ID: &str = "run-1";
const FIELD_RUN_ID: &str = "runId";
const FIELD_OK: &str = "ok";
const FIELD_ENGINE_KEY: &str = "engine";

fn native_reply_body() -> Value {
    let reply = launched_reply(crate::models::launched_run::LaunchedRun::Native {
        run_id: RUN_ID.to_string(),
    });
    match reply {
        Reply::Json(status, body) => {
            assert_eq!(status, HTTP_OK);
            body
        }
        _ => panic!("a native launch must answer JSON"),
    }
}

#[test]
fn native_launch_replies_with_the_run_id_and_the_native_mode() {
    let body = native_reply_body();
    assert_eq!(body[FIELD_OK], Value::Bool(true));
    assert_eq!(body[FIELD_RUN_ID], Value::String(RUN_ID.to_string()));
    assert_eq!(
        body[FIELD_MODE_KEY],
        Value::String(
            crate::models::enums::engine::Engine::Native
                .as_str()
                .to_string()
        )
    );
}

#[test]
fn template_start_without_engine_drops_the_engine_key() {
    let body = json!({
        FIELD_ENGINE_KEY: crate::models::constants::engine::ENGINE_V7_ALIAS,
        FIELD_TEMPLATE_ID_KEY: DEV_TEMPLATE_ID,
    });
    let adjusted = crate::client::with_template_engine(body, None);
    assert!(adjusted.get(FIELD_ENGINE_KEY).is_none());
}

#[test]
fn template_start_with_workflow_engine_keeps_the_session_path() {
    let engine = crate::models::constants::engine::ENGINE_WORKFLOW.to_string();
    let adjusted = crate::client::with_template_engine(json!({}), Some(engine.clone()));
    assert_eq!(adjusted[FIELD_ENGINE_KEY], Value::String(engine));
}
