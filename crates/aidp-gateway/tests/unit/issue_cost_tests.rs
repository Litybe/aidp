use super::*;
use aidp_testkit::TempWorkspace;
use serde_json::json;

const KEY: &str = "GH-7";

fn stage(
    name: &str,
    agent: Option<&str>,
    model: &str,
    counters: [u64; 4],
    cost: Option<f64>,
) -> serde_json::Value {
    json!({
        "name": name,
        "status": "success",
        "stage_dir": "/tmp/stage",
        "usage": {
            "input": counters[0],
            "output": counters[1],
            "cache_creation": counters[2],
            "cache_read": counters[3],
            "model": model,
            "agent_type": agent,
            "bucket": "medium",
            "cost_usd": cost,
            "source": "jsonl",
        },
    })
}

fn state(run_id: &str, stages: Vec<serde_json::Value>) -> RunState {
    serde_json::from_value(json!({
        "run_id": run_id,
        "pipeline_path": "/tmp/p.yaml",
        "pipeline_id": "dev-unity",
        "workspace_root": "/tmp",
        "run_dir": "/tmp/run",
        "started_at": "2026-11-06T10:00:00Z",
        "stages": stages,
    }))
    .unwrap()
}

fn session(id: &str, ticket: &str, input: u64, cost: Option<f64>) -> SessionRecord {
    serde_json::from_value(json!({
        "id": id,
        "projectId": "p",
        "projectName": "P",
        "cwd": "/tmp",
        "engine": "v7",
        "prompt": "x",
        "permissionMode": "open",
        "status": "done",
        "startedAt": "2026-11-06T10:00:00Z",
        "origin": { "kind": "flow", "ref": ticket },
        "usage": { "input": input, "output": 1, "cacheRead": 2, "cacheCreate": 3 },
        "costUsd": cost,
    }))
    .unwrap()
}

#[test]
fn a_recorded_stage_cost_is_kept_and_the_counters_are_summed_per_run() {
    let table = PriceTable::from_env_or_embedded();
    let run = run_cost(
        &state(
            "r1",
            vec![
                stage(
                    "plan",
                    Some("planner"),
                    "claude-sonnet-5",
                    [10, 20, 30, 40],
                    Some(1.5),
                ),
                stage(
                    "code",
                    Some("executor"),
                    "claude-sonnet-5",
                    [1, 2, 3, 4],
                    Some(0.5),
                ),
            ],
        ),
        &table,
    );

    assert_eq!(run.run_id, "r1");
    assert_eq!(run.stages.len(), 2);
    assert_eq!(run.amount.cost_usd, 2.0);
    assert_eq!(run.amount.tokens.input, 11);
    assert_eq!(run.amount.tokens.output, 22);
    assert_eq!(run.amount.tokens.cache_create, 33);
    assert_eq!(run.amount.tokens.cache_read, 44);
    assert_eq!(run.amount.total_tokens, 110);
}

#[test]
fn a_stage_without_a_recorded_cost_is_priced_from_its_model() {
    let table = PriceTable::from_env_or_embedded();
    let run = run_cost(
        &state(
            "r1",
            vec![stage(
                "plan",
                None,
                "claude-sonnet-5",
                [1_000_000, 0, 0, 0],
                None,
            )],
        ),
        &table,
    );

    let expected = table.price(tier("claude-sonnet-5").as_str()).input;
    assert!(expected > 0.0, "the embedded grid prices sonnet");
    assert!((run.amount.cost_usd - expected).abs() < 1e-9);
}

#[test]
fn a_stage_of_an_unknown_model_without_cost_is_free_not_an_error() {
    let table = PriceTable::from_env_or_embedded();
    let mut raw = stage("plan", None, "mystery", [5, 5, 5, 5], None);
    raw["usage"]["model"] = serde_json::Value::Null;
    let run = run_cost(&state("r1", vec![raw]), &table);

    assert_eq!(run.amount.cost_usd, 0.0);
    assert_eq!(run.amount.total_tokens, 20);
}

#[test]
fn stages_without_usage_are_left_out() {
    let table = PriceTable::from_env_or_embedded();
    let bare = json!({ "name": "wait", "status": "pending", "stage_dir": "/tmp/s" });
    let run = run_cost(&state("r1", vec![bare]), &table);

    assert!(run.stages.is_empty());
    assert_eq!(run.amount, CostAmount::default());
}

