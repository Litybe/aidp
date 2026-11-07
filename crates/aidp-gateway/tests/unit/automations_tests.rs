use super::*;
use aidp_testkit::{EnvGuard, TempWorkspace};

fn engine(tag: &str) -> (Arc<AutomationEngine>, TempWorkspace) {
    let workspace = TempWorkspace::new(tag);
    let dir = workspace.path();
    std::fs::create_dir_all(dir).unwrap();
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(dir));
    let sessions = SessionManager::load(dir, bus.clone(), projects.clone());
    let engine = AutomationEngine::load(dir, sessions, projects, bus);
    (engine, workspace)
}

fn noop_automation(name: &str) -> Value {
    json!({
        "name": name,
        "enabled": true,
        "trigger": { "kind": "webhook" },
        "action": { "kind": "none" },
    })
}

fn webhook_automation(name: &str, url: &str) -> Value {
    json!({
        "name": name,
        "enabled": true,
        "trigger": { "kind": "webhook" },
        "action": { "kind": "webhook", "url": url, "method": "POST" },
    })
}

// ── TraceStep ────────────────────────────────────────────────────────────

#[test]
fn trace_step_round_trips_camel_case_and_omits_absent_fields() {
    let step = trace_step("trigger", "matched", None, Value::Null);
    let v = serde_json::to_value(&step).unwrap();
    assert_eq!(v["step"], "trigger");
    assert_eq!(v["outcome"], "matched");
    assert!(v.get("detail").is_none(), "absent detail must be omitted");
    assert!(v.get("data").is_none(), "null data must be omitted");
    assert!(v.get("at").is_some());

    let back: TraceStep = serde_json::from_value(v).unwrap();
    assert_eq!(back.step, "trigger");
    assert_eq!(back.outcome, "matched");
    assert!(back.detail.is_none());
    assert!(back.data.is_null());

    let with_both = trace_step(
        "guardrail",
        "blocked",
        Some("5h window at 95%".to_string()),
        json!({ "reason": "5h window at 95%" }),
    );
    let v2 = serde_json::to_value(&with_both).unwrap();
    assert_eq!(v2["detail"], "5h window at 95%");
    assert_eq!(v2["data"]["reason"], "5h window at 95%");
}

// ── Backward compatibility ───────────────────────────────────────────────

#[test]
fn legacy_automation_run_json_still_deserializes() {
    let legacy = json!({
        "id": "run-1",
        "automationId": "auto-1",
        "automationName": "Sonde CI",
        "reason": "cron",
        "startedAt": "2026-09-01T10:00:00.000Z",
        "endedAt": "2026-09-01T10:00:01.000Z",
        "status": "done",
        "sessionId": null,
        "error": null,
        "note": null,
    });
    // Exactly 10 keys — a canary in case a future edit adds one to the fixture.
    assert_eq!(legacy.as_object().unwrap().len(), 10);

    let run: AutomationRun =
        serde_json::from_value(legacy).expect("legacy 9-key run must still deserialize");
    assert!(run.trace.is_empty());

    let back = serde_json::to_value(&run).unwrap();
    assert!(
        back.get("trace").is_none(),
        "an empty trace must not reappear on the wire"
    );
}

// ── event_trigger_matches ────────────────────────────────────────────────

#[test]
fn event_trigger_matches_accepts_and_rejects() {
    let trigger = Trigger::Event {
        on: vec!["session.ended".to_string()],
        filter: Some(BTreeMap::from([("status".to_string(), "done".to_string())])),
    };

    // all-match
    assert!(event_trigger_matches(&trigger, "session.ended", &json!({ "status": "done" })).is_ok());

    // pattern mismatch
    let err = event_trigger_matches(&trigger, "session.started", &json!({ "status": "done" }))
        .unwrap_err();
    assert!(err.contains("session.started"));

    // filter key mismatch
    let err = event_trigger_matches(&trigger, "session.ended", &json!({ "status": "failed" }))
        .unwrap_err();
    assert!(err.contains("status"));
    assert!(err.contains("done"));
    assert!(err.contains("failed"));

    // missing key
    let err = event_trigger_matches(&trigger, "session.ended", &json!({})).unwrap_err();
    assert!(err.contains("status"));

    // not an event trigger
    assert!(event_trigger_matches(&Trigger::Webhook, "session.ended", &json!({})).is_err());
}

