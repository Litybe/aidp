use super::*;

fn mk_session(
    id: &str,
    status: &str,
    started_at: &str,
    ended_at: Option<&str>,
    pending: bool,
) -> SessionRecord {
    SessionRecord {
        mcp_config_id: None,
        provider: None,
        provider_session_id: None,
        id: id.to_string(),
        project_id: "proj1".to_string(),
        project_name: "Project One".to_string(),
        cwd: "/repo".to_string(),
        engine: "v7".to_string(),
        template_id: None,
        target: None,
        inputs: None,
        prompt: "do stuff".to_string(),
        permission_mode: "default".to_string(),
        effective_permission_mode: None,
        model: None,
        effort: None,
        max_budget_usd: None,
        status: SessionStatus::parse(status).expect("known session status"),
        started_at: started_at.to_string(),
        ended_at: ended_at.map(|s| s.to_string()),
        start_sha: None,
        exit_code: None,
        claude_session_id: None,
        resumed_from: None,
        cost_usd: None,
        turns: 0,
        last_text: None,
        pending_question: if pending {
            Some(crate::sessions::PendingQuestion {
                permission_request_id: None,
                tool_use_id: "tu1".to_string(),
                questions: serde_json::json!([]),
            })
        } else {
            None
        },
        origin: crate::sessions::Origin::new("test"),
        automation_id: None,
        automation_run_id: None,
        workflow_run_id: None,
        event_count: 0,
        auto_close: false,
        usage: None,
        title: None,
        pinned: false,
        archived: false,
        unread: false,
        context_from: None,
        chain_depth: None,
        workspace: None,
        warnings: Vec::new(),
        headless: false,
    }
}

fn mk_run(
    id: &str,
    automation_id: &str,
    automation_name: &str,
    status: &str,
    started_at: &str,
    note: Option<&str>,
) -> AutomationRun {
    AutomationRun {
        id: id.to_string(),
        automation_id: automation_id.to_string(),
        automation_name: automation_name.to_string(),
        reason: "scheduled".to_string(),
        started_at: started_at.to_string(),
        ended_at: None,
        status: status.to_string(),
        session_id: None,
        queue_id: None,
        error: None,
        note: note.map(|s| s.to_string()),
        trace: Vec::new(),
    }
}

#[test]
fn urgency_ordering_across_all_four_kinds() {
    let sessions = vec![
        mk_session("gate1", "running", "2026-09-10T08:00:00Z", None, true),
        mk_session(
            "fail1",
            "failed",
            "2026-09-09T20:00:00Z",
            Some("2026-09-09T20:30:00Z"),
            false,
        ),
        mk_session("run1", "running", "2026-09-10T07:00:00Z", None, false),
    ];
    let runs = vec![
        mk_run(
            "r1",
            "auto1",
            "Nightly",
            "skipped",
            "2026-09-10T01:00:00Z",
            None,
        ),
        mk_run(
            "r2",
            "auto1",
            "Nightly",
            "skipped",
            "2026-09-10T02:00:00Z",
            None,
        ),
    ];
    let now = "2026-09-10T13:00:00Z";
    let out = build_inbox(&sessions, &runs, &[], now);
    let kinds: Vec<&str> = out.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec!["gate", "failure", "skippedAutomation", "runningSession"]
    );
}

#[test]
fn recency_tie_break_within_urgency_band() {
    let sessions = vec![
        mk_session("gate_old", "running", "2026-09-10T05:00:00Z", None, true),
        mk_session("gate_new", "running", "2026-09-10T09:00:00Z", None, true),
    ];
    let out = build_inbox(&sessions, &[], &[], "2026-09-10T13:00:00Z");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].session_id.as_deref(), Some("gate_new"));
    assert_eq!(out[1].session_id.as_deref(), Some("gate_old"));
}

#[test]
fn gate_kind_action_and_identifier() {
    let sessions = vec![mk_session(
        "s1",
        "running",
        "2026-09-10T08:00:00Z",
        None,
        true,
    )];
    let out = build_inbox(&sessions, &[], &[], "2026-09-10T13:00:00Z");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, "gate");
    assert_eq!(out[0].action, "reply");
    assert_eq!(out[0].session_id.as_deref(), Some("s1"));
}

#[test]
fn failure_kind_action_and_identifier() {
    let sessions = vec![mk_session(
        "s1",
        "failed",
        "2026-09-09T12:00:00Z",
        Some("2026-09-09T12:30:00Z"),
        false,
    )];
    let out = build_inbox(&sessions, &[], &[], "2026-09-10T00:00:00Z");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, "failure");
    assert_eq!(out[0].action, InboxAction::Open.as_str());
    assert_eq!(out[0].session_id.as_deref(), Some("s1"));
}

#[test]
fn skipped_automation_kind_action_and_identifier() {
    let runs = vec![
        mk_run(
            "r1",
            "auto1",
            "Nightly",
            "skipped",
            "2026-09-10T01:00:00Z",
            None,
        ),
        mk_run(
            "r2",
            "auto1",
            "Nightly",
            "skipped",
            "2026-09-10T02:00:00Z",
            Some("no budget"),
        ),
    ];
    let out = build_inbox(&[], &runs, &[], "2026-09-10T13:00:00Z");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, "skippedAutomation");
    assert_eq!(out[0].action, "dismiss");
    assert_eq!(out[0].automation_id.as_deref(), Some("auto1"));
    assert_eq!(out[0].detail.as_deref(), Some("no budget"));
}

