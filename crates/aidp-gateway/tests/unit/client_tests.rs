use super::*;

#[test]
fn text_presets_empty() {
    assert_eq!(text_presets(&json!([])), "no presets");
}

#[test]
fn text_presets_renders_one_row_per_preset() {
    let v = json!([
        { "id": "p1", "name": "Nightly dev", "templateId": "dev", "target": "PIXEL-1", "projectId": "proj-a" },
        { "id": "p2", "name": "Study run", "templateId": "study", "target": "PIXEL-2", "projectId": "proj-b" },
    ]);
    let text = text_presets(&v);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("Nightly dev"));
    assert!(lines[0].contains("dev"));
    assert!(lines[0].contains("p1"));
    assert!(lines[1].contains("Study run"));
    assert!(lines[1].contains("study"));
    assert!(lines[1].contains("p2"));
}

#[test]
fn text_presets_absorbs_missing_optional_fields() {
    let v = json!([{ "id": "p3", "name": "Bare", "templateId": "dev" }]);
    let text = text_presets(&v);
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("Bare"));
    assert!(text.contains("p3"));
}

#[test]
fn text_queue_empty() {
    assert_eq!(text_queue(&json!([])), "no queued launches");
}

#[test]
fn text_queue_renders_one_row_per_entry() {
    let v = json!([
        { "id": "q1", "projectId": "proj-a", "projectName": "Cheat API", "templateId": "dev", "target": "PIXEL-1", "priority": 0 },
        { "id": "q2", "projectId": "proj-b", "projectName": "Gateway", "templateId": "study", "target": "PIXEL-2", "priority": 1 },
    ]);
    let text = text_queue(&v);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("q1"));
    assert!(lines[0].contains("Cheat API"));
    assert!(lines[0].contains("dev"));
    assert!(lines[1].contains("q2"));
    assert!(lines[1].contains("Gateway"));
    assert!(lines[1].contains("study"));
}

#[test]
fn text_queue_absorbs_missing_optional_fields() {
    let v = json!([{ "id": "q3", "projectId": "proj-c", "projectName": "Bare", "priority": 0 }]);
    let text = text_queue(&v);
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("q3"));
    assert!(text.contains("Bare"));
}

#[test]
fn text_dispatcher_status_shape() {
    let v = json!({
        "mode": "normal",
        "paused": false,
        "lastTickAt": "2026-09-14T09:00:00Z",
        "lastReason": "interval",
        "slots": { "used": 1, "max": 3 },
        "running": [
            { "key": "PIXEL-1", "sessionId": "s-1", "project": "Cheat API", "startedAt": "…" }
        ],
        "lastPlan": {
            "launched": ["PIXEL-1"],
            "skipped": [{ "key": "PIXEL-2", "reason": "project-busy" }]
        }
    });
    let text = text_dispatcher(&v);
    assert!(text.contains("mode normal"));
    assert!(text.contains("slots 1/3"));
    assert!(text.contains("running"));
    assert!(text.contains("PIXEL-1"));
    assert!(text.contains("s-1"));
    assert!(text.contains("skip"));
    assert!(text.contains("PIXEL-2"));
    assert!(text.contains("project-busy"));
}

#[test]
fn text_dispatcher_plan_only_shape() {
    // plan_only()'s skipped entries are `[key, reason]` tuples, not
    // `{key, reason}` objects — text_dispatcher must still render them.
    let v = json!({
        "mode": "normal",
        "slotsUsed": 0,
        "slotsMax": 3,
        "launches": [{ "key": "PIXEL-3" }],
        "skipped": [["PIXEL-4", "project-busy"]],
    });
    let text = text_dispatcher(&v);
    assert!(text.contains("slots 0/3"));
    assert!(text.contains("would-launch"));
    assert!(text.contains("PIXEL-3"));
    assert!(text.contains("skip"));
    assert!(text.contains("PIXEL-4"));
    assert!(text.contains("project-busy"));
}

#[test]
fn auth_header_value_empty_is_none() {
    assert_eq!(auth_header_value(""), None);
}

#[test]
fn auth_header_value_whitespace_is_none() {
    assert_eq!(auth_header_value("  "), None);
}

#[test]
fn auth_header_value_nonempty_is_bearer() {
    assert_eq!(auth_header_value("abc"), Some("Bearer abc".to_string()));
}

#[test]
fn base_defaults_to_loopback() {
    assert_eq!(base(&None), "http://127.0.0.1:3001");
}

#[test]
fn start_body_includes_context_from() {
    let with_context = start_body(StartBodyArgs {
        pid: "proj-1".to_string(),
        template: Some("dev".to_string()),
        target: None,
        inputs_map: serde_json::Map::new(),
        prompt: None,
        permission_mode: None,
        model: None,
        effort: None,
        max_budget_usd: None,
        context_from: Some("sess-abc".to_string()),
        keep_open: false,
        workspace: None,
    });
    assert_eq!(with_context["contextFrom"], json!("sess-abc"));

    // client.rs keeps every optional key present in the body (the daemon
    // ignores nulls — see maxBudgetUsd/permissionMode/model/effort above)
    // rather than dropping the key when the flag is absent; contextFrom
    // follows that same convention.
    let without_context = start_body(StartBodyArgs {
        pid: "proj-1".to_string(),
        template: Some("dev".to_string()),
        target: None,
        inputs_map: serde_json::Map::new(),
        prompt: None,
        permission_mode: None,
        model: None,
        effort: None,
        max_budget_usd: None,
        context_from: None,
        keep_open: false,
        workspace: None,
    });
    assert_eq!(without_context["contextFrom"], Value::Null);
    assert_eq!(without_context["workspace"], Value::Null);
}

#[test]
fn start_body_carries_workspace() {
    let body = start_body(StartBodyArgs {
        pid: "proj-1".to_string(),
        template: None,
        target: None,
        inputs_map: serde_json::Map::new(),
        prompt: Some("go".to_string()),
        permission_mode: None,
        model: None,
        effort: None,
        max_budget_usd: None,
        context_from: None,
        keep_open: false,
        workspace: Some(crate::models::WorkspaceRequest {
            kind: crate::models::enums::WorkspaceKind::Scratch,
            ticket: None,
            name: Some("try".to_string()),
            path: None,
        }),
    });
    assert_eq!(body["workspace"]["kind"], json!("scratch"));
    assert_eq!(body["workspace"]["name"], json!("try"));
}

#[test]
fn worktrees_path_without_project_has_no_query() {
    assert_eq!(worktrees_path(None), "/api/worktrees");
}

#[test]
fn worktrees_path_with_project_adds_query() {
    assert_eq!(
        worktrees_path(Some("proj-a")),
        "/api/worktrees?project=proj-a"
    );
}

#[test]
fn prunable_entries_keeps_only_flagged_rows() {
    let listing = json!([
        { "key": "A-1", "prunable": true },
        { "key": "A-2", "prunable": false },
        { "key": "A-3" },
    ]);
    assert_eq!(
        prunable_entries(&listing),
        json!([{ "key": "A-1", "prunable": true }])
    );
}

#[test]
fn prunable_entries_of_a_non_array_is_empty() {
    assert_eq!(prunable_entries(&json!({})), json!([]));
}
