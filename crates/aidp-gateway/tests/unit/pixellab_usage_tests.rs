use super::*;
use aidp_testkit::TempWorkspace;
use serde_json::json;

fn write_usage(run_dir: &Path, stage: &str, body: Value) {
    let dir = run_dir.join(stage);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(USAGE_FILE), body.to_string()).unwrap();
}

fn usage(before: f64, after: f64, generations: u32, at: &str) -> Value {
    json!({
        "balanceBefore": { "credits": before },
        "balanceAfter": { "credits": after },
        "generations": generations,
        "jobs": ["j1", "j2"],
        "recordedAt": at,
    })
}

#[test]
fn balance_amount_reads_numbers_keys_and_text() {
    assert_eq!(balance_amount(&json!(12.5)), Some(12.5));
    assert_eq!(balance_amount(&json!({ "credits": 7 })), Some(7.0));
    assert_eq!(
        balance_amount(&json!({ "Balance": { "usd": "3.25" } })),
        Some(3.25)
    );
    assert_eq!(
        balance_amount(&json!("Credits: $4.50 remaining")),
        Some(4.5)
    );
    assert_eq!(balance_amount(&json!({ "other": 1 })), None);
    assert_eq!(balance_amount(&json!(null)), None);
}

#[test]
fn a_run_sums_its_stages_and_spans_the_balance() {
    let vault = TempWorkspace::new("pixellab-run");
    let run_dir = vault.path().join("run-1");
    write_usage(
        &run_dir,
        "generate",
        usage(10.0, 8.0, 4, "2026-11-06T10:00:00Z"),
    );
    write_usage(
        &run_dir,
        "animate",
        usage(8.0, 5.5, 3, "2026-11-06T11:00:00Z"),
    );
    let run = aggregate_run("run-1", &run_dir);
    assert_eq!(run.generations, 7);
    assert_eq!(run.jobs, 4);
    assert_eq!(run.stages, 2);
    assert_eq!(run.balance_before, Some(10.0));
    assert_eq!(run.balance_after, Some(5.5));
    assert_eq!(run.spent, Some(4.5));
    assert_eq!(run.recorded_at.as_deref(), Some("2026-11-06T11:00:00Z"));
}

#[test]
fn a_run_without_usage_files_is_empty() {
    let vault = TempWorkspace::new("pixellab-empty");
    let run = aggregate_run("run-0", vault.path());
    assert_eq!(run.stages, 0);
    assert_eq!(run.generations, 0);
}

#[test]
fn a_ticket_sums_its_runs() {
    let vault = TempWorkspace::new("pixellab-ticket");
    let feature = vault.path().join("Pipelines").join("game").join("GH-3");
    write_usage(
        &feature.join("run-a"),
        "generate",
        usage(10.0, 9.0, 4, "2026-11-06T10:00:00Z"),
    );
    write_usage(
        &feature.join("run-b"),
        "generate",
        usage(9.0, 6.0, 5, "2026-11-06T12:00:00Z"),
    );
    std::fs::create_dir_all(feature.join("run-c")).unwrap();
    let other = vault.path().join("Pipelines").join("game").join("GH-4");
    write_usage(
        &other.join("run-x"),
        "generate",
        usage(6.0, 1.0, 9, "2026-11-06T13:00:00Z"),
    );
    let ticket = aggregate_ticket(vault.path(), "GH-3");
    assert_eq!(ticket.generations, 9);
    assert_eq!(ticket.runs.len(), 2);
    assert_eq!(ticket.runs[0].run_id, "run-b");
    assert_eq!(ticket.spent, 4.0);
}

#[test]
fn the_latest_balance_is_the_newest_after_balance_of_the_vault() {
    let vault = TempWorkspace::new("pixellab-balance");
    let game = vault.path().join("Pipelines").join("game");
    write_usage(
        &game.join("GH-3").join("run-a"),
        "generate",
        usage(10.0, 9.0, 1, "2026-11-06T10:00:00Z"),
    );
    write_usage(
        &game.join("GH-4").join("run-b"),
        "generate",
        usage(9.0, 6.5, 1, "2026-11-06T12:00:00Z"),
    );
    let balance = scan_latest_balance(vault.path()).expect("a balance");
    assert_eq!(balance.amount, Some(6.5));
    assert_eq!(balance.recorded_at, "2026-11-06T12:00:00Z");
    assert_eq!(balance.run_id, "run-b");
    assert_eq!(latest_balance(vault.path()).unwrap().amount, Some(6.5));
    forget_balance(vault.path());
}