#[test]
fn running_session_kind_action_and_identifier() {
    let sessions = vec![mk_session(
        "s1",
        "running",
        "2026-09-10T08:00:00Z",
        None,
        false,
    )];
    let out = build_inbox(&sessions, &[], &[], "2026-09-10T13:00:00Z");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, "runningSession");
    assert_eq!(out[0].action, InboxAction::Open.as_str());
    assert_eq!(out[0].session_id.as_deref(), Some("s1"));
}

#[test]
fn failure_24h_boundary() {
    let now = "2026-09-10T12:00:00Z";
    let inside = mk_session(
        "inside",
        "failed",
        "2026-09-09T00:00:00Z",
        Some("2026-09-09T13:00:00Z"), // 23h before now
        false,
    );
    let outside = mk_session(
        "outside",
        "failed",
        "2026-09-09T00:00:00Z",
        Some("2026-09-09T11:00:00Z"), // 25h before now
        false,
    );
    let out = build_inbox(&[inside, outside], &[], &[], now);
    let ids: Vec<&str> = out.iter().filter_map(|e| e.session_id.as_deref()).collect();
    assert!(ids.contains(&"inside"));
    assert!(!ids.contains(&"outside"));
}

#[test]
fn gate_excludes_running_session_double_count() {
    let sessions = vec![mk_session(
        "s1",
        "running",
        "2026-09-10T08:00:00Z",
        None,
        true,
    )];
    let out = build_inbox(&sessions, &[], &[], "2026-09-10T13:00:00Z");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, "gate");
}

#[test]
fn recurrence_rule_variants() {
    // one skipped run alone yields nothing
    let runs_one = vec![mk_run(
        "r1",
        "auto1",
        "Nightly",
        "skipped",
        "2026-09-10T01:00:00Z",
        None,
    )];
    let out = build_inbox(&[], &runs_one, &[], "2026-09-10T13:00:00Z");
    assert!(out.is_empty());

    // two skipped runs in the window yield exactly one entry
    let runs_two = vec![
        mk_run(
            "r1",
            "auto1",
            "Nightly",
            "skipped",
            "2026-09-10T01:00:00Z",
            None,
        ),
        mk_run(
            "r2",
            "auto1",
            "Nightly",
            "skipped",
            "2026-09-10T02:00:00Z",
            None,
        ),
    ];
    let out = build_inbox(&[], &runs_two, &[], "2026-09-10T13:00:00Z");
    assert_eq!(out.len(), 1);

    // latest run done, even with two older skipped runs, yields nothing
    let runs_done = vec![
        mk_run(
            "r1",
            "auto1",
            "Nightly",
            "skipped",
            "2026-09-10T01:00:00Z",
            None,
        ),
        mk_run(
            "r2",
            "auto1",
            "Nightly",
            "skipped",
            "2026-09-10T02:00:00Z",
            None,
        ),
        mk_run(
            "r3",
            "auto1",
            "Nightly",
            "done",
            "2026-09-10T03:00:00Z",
            None,
        ),
    ];
    let out = build_inbox(&[], &runs_done, &[], "2026-09-10T13:00:00Z");
    assert!(out.is_empty());
}

#[test]
fn unparseable_now_keeps_failures_without_panic() {
    let sessions = vec![mk_session(
        "s1",
        "failed",
        "2026-09-01T00:00:00Z",
        Some("2026-09-01T01:00:00Z"),
        false,
    )];
    let out = build_inbox(&sessions, &[], &[], "not-a-timestamp");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, "failure");
}

#[test]
fn serde_round_trip_camel_case_and_omitted_optionals() {
    let entry = InboxEntry {
        kind: "gate".to_string(),
        action: "reply".to_string(),
        title: "My Session".to_string(),
        detail: None,
        session_id: Some("s1".to_string()),
        fingerprint: None,
        automation_id: None,
        project_id: Some("p1".to_string()),
        project_name: Some("Project".to_string()),
        at: "2026-09-10T08:00:00Z".to_string(),
        urgency: 0,
    };
    let v = serde_json::to_value(&entry).unwrap();
    let obj = v.as_object().unwrap();
    assert!(obj.contains_key("sessionId"));
    assert!(obj.contains_key("projectId"));
    assert!(obj.contains_key("projectName"));
    assert!(!obj.contains_key("detail"));
    assert!(!obj.contains_key("fingerprint"));
    assert!(!obj.contains_key("automationId"));
    assert!(!obj.contains_key("session_id"));

    let round: InboxEntry = serde_json::from_value(v).unwrap();
    assert_eq!(round.kind, "gate");
    assert_eq!(round.session_id.as_deref(), Some("s1"));
}
fn mk_attention(fingerprint: &str, notified: bool, read: bool) -> PrAttentionRecord {
    PrAttentionRecord {
        fingerprint: fingerprint.to_string(),
        repo: "acme/svc".to_string(),
        number: 7,
        signal: crate::models::enums::attention_signal_kind::AttentionSignalKind::NewComment,
        head_sha: "sha1".to_string(),
        detail: Some("c1".to_string()),
        first_seen: "2026-09-10T12:00:00Z".to_string(),
        notified,
        read,
    }
}

#[test]
fn unread_notified_attention_becomes_an_entry() {
    let records = vec![mk_attention("fp1", true, false)];
    let out = build_inbox(&[], &[], &records, "2026-09-10T13:00:00Z");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, InboxKind::Attention.as_str());
    assert_eq!(out[0].action, InboxAction::Open.as_str());
    assert_eq!(out[0].fingerprint.as_deref(), Some("fp1"));
    assert_eq!(out[0].urgency, 1);
}

#[test]
fn read_or_silent_attention_is_left_out() {
    let records = vec![
        mk_attention("fp1", true, true),
        mk_attention("fp2", false, false),
    ];
    let out = build_inbox(&[], &[], &records, "2026-09-10T13:00:00Z");
    assert!(out.is_empty());
}
