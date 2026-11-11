use super::*;
use aidp_testkit::TempWorkspace;
use serde_json::json;

fn stage_session(id: &str, ticket: &str, run: &str, template: &str, tokens: u64) -> SessionRecord {
    serde_json::from_value(json!({
        "id": id,
        "projectId": "p",
        "projectName": "P",
        "cwd": "/tmp",
        "engine": "v7",
        "templateId": template,
        "target": ticket,
        "prompt": "x",
        "permissionMode": "open",
        "status": "done",
        "startedAt": "2026-11-10T10:00:00Z",
        "model": "claude-sonnet-5",
        "origin": { "kind": "run", "ref": run, "label": "stage" },
        "usage": { "input": tokens, "output": 0, "cacheRead": 0, "cacheCreate": 0 },
    }))
    .unwrap()
}

fn write_run_state(vault: &Path, ticket: &str, run: &str, pipeline: &str, route: Option<&str>) {
    let dir = vault.join("Pipelines").join("Repo").join(ticket).join(run);
    std::fs::create_dir_all(&dir).unwrap();
    let mut inputs = json!({ "feature": ticket });
    if let Some(route) = route {
        inputs["route"] = json!(route);
    }
    let state = json!({
        "run_id": run,
        "pipeline_path": "p.yaml",
        "pipeline_id": pipeline,
        "workspace_root": "w",
        "run_dir": dir,
        "started_at": "2026-11-10T10:00:00Z",
        "inputs": inputs,
        "plan": [],
        "stages": [],
    });
    std::fs::write(dir.join("run-state.json"), state.to_string()).unwrap();
}

#[test]
fn a_run_stage_session_belongs_to_its_target_ticket_not_to_its_run_id() {
    let record = stage_session("s1", "GH-410", "20261110-140813-4648", "art-produce", 10);
    assert_eq!(ticket_of(&record), Some("GH-410"));
    assert_eq!(run_of(&record), Some("20261110-140813-4648"));
}

#[test]
fn the_route_group_reads_the_run_state_then_the_pipeline_then_the_template() {
    let vault = TempWorkspace::new("cost-routes");
    write_run_state(
        vault.path(),
        "GH-410",
        "run-art",
        "art-produce",
        Some("art"),
    );
    write_run_state(vault.path(), "GH-418", "run-code", "unity-implement", None);
    let records = vec![
        stage_session("s1", "GH-410", "run-art", "art-produce", 100),
        stage_session("s2", "GH-418", "run-code", "unity-implement", 50),
        stage_session("s3", "GH-500", "run-gone", "map-room", 7),
        stage_session("s4", "GH-501", "run-none", "study", 3),
    ];
    let report = usage_report(&records, None, None, "route", vault.path());
    let tokens: Vec<(&str, u64)> = report
        .buckets
        .iter()
        .map(|bucket| (bucket.key.as_str(), bucket.total_tokens))
        .collect();
    assert_eq!(
        tokens,
        vec![("art", 100), ("code", 50), ("map", 7), ("unknown", 3)]
    );
}

#[test]
fn the_ticket_group_keys_on_the_issue_and_lists_its_runs() {
    let vault = TempWorkspace::new("cost-tickets");
    let records = vec![
        stage_session("s1", "GH-410", "run-a", "art-produce", 100),
        stage_session("s2", "GH-410", "run-a", "art-produce", 20),
        stage_session("s3", "GH-410", "run-b", "art-fix", 5),
    ];
    let report = usage_report(&records, None, None, "ticket", vault.path());
    assert_eq!(report.buckets.len(), 1);
    assert_eq!(report.buckets[0].key, "GH-410");
    assert_eq!(report.buckets[0].total_tokens, 125);
    assert_eq!(report.buckets[0].runs, vec!["run-a", "run-b"]);
}

#[test]
fn the_route_of_a_ticket_comes_from_its_latest_run_state() {
    let vault = TempWorkspace::new("cost-route-key");
    write_run_state(
        vault.path(),
        "GH-7",
        "20261109-000000-aaaa",
        "dev",
        Some("code"),
    );
    write_run_state(
        vault.path(),
        "GH-7",
        "20261110-000000-bbbb",
        "art-produce",
        Some("art"),
    );
    assert_eq!(
        route_of_key_in(
            &vault_index::snapshot(vault.path()),
            vault.path(),
            "GH-7",
            &[]
        )
        .as_deref(),
        Some("art")
    );
}

#[test]
fn the_batch_costs_equal_the_cost_of_each_ticket() {
    let vault = TempWorkspace::new("cost-batch");
    let records = vec![
        stage_session("s1", "GH-410", "run-a", "art-produce", 100),
        stage_session("s2", "GH-410", "run-a", "art-produce", 20),
        stage_session("s3", "GH-418", "run-b", "unity-implement", 7),
    ];
    let batch = ticket_costs_report(&records, vault.path(), None);
    assert_eq!(batch.len(), 2);
    for key in ["GH-410", "GH-418"] {
        let single = ticket_cost_report(key, &records, vault.path(), None).unwrap();
        assert_eq!(batch[key].totals.total_tokens, single.totals.total_tokens);
        assert_eq!(batch[key].cost_usd, single.cost_usd);
    }
    assert_eq!(batch["GH-410"].totals.total_tokens, 120);
}

#[test]
fn the_batch_costs_honour_the_keys_and_skip_unknown_tickets() {
    let vault = TempWorkspace::new("cost-batch-keys");
    let records = vec![
        stage_session("s1", "GH-410", "run-a", "art-produce", 100),
        stage_session("s2", "GH-418", "run-b", "unity-implement", 7),
    ];
    let keys = vec!["GH-418".to_string(), "GH-999".to_string()];
    let batch = ticket_costs_report(&records, vault.path(), Some(&keys));
    assert_eq!(batch.keys().collect::<Vec<_>>(), vec!["GH-418"]);
}

#[test]
fn a_ticket_without_a_priced_session_has_no_dollars() {
    let vault = TempWorkspace::new("cost-batch-unpriced");
    let mut record = stage_session("s1", "GH-410", "run-a", "art-produce", 100);
    record.model = None;
    record.cost_usd = None;
    let batch = ticket_costs_report(&[record], vault.path(), None);
    assert_eq!(batch["GH-410"].cost_usd, None);
    let body = serde_json::to_value(batch["GH-410"]).unwrap();
    assert_eq!(body["costUsd"], serde_json::Value::Null);
}