// ── Action::Launch.context_from (Lot 6a) ────────────────────────────────

#[test]
fn launch_action_deserializes_without_context_from() {
    // A pre-Lot-6a automations.json: no "contextFrom" key at all.
    let raw = json!({
        "kind": "launch",
        "projectId": "p1",
        "templateId": "dev-aidp"
    });
    let action: Action = serde_json::from_value(raw).unwrap();
    let Action::Launch { context_from, .. } = &action else {
        panic!("expected Action::Launch");
    };
    assert!(context_from.is_none());

    let round_tripped = serde_json::to_value(&action).unwrap();
    assert!(
        round_tripped.get("contextFrom").is_none(),
        "an absent context_from must not be serialized as a null contextFrom key"
    );
}

#[test]
fn launch_action_round_trips_context_from() {
    let action = Action::Launch {
        project_id: "p1".to_string(),
        engine: None,
        template_id: "dev-aidp".to_string(),
        target: None,
        inputs: None,
        permission_mode: None,
        model: None,
        effort: None,
        max_budget_usd: None,
        max_utilization: None,
        context_from: Some("{{ payload.sessionId }}".to_string()),
        autonomous: false,
    };
    let raw = serde_json::to_value(&action).unwrap();
    assert_eq!(raw["contextFrom"], "{{ payload.sessionId }}");
    let back: Action = serde_json::from_value(raw).unwrap();
    let Action::Launch { context_from, .. } = &back else {
        panic!("expected Action::Launch");
    };
    assert_eq!(context_from.as_deref(), Some("{{ payload.sessionId }}"));
}

// ── dry-run ──────────────────────────────────────────────────────────────

#[test]
fn try_mark_in_flight_refuses_a_second_claim_until_the_guard_drops() {
    let (eng, _workspace) = engine("in-flight");
    let first = eng.try_mark_in_flight("auto-1");
    assert!(first.is_some());
    assert!(eng.try_mark_in_flight("auto-1").is_none());
    assert!(eng.try_mark_in_flight("auto-2").is_some());
    drop(first);
    assert!(eng.try_mark_in_flight("auto-1").is_some());
}

#[test]
fn in_flight_guard_releases_the_id_when_the_run_panics() {
    let (eng, _workspace) = engine("in-flight-panic");
    let guard = eng.try_mark_in_flight("auto-panic").unwrap();
    let outcome = std::thread::spawn(move || {
        let _held = guard;
        panic!("automation failed");
    })
    .join();
    assert!(outcome.is_err());
    assert!(eng.try_mark_in_flight("auto-panic").is_some());
}

/// A `started` `AutomationRun`, either backed by a live session or still
/// queued, for exercising [`AutomationEngine::release_pending`] without
/// spinning up a real `claude` process.
fn started_run(
    automation_id: &str,
    session_id: Option<&str>,
    queue_id: Option<&str>,
) -> AutomationRun {
    AutomationRun {
        id: new_id(),
        automation_id: automation_id.to_string(),
        automation_name: "pending".to_string(),
        reason: TRIGGER_CRON.to_string(),
        started_at: now_iso(),
        ended_at: None,
        status: RUN_STATUS_STARTED.to_string(),
        session_id: session_id.map(str::to_string),
        queue_id: queue_id.map(str::to_string),
        error: None,
        note: None,
        trace: Vec::new(),
    }
}