#[test]
fn no_usage_files_mean_no_balance() {
    let vault = TempWorkspace::new("pixellab-none");
    assert_eq!(scan_latest_balance(vault.path()), None);
}

#[test]
fn a_usage_file_with_only_a_before_balance_still_counts() {
    let vault = TempWorkspace::new("pixellab-before");
    let run_dir = vault
        .path()
        .join("Pipelines")
        .join("game")
        .join("GH-3")
        .join("run-a");
    write_usage(
        &run_dir,
        "generate",
        json!({ "balanceBefore": { "credits": 4 }, "generations": 0, "recordedAt": "2026-11-06T10:00:00Z" }),
    );
    assert_eq!(scan_latest_balance(vault.path()).unwrap().amount, Some(4.0));
}

#[test]
fn the_balance_report_has_nulls_before_any_stage_recorded_one() {
    let vault = TempWorkspace::new("p5");
    forget_balance(vault.path());

    let report = balance_report(vault.path());

    assert!(report["balance"].is_null());
    assert_eq!(report["belowReserve"], false);
}

#[test]
fn the_balance_report_compares_the_balance_with_the_reserve() {
    let vault = TempWorkspace::new("p5");
    forget_balance(vault.path());
    let todo = vault.path().join("TODO");
    std::fs::create_dir_all(&todo).unwrap();
    std::fs::write(
        todo.join("config.yaml"),
        "budget:\n  pixellab:\n    min_reserve: 10\n",
    )
    .unwrap();
    write_usage(
        &vault.path().join("Pipelines/repo/GH-1/run-1"),
        "generate",
        usage(15.0, 6.0, 4, "2026-11-06T10:00:00Z"),
    );

    let report = balance_report(vault.path());

    assert_eq!(report["balance"], 6.0);
    assert_eq!(report["minReserve"], 10.0);
    assert_eq!(report["belowReserve"], true);
    assert_eq!(report["runId"], "run-1");
}

#[test]
fn a_text_balance_counts_the_subscription_generations_not_the_empty_credits() {
    let text = "credits: $0.00; generations_remaining: 1436.60; generations_used: 563.40; generations_total: 2000; subscription: active (Tier 1: Pixel Apprentice); generations_reset: 2026-12-05";
    assert_eq!(balance_amount(&json!(text)), Some(1436.6));
    assert_eq!(
        balance_amount(&json!("credits: $4.50; generations_remaining: 0.00")),
        Some(4.5)
    );
}

#[test]
fn a_stage_file_with_job_objects_and_fractional_generations_is_read() {
    let vault = TempWorkspace::new("pixellab-real");
    let run_dir = vault.path().join("run-1");
    let before = "credits: $0.00; generations_remaining: 1446.70; generations_total: 2000";
    let after = "credits: $0.00; generations_remaining: 1436.60; generations_total: 2000";
    write_usage(
        &run_dir,
        "generate_pixellab",
        json!({
            "balanceBefore": before,
            "balanceAfter": after,
            "generations": 10.1,
            "jobs": [
                { "id": "a", "tool": "edit_image_pro_flash", "status": "completed", "generations": 5 },
                { "id": "b", "tool": "reduce_colors", "status": "completed", "generations": 0.1 }
            ],
            "recordedAt": "2026-11-10T14:40:00Z",
        }),
    );
    let run = aggregate_run("run-1", &run_dir);
    assert_eq!(run.stages, 1);
    assert_eq!(run.generations, 10);
    assert_eq!(run.jobs, 2);
    assert_eq!(run.balance_before, Some(1446.7));
    assert_eq!(run.balance_after, Some(1436.6));
    assert!((run.spent.unwrap() - 10.1).abs() < 1e-6);
}