#[test]
fn runs_are_folded_per_stage_and_per_agent_biggest_cost_first() {
    let table = PriceTable::from_env_or_embedded();
    let first = run_cost(
        &state(
            "r1",
            vec![
                stage("code", Some("executor"), "m", [100, 0, 0, 0], Some(3.0)),
                stage("review", Some("reviewer"), "m", [10, 0, 0, 0], Some(1.0)),
            ],
        ),
        &table,
    );
    let second = run_cost(
        &state(
            "r2",
            vec![
                stage("code", Some("executor"), "m", [50, 0, 0, 0], Some(2.0)),
                stage("lint", None, "m", [1, 0, 0, 0], Some(0.25)),
            ],
        ),
        &table,
    );

    let cost = aggregate(KEY, vec![first, second], &[]);

    assert_eq!(cost.totals.cost_usd, 6.25);
    assert_eq!(cost.totals.tokens.input, 161);
    let stages: Vec<(&str, f64, u64)> = cost
        .by_stage
        .iter()
        .map(|g| (g.key.as_str(), g.amount.cost_usd, g.count))
        .collect();
    assert_eq!(
        stages,
        [("code", 5.0, 2), ("review", 1.0, 1), ("lint", 0.25, 1)]
    );
    let agents: Vec<&str> = cost.by_agent.iter().map(|g| g.key.as_str()).collect();
    assert_eq!(agents, ["executor", "reviewer", "unknown"]);
}

#[test]
fn only_the_sessions_of_the_issue_are_counted() {
    let records = vec![
        session("s1", KEY, 10, Some(0.5)),
        session("s2", KEY, 20, None),
        session("s3", "GH-8", 999, Some(9.0)),
    ];

    let cost = aggregate(KEY, Vec::new(), &records);

    assert_eq!(cost.session_count, 2);
    assert_eq!(cost.sessions.tokens.input, 30);
    assert_eq!(cost.sessions.tokens.cache_read, 4);
    assert_eq!(cost.sessions.cost_usd, 0.5);
    assert_eq!(cost.totals, CostAmount::default());
    assert_eq!(cost.pixellab, None);
}

#[test]
fn pixellab_credits_are_summed_over_the_runs_that_recorded_them() {
    let table = PriceTable::from_env_or_embedded();
    let mut art = run_cost(&state("r1", vec![]), &table);
    art.pixellab = Some(RunPixellabUsage {
        run_id: "r1".into(),
        generations: 4,
        spent: Some(12.5),
        stages: 1,
        ..RunPixellabUsage::default()
    });
    let mut other = run_cost(&state("r2", vec![]), &table);
    other.pixellab = Some(RunPixellabUsage {
        run_id: "r2".into(),
        generations: 2,
        spent: None,
        stages: 1,
        ..RunPixellabUsage::default()
    });

    let cost = aggregate(KEY, vec![art, other], &[]);

    assert_eq!(
        cost.pixellab,
        Some(PixellabTotals {
            generations: 6,
            credits: Some(12.5)
        })
    );
}

#[test]
fn the_report_reads_the_run_states_of_the_vault() {
    let vault = TempWorkspace::new("issue-cost");
    let run_dir = vault
        .path()
        .join(PIPELINES_DIR)
        .join("repo")
        .join(KEY)
        .join("run-1");
    std::fs::create_dir_all(&run_dir).unwrap();
    let raw = serde_json::to_string(&json!({
        "run_id": "run-1",
        "pipeline_path": "/tmp/p.yaml",
        "pipeline_id": "dev-unity",
        "workspace_root": "/tmp",
        "run_dir": "/tmp/run",
        "started_at": "2026-11-06T10:00:00Z",
        "stages": [stage("code", Some("executor"), "m", [7, 0, 0, 0], Some(0.75))],
    }))
    .unwrap();
    std::fs::write(run_dir.join("run-state.json"), raw).unwrap();
    std::fs::write(
        run_dir.join("pixellab-usage.json"),
        r#"{"generations":3,"jobs":["a"],"balanceBefore":10,"balanceAfter":7}"#,
    )
    .unwrap();

    let cost = issue_cost_report(vault.path(), KEY, &[]);

    assert_eq!(cost.runs.len(), 1);
    assert_eq!(cost.totals.cost_usd, 0.75);
    assert_eq!(cost.totals.tokens.input, 7);
    assert_eq!(
        cost.pixellab,
        Some(PixellabTotals {
            generations: 3,
            credits: Some(3.0)
        })
    );
}

#[test]
fn an_issue_without_runs_or_vault_reports_zeroes() {
    let cost = issue_cost_report(Path::new(""), KEY, &[]);

    assert_eq!(cost.key, KEY);
    assert!(cost.runs.is_empty());
    assert_eq!(cost.totals, CostAmount::default());
}