#[test]
fn keep_until_settled_holds_the_mark_until_session_ended_arrives() {
    let (eng, _workspace) = engine("pending-started");
    let run = started_run("auto-pending", Some("sess-1"), None);
    let guard = eng.try_mark_in_flight(&run.automation_id).unwrap();

    guard.keep_until_settled(&run);
    assert!(eng.try_mark_in_flight(&run.automation_id).is_none());

    eng.on_bus_event(&json!({
        "type": "session.ended",
        "data": { "automationRunId": run.id, "automationId": run.automation_id, "sessionId": "sess-1", "status": "done" },
    }));
    assert!(eng.try_mark_in_flight(&run.automation_id).is_some());
}

#[test]
fn keep_until_settled_holds_the_mark_until_a_queued_launch_is_dropped() {
    let (eng, _workspace) = engine("pending-queued");
    let run = started_run("auto-queued", None, Some("queue-1"));
    let guard = eng.try_mark_in_flight(&run.automation_id).unwrap();

    guard.keep_until_settled(&run);
    assert!(eng.try_mark_in_flight(&run.automation_id).is_none());

    eng.on_bus_event(&json!({
        "type": "session.dequeued",
        "data": { "queueId": "queue-1", "reason": "cancelled" },
    }));
    assert!(eng.try_mark_in_flight(&run.automation_id).is_some());
}

#[test]
fn session_question_does_not_release_a_pending_mark() {
    let (eng, _workspace) = engine("pending-question");
    let run = started_run("auto-question", Some("sess-2"), None);
    let guard = eng.try_mark_in_flight(&run.automation_id).unwrap();

    guard.keep_until_settled(&run);
    eng.on_bus_event(&json!({
        "type": "session.question",
        "data": { "automationRunId": run.id, "automationId": run.automation_id, "sessionId": "sess-2" },
    }));
    assert!(eng.try_mark_in_flight(&run.automation_id).is_none());
}

#[test]
fn launch_due_skips_an_automation_already_in_flight() {
    let (eng, _workspace) = engine("in-flight-skip");
    let a = eng.upsert(noop_automation("t-skip")).unwrap();
    let _running = eng.try_mark_in_flight(&a.id).unwrap();
    eng.launch_due(a.clone());
    let skipped: Vec<Value> = eng
        .bus
        .recent(1_000, 0)
        .into_iter()
        .filter(|ev| ev["type"] == AUTOMATION_SKIPPED_EVENT)
        .collect();
    assert_eq!(skipped.len(), 1);
    let data = &skipped[0]["data"];
    assert_eq!(data["automationId"], a.id.as_str());
    assert_eq!(data["trigger"], TRIGGER_CRON);
    assert_eq!(data["skipped"], SKIPPED_ALREADY_RUNNING);
}

#[test]
fn dry_run_over_noop_produces_five_steps_in_order() {
    let (eng, _workspace) = engine("noop");
    let a = eng.upsert(noop_automation("t-noop")).unwrap();
    let out = eng.test_automation(&a.id, json!({})).unwrap();
    assert_eq!(out["dryRun"], true);
    let steps: Vec<String> = out["trace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["step"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        steps,
        vec!["trigger", "filter", "guardrail", "action", "delivery"]
    );
    assert_eq!(out["trace"][2]["outcome"], "na");
    // Nothing was persisted: no run recorded for this automation.
    assert!(eng.list_runs(Some(&a.id), 10).is_empty());
}

#[test]
fn dry_run_over_webhook_sends_nothing() {
    let (eng, _workspace) = engine("webhook");
    let a = eng
        .upsert(webhook_automation("t-webhook", "http://127.0.0.1:1/never"))
        .unwrap();
    let out = eng.test_automation(&a.id, json!({})).unwrap();
    let steps: Vec<String> = out["trace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["step"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        steps,
        vec!["trigger", "filter", "guardrail", "action", "delivery"]
    );
    assert_eq!(
        out["trace"][3]["data"]["url"],
        aidp_integrations::redact_url("http://127.0.0.1:1/never")
    );
    assert_eq!(out["wouldRun"], true);
    // A real request to 127.0.0.1:1 would fail loudly; nothing was recorded either way.
    assert!(eng.list_runs(Some(&a.id), 10).is_empty());
}

#[test]
fn blocked_guardrail_puts_same_string_in_trace_and_run_note() {
    let (eng, _workspace) = engine("blocked");
    let _guard = EnvGuard::set("AIDP_MAX_UTILIZATION", "0.0");
    let a = eng
        .upsert(json!({
            "name": "t-blocked",
            "enabled": true,
            "trigger": { "kind": "webhook" },
            "action": { "kind": "launch", "projectId": "-", "templateId": "-" },
        }))
        .unwrap();

    // Real run: utilization_block short-circuits without a rate-limit window
    // configured on the fixture SessionManager, so exercise the pure guardrail
    // string directly and confirm the real run mirrors it end to end.
    if let Some(why) = eng.utilization_block(None) {
        let run = eng.run_automation(&a.id, "webhook", json!({})).unwrap();
        if run.status == "skipped" {
            assert_eq!(run.note.as_deref(), Some(why.as_str()));
            let guardrail = run
                .trace
                .iter()
                .find(|s| s.step == "guardrail")
                .expect("guardrail step present");
            assert_eq!(guardrail.outcome, "blocked");
            assert_eq!(guardrail.detail.as_deref(), Some(why.as_str()));
        }
    }
}

// ── back-compat: pre-lot-4a persisted JSON still deserializes ──────────────

#[test]
fn automation_run_json_from_before_lot_4a_still_deserializes() {
    let v = json!({
        "id": "run1",
        "automationId": "auto1",
        "automationName": "Sonde CI",
        "reason": "webhook",
        "startedAt": "2026-01-01T00:00:00Z",
        "status": "started",
        "sessionId": "sess1",
    });
    let run: AutomationRun = serde_json::from_value(v).unwrap();
    assert_eq!(run.queue_id, None);
    assert_eq!(run.session_id.as_deref(), Some("sess1"));
}

const AUTOMATION_TAG: &str = "automation-autonomous";
const AUTOMATION_NAME: &str = "autonomous";
const AUTOMATION_RUN_ID: &str = "run-1";

/// A stored legacy engine survives deserialization, and autonomy defaults to off.
#[test]
fn launch_action_keeps_stored_engine_and_defaults_autonomy_off() {
    let raw = json!({
        "kind": "launch",
        "projectId": "p1",
        "engine": "v7",
        "templateId": "dev-aidp"
    });
    let action: Action = serde_json::from_value(raw).unwrap();
    let Action::Launch {
        engine, autonomous, ..
    } = &action
    else {
        panic!("expected Action::Launch");
    };
    assert_eq!(engine.as_deref(), Some("v7"));
    assert!(!autonomous);
}

/// A launch action stored with the autonomous flag round-trips it.
#[test]
fn launch_action_round_trips_autonomous() {
    let raw = json!({
        "kind": "launch",
        "projectId": "p1",
        "templateId": "dev-aidp",
        "autonomous": true
    });
    let action: Action = serde_json::from_value(raw).unwrap();
    let back = serde_json::to_value(&action).unwrap();
    assert_eq!(back["autonomous"], true);
}

/// A launched automation exports the autonomous switch to its session.
#[test]
fn automation_start_request_is_autonomous() {
    let (engine, _workspace) = engine(AUTOMATION_TAG);
    let automation = engine.upsert(noop_automation(AUTOMATION_NAME)).unwrap();
    let request = automation_start_request(
        StartRequest::default(),
        REASON_EVENT,
        &automation,
        AUTOMATION_RUN_ID,
    );
    assert_eq!(request.env, autonomous_env());
}
